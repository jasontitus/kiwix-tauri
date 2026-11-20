// SPDX-License-Identifier: GPL-3.0-or-later
//! Unit tests for `store`, kept beside it rather than inline.
#![forbid(unsafe_code)]
use super::*;

#[test]
fn history_resource_identity_matches_recently_read_grouping() {
    for (left, right) in [
        ("A/Article#one", "A/Article#two"),
        ("A/caf%C3%A9", "A/café"),
        ("A/%41rticle", "A/Article"),
        ("A/100%done", "A/100%25done"),
        ("A/a%3Fb%23c?exact=%41#fragment", "A/a%3fb%23c?exact=%41"),
        ("A/%FF?query#one", "A/%FF?query#two"),
    ] {
        assert_eq!(history_resource_identity(left), history_resource_identity(right),
            "{left} and {right} must group together");
    }
    for (left, right) in [
        ("A/Article", "A/article"),
        ("A/Article?q=%41", "A/Article?q=A"),
        ("A/Article?q=one", "A/Article?q=two"),
        ("A/a%3Fb", "A/a?b"),
        ("A/a%23b", "A/a#b"),
        ("A/%2541", "A/A"),
        ("A/%FF", "A/%ff"),
        ("A/%FF", "A/%25FF"),
        ("A/%C0%AF", "A//"),
        ("A/%ED%A0%80", "A/�"),
        ("A/%E2%82", "A/%E2%82%AC"),
    ] {
        assert_ne!(history_resource_identity(left), history_resource_identity(right),
            "{left} and {right} must remain distinct");
    }
    assert!(!history_resource_identity("A/%FF").0);
    assert!(history_resource_identity("A/%25FF").0);
}

struct Profile(PathBuf);
impl Profile {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("kiwix-store-test-{}", Uuid::new_v4())))
    }
}
impl Drop for Profile {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[cfg(desktop)]
#[test]
fn saved_theme_hint_reads_only_valid_shell_themes() {
    let profile = Profile::new();
    fs::create_dir_all(&profile.0).unwrap();
    let path = profile.0.join("state.json");
    fs::write(&path, br#"{"settings":{"theme":"dark"}}"#).unwrap();
    assert_eq!(saved_theme_hint(&profile.0).as_deref(), Some("dark"));
    fs::write(&path, br#"{"settings":{"theme":"chartreuse"}}"#).unwrap();
    assert_eq!(saved_theme_hint(&profile.0), None);
    fs::write(&path, b"not json").unwrap();
    assert_eq!(saved_theme_hint(&profile.0), None);
    assert_eq!(saved_theme_hint(&profile.0.join("missing")), None);
}
fn tab(id: &str, path: &str) -> Tab {
    Tab {
        restricted: false,
        id: id.into(),
        book_id: Some("missing-archive".into()),
        path: path.into(),
        title: path.into(),
        scroll: 12.0,
        back: Vec::new(),
        forward: Vec::new(),
    }
}

#[test]
fn scalar_and_private_sync_accessors_match_the_saved_state() {
    let profile = Profile::new();
    let store = Store::new(profile.0.clone()).unwrap();
    let mut peer = crate::private_sync::Peer {
        id: "a".repeat(64), identity: "a".repeat(64), name: "Phone".into(),
        endpoints: vec!["127.0.0.1:9".parse().unwrap()], enabled: true,
        automatic: true, last_sync: None, acknowledged_counter: 3,
    };
    store.update(|state| {
        state.private_sync.listen_port = 45_678;
        state.private_sync.peers.push(peer.clone());
        state.reading_sync.enabled = true;
        Ok(())
    }).unwrap();
    assert!(store.reading_sync_enabled().unwrap());
    assert_eq!(store.reading_sync_counter().unwrap(), 0);
    assert_eq!(store.private_sync_listen_port().unwrap(), 45_678);
    assert_eq!(store.private_sync_peer(&peer.id).unwrap(), Some(peer.clone()));
    assert_eq!(store.private_sync_peer("missing").unwrap(), None);
    assert_eq!(store.private_sync_automatic_peers().unwrap(), vec![peer.clone()]);
    peer.id = "b".repeat(64);
    peer.identity = peer.id.clone();
    peer.automatic = false;
    store.update(|state| { state.private_sync.peers.push(peer); Ok(()) }).unwrap();
    assert_eq!(store.private_sync_automatic_peers().unwrap().len(), 1);
    assert_eq!(store.private_sync_peer(&"b".repeat(64)).unwrap().unwrap().acknowledged_counter, 3);
}

#[test]
fn search_snippets_default_on_and_saved_off_survives_restart() {
    let legacy: Settings = serde_json::from_value(serde_json::json!({})).unwrap();
    assert!(legacy.show_search_snippets);
    let profile = Profile::new();
    let store = Store::new(profile.0.clone()).unwrap();
    store.update(|state| {
        state.settings.show_search_snippets = false;
        Ok(())
    }).unwrap();
    drop(store);
    let restarted = Store::new(profile.0.clone()).unwrap();
    assert!(!restarted.settings().unwrap().show_search_snippets);
}

#[test]
fn legacy_link_previews_opt_out_survives_unrelated_settings_patch_and_restart() {
    let profile = Profile::new();
    fs::create_dir_all(&profile.0).unwrap();
    let path = profile.0.join("state.json");
    fs::write(&path, serde_json::to_vec(&serde_json::json!({
        "version": STATE_VERSION,
        "settings": { "theme": "dark" }
    })).unwrap()).unwrap();

    let store = Store::new(profile.0.clone()).unwrap();
    assert!(store.settings().unwrap().link_previews);
    assert_eq!(store.settings().unwrap().theme, "dark");
    store.update(|state| {
        state.settings.link_previews = false;
        Ok(())
    }).unwrap();
    drop(store);

    let reopened = Store::new(profile.0.clone()).unwrap();
    assert!(!reopened.settings().unwrap().link_previews);
    // Match the settings command's serialize, merge, deserialize contract.
    let mut merged = serde_json::to_value(reopened.settings().unwrap()).unwrap();
    merged.as_object_mut().unwrap().insert("theme".into(), serde_json::json!("light"));
    let settings: Settings = serde_json::from_value(merged).unwrap();
    reopened.update(|state| {
        state.settings = settings;
        Ok(())
    }).unwrap();
    drop(reopened);

    let saved: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved["settings"]["linkPreviews"], serde_json::json!(false));
    let restarted = Store::new(profile.0.clone()).unwrap();
    let settings = restarted.settings().unwrap();
    assert!(!settings.link_previews);
    assert_eq!(settings.theme, "light");
}

#[test]
fn pinned_books_persist_in_pin_order_across_restart() {
    let profile = Profile::new();
    let store = Store::new(profile.0.clone()).unwrap();
    assert!(store.settings().unwrap().pinned_book_ids.is_empty());
    store.update(|state| {
        state.settings.pinned_book_ids = vec!["wikipedia".into(), "wiktionary".into()];
        Ok(())
    }).unwrap();
    drop(store);
    let restarted = Store::new(profile.0.clone()).unwrap();
    assert_eq!(
        restarted.settings().unwrap().pinned_book_ids,
        ["wikipedia", "wiktionary"],
    );
    let saved: serde_json::Value = serde_json::from_slice(
        &fs::read(profile.0.join("state.json")).unwrap(),
    ).unwrap();
    assert_eq!(
        saved["settings"]["pinnedBookIds"],
        serde_json::json!(["wikipedia", "wiktionary"]),
    );
}

#[test]
fn pinned_books_are_capped_at_three_and_unique() {
    let mut settings = Settings {
        pinned_book_ids: vec!["a".into(), "b".into(), "c".into()],
        ..Settings::default()
    };
    settings.validate().unwrap();
    settings.pinned_book_ids.push("d".into());
    assert_eq!(
        settings.validate().unwrap_err(),
        "Pin at most 3 library archives at a time",
    );
    settings.pinned_book_ids = vec!["a".into(), "a".into()];
    assert_eq!(settings.validate().unwrap_err(), "Pinned archives must be unique");
    settings.pinned_book_ids = vec![String::new()];
    assert!(settings.validate().is_err());
}

#[test]
fn pinned_archive_identity_follows_its_replacement() {
    let mut settings = Settings {
        pinned_book_ids: vec!["first".into(), "old".into()],
        ..Settings::default()
    };
    settings.remap_pinned_book("old", "new");
    assert_eq!(settings.pinned_book_ids, ["first", "new"]);
    settings.remap_pinned_book("missing", "other");
    assert_eq!(settings.pinned_book_ids, ["first", "new"]);
    settings.remap_pinned_book("first", "new");
    assert_eq!(settings.pinned_book_ids, ["new"]);
    settings.validate().unwrap();
}

#[test]
fn sequential_pin_edits_use_current_state_not_a_stale_snapshot() {
    let profile = Profile::new();
    let store = Store::new(profile.0.clone()).unwrap();
    let mut books = Vec::new();
    for (index, id) in ["alpha", "beta", "gamma", "delta"].into_iter().enumerate() {
        let mut book = fixture_book();
        book.metadata.id = id.into();
        book.installation_id = new_installation_id();
        book.path = format!("/archives/{index}-{id}.zim");
        books.push(book);
    }
    store.update(|state| { state.books = books; Ok(()) }).unwrap();
    // Both toggles start from the same empty UI snapshot. The command sends
    // intent, so the store lock re-reads the current list instead of letting
    // the second toggle overwrite the first with a stale whole array.
    let stale = store.settings().unwrap().pinned_book_ids;
    assert!(stale.is_empty());
    assert_eq!(
        store.set_book_pinned("alpha", true).unwrap(),
        (true, vec!["alpha".to_string()]),
    );
    assert_eq!(
        store.set_book_pinned("beta", true).unwrap(),
        (true, vec!["alpha".to_string(), "beta".to_string()]),
    );
    assert_eq!(
        store.set_book_pinned("gamma", true).unwrap(),
        (true, vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()]),
    );
    assert_eq!(
        store.set_book_pinned("delta", true).unwrap_err(),
        "Pin at most 3 library archives at a time",
    );
    assert_eq!(
        store.set_book_pinned("beta", false).unwrap(),
        (true, vec!["alpha".to_string(), "gamma".to_string()]),
    );
    assert_eq!(
        store.set_book_pinned("beta", false).unwrap(),
        (false, vec!["alpha".to_string(), "gamma".to_string()]),
    );
    assert_eq!(
        store.set_book_pinned("missing", true).unwrap_err(),
        "Archive is not in the library",
    );
    drop(store);
    let restarted = Store::new(profile.0.clone()).unwrap();
    assert_eq!(
        restarted.settings().unwrap().pinned_book_ids,
        ["alpha", "gamma"],
    );
}

#[test]
fn stale_persisted_pin_does_not_consume_a_slot() {
    let profile = Profile::new();
    let store = Store::new(profile.0.clone()).unwrap();
    let mut books = Vec::new();
    for (index, id) in ["alpha", "beta", "gamma"].into_iter().enumerate() {
        let mut book = fixture_book();
        book.metadata.id = id.into();
        book.installation_id = new_installation_id();
        book.path = format!("/archives/{index}-{id}.zim");
        books.push(book);
    }
    store.update(|state| {
        state.books = books;
        // "stale" was skipped when the profile loaded, so the UI has no row
        // from which to unpin it.
        state.settings.pinned_book_ids = vec!["alpha".into(), "beta".into(), "stale".into()];
        Ok(())
    }).unwrap();
    assert_eq!(
        store.set_book_pinned("gamma", true).unwrap(),
        (true, vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()]),
    );
    let saved = store.settings().unwrap();
    assert_eq!(saved.pinned_book_ids, ["alpha", "beta", "gamma"]);
    assert!(!saved.pinned_book_ids.iter().any(|id| id == "stale"));
}

#[test]
fn technical_errors_default_off_and_saved_on_survives_restart() {
    let legacy: Settings = serde_json::from_value(serde_json::json!({})).unwrap();
    assert!(!legacy.show_technical_errors);
    let profile = Profile::new();
    let store = Store::new(profile.0.clone()).unwrap();
    store.update(|state| {
        state.settings.show_technical_errors = true;
        Ok(())
    }).unwrap();
    drop(store);
    let restarted = Store::new(profile.0.clone()).unwrap();
    assert!(restarted.settings().unwrap().show_technical_errors);
}

#[test]
fn reader_width_defaults_to_original_round_trips_and_rejects_unknown_choices() {
    let legacy: Settings = serde_json::from_value(serde_json::json!({})).unwrap();
    assert_eq!(legacy.reader_width, "original");
    let profile = Profile::new();
    let store = Store::new(profile.0.clone()).unwrap();
    store.update(|state| {
        state.settings.reader_width = "comfortable".into();
        Ok(())
    }).unwrap();
    drop(store);
    let restarted = Store::new(profile.0.clone()).unwrap();
    assert_eq!(restarted.settings().unwrap().reader_width, "comfortable");
    let saved: serde_json::Value = serde_json::from_slice(
        &fs::read(profile.0.join("state.json")).unwrap(),
    ).unwrap();
    assert_eq!(saved["settings"]["readerWidth"], serde_json::json!("comfortable"));

    let mut invalid = Settings {
        reader_width: "wide".into(),
        ..Default::default()
    };
    assert!(invalid.validate().is_err());
    invalid.reader_width = "full".into();
    assert!(invalid.validate().is_ok());
}

#[test]
fn pdf_handoff_serializes_installation_and_allows_explicit_private_export() {
    let profile = Profile::new();
    let store = Store::new(profile.0.clone()).unwrap();
    let book = fixture_book();
    store.update(|state| { state.books.push(book.clone()); Ok(()) }).unwrap();
    store.set_runtime_mode(true, false).unwrap();
    let mut called = false;
    store.with_current_installation(&book.metadata.id, &book.installation_id, || {
        assert!(matches!(store.inner.try_lock(), Err(std::sync::TryLockError::WouldBlock)));
        called = true;
        Ok(())
    }).unwrap();
    assert!(called);
    store.set_runtime_mode(true, true).unwrap();
    assert!(store.with_current_installation(&book.metadata.id, &book.installation_id,
        || -> Result<(), String> { panic!("read-only export must not hand off") }).is_err());
}

#[test]
fn pdf_handoff_rejects_same_uuid_reinstallation_and_unavailable_source() {
    let profile = Profile::new();
    let store = Store::new(profile.0.clone()).unwrap();
    let book = fixture_book();
    let replacement = new_installation_id();
    store.update(|state| {
        let mut current = book.clone();
        current.installation_id = replacement.clone();
        state.books.push(current);
        Ok(())
    }).unwrap();
    assert!(store.with_current_installation(&book.metadata.id, &book.installation_id,
        || -> Result<(), String> { panic!("retired installation must not hand off") }).is_err());
    store.with_current_installation(&book.metadata.id, &replacement, || Ok(())).unwrap();
    store.update(|state| { state.books[0].available = false; Ok(()) }).unwrap();
    assert!(store.with_current_installation(&book.metadata.id, &replacement,
        || -> Result<(), String> { panic!("unavailable source must not hand off") }).is_err());
}

#[test]
fn legacy_global_speech_migrates_without_remaining_ipc_aliases() {
    let mut saved = serde_json::to_value(State::default()).unwrap();
    saved["settings"].as_object_mut().unwrap().remove("speechPreferences");
    saved["settings"]["ttsVoice"] = serde_json::json!("legacy-installed-voice");
    saved["settings"]["ttsRate"] = serde_json::json!(0.25);
    let state: State = serde_json::from_value(saved).unwrap();
    assert_eq!(state.settings.speech_preferences["*"].rate, 0.25);
    assert_eq!(state.settings.speech_preferences["*"].voice, "legacy-installed-voice");
    let serialized = serde_json::to_value(&state).unwrap();
    assert!(serialized["settings"].get("ttsVoice").is_none());
    assert!(serialized["settings"].get("ttsRate").is_none());
    assert!(serde_json::from_value::<Settings>(serde_json::json!({"ttsRate":1.0})).is_err());
}

#[test]
fn speech_intents_merge_languages_and_independent_voice_rate_edits() {
    let profile = Profile::new();
    let store = Store::new(profile.0.clone()).unwrap();
    for (language, patch) in [
        ("en", SpeechPreferencePatch { voice: Some("english".into()), rate: None }),
        ("fr", SpeechPreferencePatch { voice: Some("french".into()), rate: None }),
        ("en", SpeechPreferencePatch { voice: None, rate: Some(0.25) }),
    ] {
        store.update(|state| state.settings.update_speech_preference(language, patch)).unwrap();
    }
    let preferences = store.settings().unwrap().speech_preferences;
    assert_eq!(preferences["en"].voice, "english");
    assert_eq!(preferences["en"].rate, 0.25);
    assert_eq!(preferences["fr"].voice, "french");
    assert_eq!(preferences["fr"].rate, 1.0);
}

#[test]
fn language_speech_preferences_survive_restart_independently() {
    let profile = Profile::new();
    let store = Store::new(profile.0.clone()).unwrap();
    store.update(|state| {
        state.settings.speech_preferences.insert("en".into(), SpeechPreference { voice: "english".into(), rate: 0.25 });
        state.settings.speech_preferences.insert("fr".into(), SpeechPreference { voice: "french".into(), rate: 2.0 });
        Ok(())
    }).unwrap();
    drop(store);
    let store = Store::new(profile.0.clone()).unwrap();
    let settings = store.settings().unwrap();
    assert_eq!(settings.speech_preferences["en"].rate, 0.25);
    assert_eq!(settings.speech_preferences["fr"].rate, 2.0);
    assert_eq!(settings.speech_preferences["en"].voice, "english");
    assert_eq!(settings.speech_preferences["fr"].voice, "french");
    let mut invalid = settings;
    invalid.speech_preferences.get_mut("en").unwrap().rate = 0.24;
    assert!(invalid.validate().is_err());
}

fn fixture_book() -> Book {
    crate::imports::load_book(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/small.zim")
            .to_str()
            .unwrap(),
        false,
        false,
    )
    .unwrap()
}

#[test]
fn complete_acquisition_set_reserves_later_missing_alias_before_commit() {
    let profile = Profile::new();
    let store = Store::new(profile.0.clone()).unwrap();
    let first = profile.0.join("set").join("book.zimaa");
    let reserved = profile.0.join("set").join("BOOK.ZIMAB");
    let mut book = fixture_book();
    book.path = reserved.to_string_lossy().into_owned();
    book.available = false;
    let installation = book.installation_id.clone();
    store.update(|state| { state.books.push(book); Ok(()) }).unwrap();
    let mut called = false;
    let error = store.with_acquisition_destinations(
        &[first, profile.0.join("set").join("book.zimab")],
        || { called = true; Ok(()) },
    ).unwrap_err();
    assert!(error.contains("reserved by an installed library book"));
    assert!(!called);
    assert_eq!(store.read().unwrap().books[0].installation_id, installation);
}

#[test]
fn acquisition_preserves_missing_reservations_and_existing_sentinels() {
    let profile = Profile::new();
    let store = Store::new(profile.0.clone()).unwrap();
    let destination = profile.0.join("installed.zim");
    let moved_archive = profile.0.join("moved-outside-library.zim");
    let prepared = profile.0.join("replacement.part");
    fs::write(&destination, b"original archive A").unwrap();
    fs::rename(&destination, &moved_archive).unwrap();
    fs::write(&prepared, b"fresh archive B with the same UUID").unwrap();
    let mut book = fixture_book();
    book.path = destination.to_str().unwrap().into();
    book.available = false;
    let installation = book.installation_id.clone();
    store
        .update(|state| {
            state.books.push(book);
            state.reader_storage.insert(
                installation.clone(),
                serde_json::json!({"localStorage":{"sentinel":"archive A"}}),
            );
            Ok(())
        })
        .unwrap();
    for existing in [false, true] {
        if existing {
            fs::write(&destination, b"reserved destination sentinel").unwrap();
        }
        let mut called = false;
        let error = store
            .with_acquisition_destination(&destination, || {
                called = true;
                fs::rename(&prepared, &destination).map_err(|error| error.to_string())
            })
            .unwrap_err();
        assert!(error.contains("reserved by an installed library book"));
        assert!(!called);
        assert_eq!(
            fs::read(&prepared).unwrap(),
            b"fresh archive B with the same UUID"
        );
        if existing {
            assert_eq!(
                fs::read(&destination).unwrap(),
                b"reserved destination sentinel"
            );
        } else {
            assert!(!destination.exists());
        }
    }
    assert_eq!(fs::read(moved_archive).unwrap(), b"original archive A");
    let state = store.read().unwrap();
    assert_eq!(state.books[0].installation_id, installation);
    assert_eq!(
        state.reader_storage[&installation]["localStorage"]["sentinel"],
        "archive A"
    );
}

#[test]
fn acquisition_path_aliases_are_lexical_unicode_and_platform_independent() {
    let equivalent = [
        ("/library/Archive.zim", "/LIBRARY/archive.ZIM"),
        ("/library/caf\u{e9}.zim", "/library/cafe\u{301}.zim"),
        ("/library/stra\u{df}e.zim", "/library/STRASSE.ZIM"),
        // Characters the pre-ICU4X (Unicode 9.0) folding left unchanged.
        ("/library/\u{1C90}.zim", "/library/\u{10D0}.zim"),
        ("/library/\u{1DF95}.zim", "/library/ss.zim"),
        ("/library/\u{1DF95}.zim", "/library/SS.ZIM"),
        ("/library/\u{1DF95}.zim", "/library/\u{df}.zim"),
        ("/library/./old/../archive.zim", "/library/archive.zim"),
        ("/library//archive.zim/", "/library/archive.zim"),
        (r"C:\Library\Archive.zim", "c:/library/archive.zim"),
        (r"\\?\C:\Library\Archive.zim", "c:/library/archive.zim"),
        (
            r"\\?\c:/Library/old/../Archive.zim",
            r"C:\library\archive.zim",
        ),
        (
            r"\\?\UNC\Server\Share\Archive.zim",
            r"\\server/share/archive.zim",
        ),
        ("../library/archive.zim", "/library/archive.zim"),
    ];
    for (left, right) in equivalent {
        assert!(
            acquisition_paths_equal(left, right, "/working"),
            "{left:?} != {right:?}"
        );
        assert!(
            acquisition_paths_equal(right, left, "/working"),
            "{right:?} != {left:?}"
        );
    }
    for (left, right) in [
        ("/library/archive.zim", "/library/other.zim"),
        ("/library/archive.zim", "/other/archive.zim"),
        ("C:/library/archive.zim", "D:/library/archive.zim"),
        (r"\\server\share\archive.zim", r"\\other\share\archive.zim"),
        ("/library/archive.zim", "/library/\u{ff41}rchive.zim"),
    ] {
        assert!(!acquisition_paths_equal(left, right, "/working"));
    }
}

#[test]
fn acquisition_win32_trailing_dot_and_space_aliases_are_portable() {
    for (left, right, working_directory) in [
        (
            r"\\.\C:\Library\archive.zim",
            r"\\?\C:\Library\archive.zim",
            "/working",
        ),
        (
            r"\\.\UNC\server\share\archive.zim.",
            r"\\?\UNC\server\share\archive.zim",
            "/working",
        ),
        (
            r"\\?/C:\Library\archive.zim.",
            r"\\?\C:\Library\archive.zim",
            "/working",
        ),
        (
            r"\\?/UNC\server\share\archive.zim.",
            r"\\?\UNC\server\share\archive.zim",
            "/working",
        ),
        (
            r"C:\Library\archive.zim.",
            r"\\?\C:\Library\archive.zim",
            "/working",
        ),
        (
            r"C:\Library\archive.zim   ",
            r"\\?\C:\Library\archive.zim",
            "/working",
        ),
        (
            r"C:\Library. \archive.zim. . ",
            r"\\?\c:\Library. \archive.zim",
            "/working",
        ),
        (
            r"C:\Library\.\old. \..\archive.zim. ",
            "c:/library/archive.zim",
            "/working",
        ),
        (
            r"C:\Library\old\.. \archive.zim",
            r"\\?\c:\library\old\.. \archive.zim",
            "/working",
        ),
        (
            r"\\Server\Share\Library. \archive.zim. ",
            r"\\?\UNC\server\share\Library. \archive.zim",
            "/working",
        ),
        (
            r"\\Server\Share\archive.zim ",
            r"\\server/share/archive.zim",
            "/working",
        ),
        (
            r"Library. \archive.zim. ",
            r"\\?\C:\working\Library. \archive.zim",
            r"C:\working",
        ),
        (
            "Library. /archive.zim. ",
            r"\\?\C:\working\Library. \archive.zim",
            r"C:\working",
        ),
        (
            r"archive.zim. ",
            r"\\server\share\Library\archive.zim",
            r"\\server\share\Library",
        ),
        (
            r"C:\Library \archive.zim",
            r"\\?\C:\Library \archive.zim",
            "/working",
        ),
        (
            r"\\Server\Share\Library \archive.zim",
            r"\\?\UNC\Server\Share\Library \archive.zim",
            "/working",
        ),
        (
            r"\\?\C:\Library\archive.zim.",
            r"\\?\c:\library\ARCHIVE.ZIM.",
            "/working",
        ),
    ] {
        assert!(
            acquisition_paths_equal(left, right, working_directory),
            "{left:?} != {right:?}"
        );
        assert!(
            acquisition_paths_equal(right, left, working_directory),
            "{right:?} != {left:?}"
        );
    }
    for (left, right) in [
        (r"\\?\C:\Library\archive.zim.", r"C:\Library\archive.zim"),
        (r"\\?\C:\Library\archive.zim.", r"C:\Library\archive.zim."),
        (r"\\?\C:\Library\archive.zim ", r"C:\Library\archive.zim "),
        (r"\\?\C:\Library. \archive.zim", r"C:\Library\archive.zim"),
        (
            r"\\?\UNC\Server\Share\archive.zim.",
            r"\\Server\Share\archive.zim.",
        ),
        (
            r"\\?\UNC\Server\Share\archive.zim ",
            r"\\Server\Share\archive.zim",
        ),
        (r"C:\Library\archive .zim", r"C:\Library\archive.zim"),
        (r"C:\Library\archive.zim.", r"D:\Library\archive.zim"),
        (
            r"\\Server\Share\archive.zim.",
            r"\\Server\Other\archive.zim",
        ),
    ] {
        assert!(
            !acquisition_paths_equal(left, right, "/working"),
            "{left:?} == {right:?}"
        );
        assert!(
            !acquisition_paths_equal(right, left, "/working"),
            "{right:?} == {left:?}"
        );
    }
    #[cfg(not(windows))]
    assert!(!acquisition_paths_equal(
        "/library/archive.zim.",
        "/library/archive.zim",
        "/working"
    ));
}

#[test]
fn acquisition_win32_alias_reservation_blocks_real_commit_and_preserves_principal() {
    let profile = Profile::new();
    let store = Store::new(profile.0.clone()).unwrap();
    let destination = fs::canonicalize(&profile.0).unwrap().join("installed.zim");
    let prepared = profile.0.join("replacement.part");
    fs::write(&prepared, b"fresh archive B with the same UUID").unwrap();
    let native = destination.to_str().unwrap();
    let ordinary = if let Some(tail) = native.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{tail}")
    } else {
        native.strip_prefix(r"\\?\").unwrap_or(native).into()
    };
    let mut book = fixture_book();
    // Backslash syntax exercises Win32 normalization even on Unix test hosts.
    book.path = format!("{}. ", ordinary.replace('/', r"\"));
    book.available = false;
    let original_path = book.path.clone();
    let installation = book.installation_id.clone();
    store
        .update(|state| {
            state.books.push(book);
            state.reader_storage.insert(
                installation.clone(),
                serde_json::json!({"localStorage":{"sentinel":"archive A"}}),
            );
            Ok(())
        })
        .unwrap();
    for existing in [false, true] {
        if existing {
            fs::write(&destination, b"reserved destination sentinel").unwrap();
        }
        let mut called = false;
        let error = store
            .with_acquisition_destination(&destination, || {
                called = true;
                fs::rename(&prepared, &destination).map_err(|error| error.to_string())
            })
            .unwrap_err();
        assert!(error.contains("reserved by an installed library book"));
        assert!(!called);
        assert_eq!(
            fs::read(&prepared).unwrap(),
            b"fresh archive B with the same UUID"
        );
        if existing {
            assert_eq!(
                fs::read(&destination).unwrap(),
                b"reserved destination sentinel"
            );
        } else {
            assert!(!destination.exists());
        }
    }
    drop(store);
    let reopened = Store::new(profile.0.clone()).unwrap().read().unwrap();
    assert_eq!(reopened.books[0].path, original_path);
    assert_eq!(reopened.books[0].installation_id, installation);
    assert_eq!(
        reopened.reader_storage[&installation]["localStorage"]["sentinel"],
        "archive A"
    );
}

#[cfg(windows)]
#[test]
fn acquisition_extended_trailing_dot_filename_is_a_distinct_real_destination() {
    let profile = Profile::new();
    let store = Store::new(profile.0.clone()).unwrap();
    let directory = fs::canonicalize(&profile.0).unwrap();
    let original = directory.join("installed.zim");
    let destination = directory.join("installed.zim.");
    let prepared = directory.join("replacement.part");
    assert!(destination.to_str().unwrap().starts_with(r"\\?\"));
    fs::write(&original, b"ordinary filename sentinel").unwrap();
    fs::write(&prepared, b"distinct verbatim filename").unwrap();
    let mut book = fixture_book();
    let native = original.to_str().unwrap();
    book.path = if let Some(tail) = native.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{tail}.")
    } else {
        format!("{}.", native.strip_prefix(r"\\?\").unwrap())
    };
    let installation = book.installation_id.clone();
    store
        .update(|state| {
            state.books.push(book);
            state.reader_storage.insert(
                installation.clone(),
                serde_json::json!({"localStorage":{"sentinel":"archive A"}}),
            );
            Ok(())
        })
        .unwrap();
    store
        .with_acquisition_destination(&destination, || {
            fs::rename(&prepared, &destination).map_err(|error| error.to_string())
        })
        .unwrap();
    assert!(!prepared.exists());
    assert_eq!(fs::read(original).unwrap(), b"ordinary filename sentinel");
    assert_eq!(
        fs::read(destination).unwrap(),
        b"distinct verbatim filename"
    );
    assert_eq!(
        store.read().unwrap().reader_storage[&installation]["localStorage"]["sentinel"],
        "archive A"
    );
}

#[test]
fn acquisition_alias_comparison_never_merges_public_book_storage_principals() {
    let profile = Profile::new();
    let store = Store::new(profile.0.clone()).unwrap();
    let mut first = fixture_book();
    first.path = r"C:\Library\archive.zim. ".into();
    let mut second = first.clone();
    second.metadata.id = new_id();
    second.installation_id = new_installation_id();
    second.path = r"\\?\C:\Library\archive.zim".into();
    let first_installation = first.installation_id.clone();
    let second_installation = second.installation_id.clone();
    assert!(acquisition_paths_equal(
        &first.path,
        &second.path,
        "/working"
    ));
    store
        .update(|state| {
            state.books.extend([first.clone(), second.clone()]);
            state.reader_storage.insert(
                first_installation.clone(),
                serde_json::json!({"localStorage":{"sentinel":"archive A"}}),
            );
            state.reader_storage.insert(
                second_installation.clone(),
                serde_json::json!({"localStorage":{"sentinel":"archive B"}}),
            );
            Ok(())
        })
        .unwrap();
    drop(store);
    let store = Store::new(profile.0.clone()).unwrap();
    assert_eq!(store.book(&first.metadata.id).unwrap(), Some(first));
    assert_eq!(store.book(&second.metadata.id).unwrap(), Some(second));
    let state = store.read().unwrap();
    assert_eq!(state.reader_storage.len(), 2);
    assert_eq!(
        state.reader_storage[&first_installation]["localStorage"]["sentinel"],
        "archive A"
    );
    assert_eq!(
        state.reader_storage[&second_installation]["localStorage"]["sentinel"],
        "archive B"
    );
}

#[test]
fn acquisition_alias_cannot_publish_into_a_missing_book_path() {
    let profile = Profile::new();
    let store = Store::new(profile.0.clone()).unwrap();
    let destination = profile.0.join("STRASSE-caf\u{e9}.zim");
    let prepared = profile.0.join("replacement.part");
    fs::write(&prepared, b"replacement").unwrap();
    let mut book = fixture_book();
    book.path = profile
        .0
        .join("stra\u{df}e-cafe\u{301}.zim")
        .to_str()
        .unwrap()
        .into();
    book.available = false;
    store
        .update(|state| {
            state.books.push(book);
            Ok(())
        })
        .unwrap();
    assert!(store
        .with_acquisition_destination(&destination, || {
            fs::rename(&prepared, &destination).map_err(|error| error.to_string())
        })
        .is_err());
    assert_eq!(fs::read(prepared).unwrap(), b"replacement");
    assert!(!destination.exists());
}

#[test]
fn acquisition_allows_a_real_unregistered_commit_and_preserves_errors() {
    let profile = Profile::new();
    let store = Store::new(profile.0.clone()).unwrap();
    let destination = profile.0.join("new.zim");
    let prepared = profile.0.join("prepared.part");
    fs::write(&prepared, b"verified archive").unwrap();
    let result = store
        .with_acquisition_destination(&destination, || {
            fs::rename(&prepared, &destination).map_err(|error| error.to_string())?;
            Ok(42)
        })
        .unwrap();
    assert_eq!(result, 42);
    assert!(!prepared.exists());
    assert_eq!(fs::read(&destination).unwrap(), b"verified archive");
    assert_eq!(
        store.with_acquisition_destination(&destination, || {
            Err::<(), _>("commit failed".into())
        }),
        Err("commit failed".into())
    );
}

#[test]
fn acquisition_checks_both_readonly_modes_before_real_publication() {
    for saved in [false, true] {
        let profile = Profile::new();
        let store = Store::new(profile.0.clone()).unwrap();
        let destination = profile.0.join("new.zim");
        let prepared = profile.0.join("prepared.part");
        fs::write(&prepared, b"prepared archive").unwrap();
        if saved {
            store
                .update(|state| {
                    state.settings.read_only = true;
                    Ok(())
                })
                .unwrap();
        } else {
            store.set_runtime_mode(false, true).unwrap();
        }
        let mut called = false;
        let error = store
            .with_acquisition_destination(&destination, || {
                called = true;
                fs::rename(&prepared, &destination).map_err(|error| error.to_string())
            })
            .unwrap_err();
        assert!(error.contains("read-only"));
        assert!(!called);
        assert_eq!(fs::read(prepared).unwrap(), b"prepared archive");
        assert!(!destination.exists());
    }
}

#[test]
fn acquisition_commit_holds_the_library_mutation_lock() {
    use std::sync::{mpsc, TryLockError};
    use std::thread;

    let profile = Profile::new();
    let store = Store::new(profile.0.clone()).unwrap();
    let destination = profile.0.join("new.zim");
    let prepared = profile.0.join("prepared.part");
    fs::write(&prepared, b"verified archive").unwrap();
    let mut book = fixture_book();
    book.path = destination.to_str().unwrap().into();
    let (started_tx, started_rx) = mpsc::channel();
    let (entered_tx, entered_rx) = mpsc::channel();
    thread::scope(|scope| {
        let worker = store
            .with_acquisition_destination(&destination, || {
                assert!(matches!(
                    store.inner.try_lock(),
                    Err(TryLockError::WouldBlock)
                ));
                let worker = scope.spawn(|| {
                    started_tx.send(()).unwrap();
                    store
                        .update(|state| {
                            entered_tx.send(()).unwrap();
                            assert_eq!(fs::read(&destination).unwrap(), b"verified archive");
                            state.books.push(book);
                            Ok(())
                        })
                        .unwrap();
                });
                started_rx.recv().unwrap();
                assert!(matches!(
                    entered_rx.try_recv(),
                    Err(mpsc::TryRecvError::Empty)
                ));
                fs::rename(&prepared, &destination).map_err(|error| error.to_string())?;
                Ok(worker)
            })
            .unwrap();
        worker.join().unwrap();
    });
    assert_eq!(
        store.read().unwrap().books[0].path,
        destination.to_str().unwrap()
    );
    assert_eq!(fs::read(destination).unwrap(), b"verified archive");
}

#[test]
fn acquisition_observes_a_reservation_committed_while_waiting_for_the_lock() {
    use std::sync::mpsc;
    use std::thread;

    let profile = Profile::new();
    let store = Store::new(profile.0.clone()).unwrap();
    let destination = profile.0.join("reserved.zim");
    let prepared = profile.0.join("prepared.part");
    fs::write(&prepared, b"replacement archive").unwrap();
    let mut book = fixture_book();
    book.path = destination.to_str().unwrap().into();
    book.available = false;
    let (locked_tx, locked_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (attempting_tx, attempting_rx) = mpsc::channel();
    thread::scope(|scope| {
        let store = &store;
        let destination = &destination;
        let prepared = &prepared;
        let mutation = scope.spawn(move || {
            store
                .update(|state| {
                    state.books.push(book);
                    locked_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok(())
                })
                .unwrap();
        });
        locked_rx.recv().unwrap();
        let acquisition = scope.spawn(move || {
            attempting_tx.send(()).unwrap();
            store.with_acquisition_destination(destination, || {
                fs::rename(prepared, destination).map_err(|error| error.to_string())
            })
        });
        attempting_rx.recv().unwrap();
        release_tx.send(()).unwrap();
        mutation.join().unwrap();
        assert!(acquisition
            .join()
            .unwrap()
            .unwrap_err()
            .contains("reserved by an installed library book"));
    });
    assert_eq!(fs::read(prepared).unwrap(), b"replacement archive");
    assert!(!destination.exists());
    assert!(!store.read().unwrap().books[0].available);
}

fn legacy_profile(profile: &Profile, read_only: bool) -> (String, Vec<u8>) {
    drop(Store::new(profile.0.clone()).unwrap());
    let book = fixture_book();
    let id = book.metadata.id.clone();
    let mut state = State::default();
    state.books.push(book);
    state.settings.read_only = read_only;
    state.reader_storage.insert(
        id.clone(),
        serde_json::json!({"localStorage":{"saved":"archive A"},"cookies":[]}),
    );
    state.reader_storage.insert(
        "orphaned-archive".into(),
        serde_json::json!({"localStorage":{"saved":"orphan"}}),
    );
    let mut json = serde_json::to_value(state).unwrap();
    json["version"] = serde_json::json!(1);
    json["books"][0]
        .as_object_mut()
        .unwrap()
        .remove("installationId");
    let bytes = serde_json::to_vec_pretty(&json).unwrap();
    fs::write(profile.0.join("state.json"), &bytes).unwrap();
    (id, bytes)
}

#[test]
fn version_one_installation_migration_preserves_storage_and_is_idempotent() {
    let profile = Profile::new();
    let (id, original) = legacy_profile(&profile, false);
    let store = Store::new(profile.0.clone()).unwrap();
    let state = store.read().unwrap();
    let installation = state.books[0].installation_id.clone();
    assert_ne!(installation, id);
    assert_eq!(state.version, STATE_VERSION);
    assert!(!state.reader_storage.contains_key(&id));
    assert_eq!(
        state.reader_storage[&installation]["localStorage"]["saved"],
        "archive A"
    );
    assert_eq!(
        state.reader_storage["orphaned-archive"]["localStorage"]["saved"],
        "orphan"
    );
    let migrated = fs::read(profile.0.join("state.json")).unwrap();
    assert_ne!(migrated, original);
    drop(store);
    let reopened = Store::new(profile.0.clone()).unwrap();
    assert_eq!(
        reopened.read().unwrap().books[0].installation_id,
        installation
    );
    assert_eq!(fs::read(profile.0.join("state.json")).unwrap(), migrated);
}

#[test]
fn readonly_legacy_migration_is_memory_only_for_runtime_and_saved_preference() {
    for saved_read_only in [false, true] {
        let profile = Profile::new();
        let (id, original) = legacy_profile(&profile, saved_read_only);
        let store = Store::new_with_mode(profile.0.clone(), !saved_read_only).unwrap();
        let state = store.read().unwrap();
        let installation = &state.books[0].installation_id;
        assert_eq!(state.version, STATE_VERSION);
        assert_ne!(installation, &id);
        assert!(!state.reader_storage.contains_key(&id));
        assert_eq!(
            state.reader_storage[installation]["localStorage"]["saved"],
            "archive A"
        );
        assert_eq!(fs::read(profile.0.join("state.json")).unwrap(), original);
        drop(store);
        assert_eq!(fs::read(profile.0.join("state.json")).unwrap(), original);
    }
}

#[test]
fn private_startup_migrates_durable_normal_storage_without_exposing_it() {
    let profile = Profile::new();
    legacy_profile(&profile, false);
    let path = profile.0.join("state.json");
    let mut json: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    json["settings"]["privateMode"] = serde_json::json!(true);
    fs::write(&path, serde_json::to_vec_pretty(&json).unwrap()).unwrap();
    let store = Store::new(profile.0.clone()).unwrap();
    let visible = store.read().unwrap();
    let installation = visible.books[0].installation_id.clone();
    assert!(visible.reader_storage.is_empty());
    let durable: State = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(durable.version, STATE_VERSION);
    assert_eq!(
        durable.reader_storage[&installation]["localStorage"]["saved"],
        "archive A"
    );
    store
        .update(|state| {
            state.settings.private_mode = false;
            Ok(())
        })
        .unwrap();
    assert_eq!(
        store.read().unwrap().reader_storage[&installation]["localStorage"]["saved"],
        "archive A"
    );
}

#[test]
fn version_two_missing_installation_skips_that_book_and_keeps_the_profile() {
    let profile = Profile::new();
    let (_, original) = legacy_profile(&profile, false);
    let mut json: serde_json::Value = serde_json::from_slice(&original).unwrap();
    json["version"] = serde_json::json!(STATE_VERSION);
    let bytes = serde_json::to_vec_pretty(&json).unwrap();
    fs::write(profile.0.join("state.json"), &bytes).unwrap();
    let store = Store::new(profile.0.clone()).unwrap();
    assert!(store.read().unwrap().books.is_empty());
    assert!(store
        .load_warnings()
        .iter()
        .any(|warning| warning.contains("book")));
    drop(store);
    // A skipped record is retained on disk; the profile is not normalized over it.
    let raw: serde_json::Value =
        serde_json::from_slice(&fs::read(profile.0.join("state.json")).unwrap()).unwrap();
    assert!(!raw["books"].as_array().unwrap().is_empty());
    let store = Store::new(profile.0.clone()).unwrap();
    assert!(store
        .load_warnings()
        .iter()
        .any(|warning| warning.contains("book")));
}

#[test]
fn unreadable_book_records_are_skipped_and_reported() {
    let profile = Profile::new();
    let store = Store::new(profile.0.clone()).unwrap();
    store
        .update(|state| {
            state.books.push(fixture_book());
            Ok(())
        })
        .unwrap();
    drop(store);
    let mut json: serde_json::Value =
        serde_json::from_slice(&fs::read(profile.0.join("state.json")).unwrap()).unwrap();
    let mut broken = json["books"][0].clone();
    broken["id"] = serde_json::json!("broken-book");
    broken["installationId"] = serde_json::json!(new_installation_id());
    broken["path"] = serde_json::json!("");
    json["books"].as_array_mut().unwrap().push(broken);
    // A record that cannot even deserialize must be skipped too.
    json["books"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"id": "not-a-book"}));
    let bytes = serde_json::to_vec_pretty(&json).unwrap();
    fs::write(profile.0.join("state.json"), &bytes).unwrap();
    let store = Store::new(profile.0.clone()).unwrap();
    assert_eq!(store.read().unwrap().books.len(), 1);
    assert!(store
        .load_warnings()
        .iter()
        .filter(|warning| warning.contains("book"))
        .count()
        >= 2);
    drop(store);
    let raw: serde_json::Value =
        serde_json::from_slice(&fs::read(profile.0.join("state.json")).unwrap()).unwrap();
    assert_eq!(raw["books"].as_array().unwrap().len(), 3);
    let store = Store::new(profile.0.clone()).unwrap();
    assert_eq!(store.read().unwrap().books.len(), 1);
    assert!(store
        .load_warnings()
        .iter()
        .filter(|warning| warning.contains("book"))
        .count()
        >= 2);
}

#[test]
fn unknown_saved_window_surface_falls_back_to_library() {
    let profile = Profile::new();
    let store = Store::new(profile.0.clone()).unwrap();
    drop(store);
    let mut json: serde_json::Value =
        serde_json::from_slice(&fs::read(profile.0.join("state.json")).unwrap()).unwrap();
    json["windows"] = serde_json::json!({
        "main": {
            "tabs": [],
            "activeTab": null,
            "geometry": null,
            "activeSurface": "not-a-surface"
        }
    });
    fs::write(
        profile.0.join("state.json"),
        serde_json::to_vec_pretty(&json).unwrap(),
    )
    .unwrap();
    let store = Store::new(profile.0.clone()).unwrap();
    assert_eq!(
        store.read().unwrap().windows["main"].active_surface,
        "library"
    );
    assert!(store.load_warnings().is_empty());
    drop(store);
    let store = Store::new(profile.0.clone()).unwrap();
    assert_eq!(
        store.read().unwrap().windows["main"].active_surface,
        "library"
    );
    assert!(store.load_warnings().is_empty());
}

#[test]
fn legacy_window_fields_migrate_without_losing_tabs_or_warning() {
    let profile = Profile::new();
    let store = Store::new(profile.0.clone()).unwrap();
    drop(store);
    let mut json: serde_json::Value =
        serde_json::from_slice(&fs::read(profile.0.join("state.json")).unwrap()).unwrap();
    json["windows"] = serde_json::json!({
        "main": {
            "tabs": [{
                "id": "legacy-tab",
                "bookId": null,
                "path": "",
                "title": "Welcome",
                "scroll": 0.0,
                "retiredField": "ignored"
            }],
            "activeTab": "legacy-tab",
            "activeSurface": "maps",
            "legacyWindowField": {"kept": "nowhere"}
        }
    });
    fs::write(
        profile.0.join("state.json"),
        serde_json::to_vec_pretty(&json).unwrap(),
    )
    .unwrap();
    let store = Store::new(profile.0.clone()).unwrap();
    let state = store.read().unwrap();
    assert_eq!(state.windows["main"].tabs.len(), 1);
    assert_eq!(state.windows["main"].active_tab.as_deref(), Some("legacy-tab"));
    assert_eq!(state.windows["main"].active_surface, "maps");
    assert!(store.load_warnings().is_empty());
    drop(store);
    let store = Store::new(profile.0.clone()).unwrap();
    assert_eq!(store.read().unwrap().windows["main"].tabs.len(), 1);
    assert!(store.load_warnings().is_empty());
}

#[test]
fn installation_principals_must_be_unique_canonical_uuid_v4() {
    let book = fixture_book();
    let mut state = State::default();
    state.books.push(book.clone());
    state.validate().unwrap();
    for invalid in ["", "not-a-uuid", "00000000-0000-0000-0000-000000000000"] {
        state.books[0].installation_id = invalid.into();
        assert!(state.validate().is_err());
    }
    state.books[0] = book.clone();
    let mut duplicate = book.clone();
    duplicate.metadata.id = new_id();
    state.books.push(duplicate);
    assert!(state.validate().is_err());
    state.books[1].installation_id = new_installation_id();
    state.validate().unwrap();
    let mut other_installation = book.clone();
    other_installation.installation_id = new_installation_id();
    assert_ne!(book, other_installation);
}

#[test]
fn private_library_replacement_cannot_inherit_preserved_installation_storage() {
    let profile = Profile::new();
    let store = Store::new(profile.0.clone()).unwrap();
    let book = fixture_book();
    let old_installation = book.installation_id.clone();
    store
        .update(|state| {
            state.books.push(book.clone());
            state.reader_storage.insert(
                old_installation.clone(),
                serde_json::json!({"localStorage":{"saved":"archive A"}}),
            );
            // Even orphaned archive-UUID data must never become a new principal.
            state.reader_storage.insert(
                book.metadata.id.clone(),
                serde_json::json!({"localStorage":{"saved":"orphan"}}),
            );
            Ok(())
        })
        .unwrap();
    store.set_runtime_mode(true, false).unwrap();
    let mut replacement = book.clone();
    replacement.installation_id = new_installation_id();
    replacement.path = "/different/archive.zim".into();
    let new_installation = replacement.installation_id.clone();
    store
        .update(|state| {
            state.books[0] = replacement;
            Ok(())
        })
        .unwrap();
    store.set_runtime_mode(false, false).unwrap();
    let state = store.read().unwrap();
    assert_eq!(state.books[0].installation_id, new_installation);
    assert_eq!(
        state.reader_storage[&old_installation]["localStorage"]["saved"],
        "archive A"
    );
    assert!(!state.reader_storage.contains_key(&new_installation));
    drop(store);
    let reopened = Store::new(profile.0.clone()).unwrap().read().unwrap();
    assert_eq!(reopened.books[0].installation_id, new_installation);
    assert!(!reopened.reader_storage.contains_key(&new_installation));
}

#[test]
fn failed_transactions_preserve_memory_and_disk() {
    let profile = Profile::new();
    let store = Store::new(profile.0.clone()).unwrap();
    let original = fs::read(profile.0.join("state.json")).unwrap();
    assert!(store
        .update(|state| {
            state.settings.zoom = 2.0;
            Err::<(), _>("abort".into())
        })
        .is_err());
    assert!(store
        .update(|state| {
            state.settings.zoom = f64::NAN;
            Ok(())
        })
        .is_err());
    assert_eq!(store.read().unwrap().settings.zoom, 1.0);
    assert_eq!(fs::read(profile.0.join("state.json")).unwrap(), original);
    store
        .update(|state| {
            state.settings.zoom = 2.0;
            Ok(())
        })
        .unwrap();
    drop(store);
    assert_eq!(
        Store::new(profile.0.clone())
            .unwrap()
            .read()
            .unwrap()
            .settings
            .zoom,
        2.0
    );
}

#[test]
fn second_instance_and_runtime_readonly_cannot_write() {
    let profile = Profile::new();
    let store = Store::new(profile.0.clone()).unwrap();
    assert!(Store::new(profile.0.clone()).is_err());
    store.set_runtime_mode(false, true).unwrap();
    assert!(store
        .update(|state| {
            state.settings.zoom = 2.0;
            Ok(())
        })
        .is_err());
    assert!(store.read().unwrap().settings.read_only);
    drop(store);
    let readonly = Store::new_with_mode(profile.0.clone(), true).unwrap();
    assert!(readonly.update(|_| Ok(())).is_err());
}

#[test]
fn readonly_preference_blocks_late_background_transactions() {
    let profile = Profile::new();
    let store = Store::new(profile.0.clone()).unwrap();
    store
        .update(|state| {
            state.settings.read_only = true;
            Ok(())
        })
        .unwrap();
    let original = fs::read(profile.0.join("state.json")).unwrap();
    assert!(store
        .update(|state| {
            state
                .reader_storage
                .insert("late".into(), serde_json::json!({"secret":true}));
            Ok(())
        })
        .is_err());
    assert_eq!(fs::read(profile.0.join("state.json")).unwrap(), original);
    assert!(!store.read().unwrap().reader_storage.contains_key("late"));
    store
        .update(|state| {
            state.settings.read_only = false;
            Ok(())
        })
        .unwrap();
    store
        .update(|state| {
            state.settings.zoom = 1.5;
            Ok(())
        })
        .unwrap();
    assert_eq!(store.settings().unwrap().zoom, 1.5);
}

#[cfg(desktop)]
#[test]
fn session_snapshot_covers_only_open_windows_and_respects_privacy_modes() {
    let profile = Profile::new();
    let store = Store::new(profile.0.clone()).unwrap();
    assert!(store.saved_session().unwrap().is_none());
    let open: HashSet<String> = ["main".to_string()].into_iter().collect();
    // An empty session never creates or replaces a snapshot.
    assert!(!store.save_session_snapshot(&open).unwrap());
    assert!(store.saved_session().unwrap().is_none());
    store
        .update(|state| {
            state.windows.insert(
                "main".into(),
                WindowSession { tabs: vec![tab("main-tab", "A/One")], active_tab: Some("main-tab".into()), geometry: None, native_tab: None, active_surface: "reader".into() },
            );
            // A window that was open in an earlier run but not at Quit.
            state.windows.insert(
                "window-stale".into(),
                WindowSession { tabs: vec![tab("stale-tab", "A/Stale")], active_tab: Some("stale-tab".into()), geometry: None, native_tab: None, active_surface: "reader".into() },
            );
            state.tabs = vec![tab("main-tab", "A/One")];
            state.active_tab = Some("main-tab".into());
            Ok(())
        })
        .unwrap();
    assert!(store.save_session_snapshot(&open).unwrap());
    let saved = store.saved_session().unwrap().unwrap();
    assert_eq!(saved.windows.len(), 1);
    assert_eq!(saved.windows["main"].tabs[0].path, "A/One");

    // Private browsing never replaces the saved session with private locations.
    let before = fs::read(profile.0.join(SESSION_SNAPSHOT_FILE)).unwrap();
    store
        .update(|state| {
            state.settings.private_mode = true;
            Ok(())
        })
        .unwrap();
    store
        .update(|state| {
            state.windows.insert(
                "main".into(),
                WindowSession { tabs: vec![tab("private-tab", "A/Secret")], active_tab: Some("private-tab".into()), geometry: None, native_tab: None, active_surface: "reader".into() },
            );
            Ok(())
        })
        .unwrap();
    assert!(!store.save_session_snapshot(&open).unwrap());
    assert_eq!(fs::read(profile.0.join(SESSION_SNAPSHOT_FILE)).unwrap(), before);
    assert_eq!(
        store.saved_session().unwrap().unwrap().windows["main"].tabs[0].path,
        "A/One"
    );

    // Read-only profiles keep the previous snapshot as well.
    store
        .update(|state| {
            state.settings.private_mode = false;
            Ok(())
        })
        .unwrap();
    store
        .update(|state| {
            state.settings.read_only = true;
            Ok(())
        })
        .unwrap();
    assert!(!store.save_session_snapshot(&open).unwrap());
    assert_eq!(fs::read(profile.0.join(SESSION_SNAPSHOT_FILE)).unwrap(), before);
}

#[cfg(desktop)]
#[test]
fn closed_main_window_is_not_resurrected_by_session_snapshot() {
    let profile = Profile::new();
    let store = Store::new(profile.0.clone()).unwrap();
    store
        .update(|state| {
            state.windows.insert(
                "main".into(),
                WindowSession { tabs: vec![tab("old-main", "A/Old")], active_tab: Some("old-main".into()), geometry: None, native_tab: None, active_surface: "reader".into() },
            );
            state.windows.insert(
                "window-1".into(),
                WindowSession { tabs: vec![tab("secondary", "A/Live")], active_tab: Some("secondary".into()), geometry: None, native_tab: None, active_surface: "reader".into() },
            );
            // Legacy live-session tabs must not stand in for a closed main.
            state.tabs = vec![tab("legacy-main", "A/Legacy")];
            state.active_tab = Some("legacy-main".into());
            Ok(())
        })
        .unwrap();
    let open: HashSet<String> = ["window-1".to_string()].into_iter().collect();
    assert!(store.save_session_snapshot(&open).unwrap());
    let saved = store.saved_session().unwrap().unwrap();
    // The empty entry stays because validation requires a main window, but
    // neither the stale window nor the legacy tab list may fill it.
    assert!(saved.windows.contains_key("main"));
    assert!(saved.windows["main"].tabs.is_empty());
    assert!(saved.windows["main"].active_tab.is_none());
    assert_eq!(saved.windows["window-1"].tabs[0].path, "A/Live");
}

#[cfg(desktop)]
#[test]
fn oversized_session_snapshot_is_refused_without_replacing_previous() {
    let profile = Profile::new();
    let store = Store::new(profile.0.clone()).unwrap();
    let open: HashSet<String> = ["main".to_string()].into_iter().collect();
    store
        .update(|state| {
            state.windows.insert(
                "main".into(),
                WindowSession { tabs: vec![tab("small", "A/One")], active_tab: Some("small".into()), geometry: None, native_tab: None, active_surface: "reader".into() },
            );
            state.tabs = vec![tab("small", "A/One")];
            state.active_tab = Some("small".into());
            Ok(())
        })
        .unwrap();
    assert!(store.save_session_snapshot(&open).unwrap());
    let before = fs::read(profile.0.join(SESSION_SNAPSHOT_FILE)).unwrap();
    let huge = format!("A/{}", "x".repeat(MAX_SESSION_BYTES as usize + 1024));
    store
        .update(|state| {
            state.windows.insert(
                "main".into(),
                WindowSession { tabs: vec![tab("huge", &huge)], active_tab: Some("huge".into()), geometry: None, native_tab: None, active_surface: "reader".into() },
            );
            Ok(())
        })
        .unwrap();
    let error = store.save_session_snapshot(&open).unwrap_err();
    assert!(error.contains("exceeds the supported size"), "{error}");
    assert_eq!(
        fs::read(profile.0.join(SESSION_SNAPSHOT_FILE)).unwrap(),
        before
    );
    // The rejected write must not leave its temporary file behind either.
    assert!(!fs::read_dir(&profile.0).unwrap().any(|entry| {
        entry
            .ok()
            .and_then(|entry| entry.file_name().into_string().ok())
            .is_some_and(|name| name.starts_with(".last-session-") && name.ends_with(".tmp"))
    }));
    assert_eq!(
        store.saved_session().unwrap().unwrap().windows["main"].tabs[0].path,
        "A/One"
    );
}

/// Windows LockFileEx contention is ERROR_LOCK_VIOLATION, not WouldBlock; a
/// held profile must still be "busy", while other OS errors stay fatal.
#[cfg(all(windows, desktop))]
#[test]
fn windows_lock_violation_is_a_busy_profile_and_other_errors_are_fatal() {
    let path = Path::new(r"C:\profile\store.lock");
    let busy = lock_failure(path, std::io::Error::from_raw_os_error(33));
    assert!(matches!(busy, ProfileLockError::Busy { .. }));
    assert!(matches!(StoreOpenError::from(busy), StoreOpenError::Busy(_)));
    for code in [1, 5, 32] { // ERROR_INVALID_FUNCTION, ACCESS_DENIED, SHARING_VIOLATION
        let fatal = lock_failure(path, std::io::Error::from_raw_os_error(code));
        assert!(matches!(fatal, ProfileLockError::Lock { .. }), "os error {code}");
    }
}

#[cfg(all(unix, desktop))]
#[test]
fn non_would_block_lock_failures_are_fatal_with_the_os_error() {
    let path = Path::new("/tmp/store.lock");
    let busy_source = std::io::Error::from_raw_os_error(libc::EWOULDBLOCK);
    assert_eq!(busy_source.kind(), std::io::ErrorKind::WouldBlock);
    let busy = lock_failure(path, busy_source);
    assert!(matches!(busy, ProfileLockError::Busy { .. }));
    assert!(matches!(StoreOpenError::from(busy), StoreOpenError::Busy(_)));

    // No NFS/SMB mount is available on this machine, so inject the errors a
    // filesystem without advisory locks returns from the lock call.
    for raw in [libc::ENOLCK, libc::EOPNOTSUPP] {
        let source = std::io::Error::from_raw_os_error(raw);
        let expected = source.to_string();
        let error = lock_failure(path, source);
        assert!(matches!(error, ProfileLockError::Lock { .. }));
        let message = error.to_string();
        assert!(
            message.contains(&expected),
            "{message} must report the OS error {expected}"
        );
        assert!(
            !message.contains("Another native/headless instance may be using this profile"),
            "{message} must not claim another instance owns the profile"
        );
        match StoreOpenError::from(error) {
            StoreOpenError::Fatal(reported) => {
                assert!(reported.contains(&expected));
                assert!(!reported.contains("another instance"));
            }
            StoreOpenError::Busy(_) => {
                panic!("a non-lock filesystem error must be fatal, not busy")
            }
        }
    }
}

#[test]
fn saved_session_rejects_unknown_windows_and_missing_main() {
    let empty = SessionSnapshot { version: SESSION_SNAPSHOT_VERSION, windows: HashMap::new(), reference_revision: 0 };
    assert!(empty.validate().is_err());
    assert!(!empty.has_content());
    let mut windows = HashMap::new();
    windows.insert("evil".into(), WindowSession::default());
    let unknown = SessionSnapshot { version: SESSION_SNAPSHOT_VERSION, windows, reference_revision: 0 };
    assert!(unknown.validate().is_err());
    let mut windows = HashMap::new();
    windows.insert("main".into(), WindowSession::default());
    windows.insert("window-1".into(), WindowSession::default());
    let valid = SessionSnapshot { version: SESSION_SNAPSHOT_VERSION, windows, reference_revision: 0 };
    assert!(valid.validate().is_ok());
    let mut windows = HashMap::new();
    windows.insert(
        "main".into(),
        WindowSession {
            tabs: vec![tab("a", "A/One")],
            active_tab: Some("missing".into()),
            ..Default::default()
        },
    );
    let stale_active = SessionSnapshot { version: SESSION_SNAPSHOT_VERSION, windows, reference_revision: 0 };
    assert!(stale_active.validate().is_err());
}

#[test]
fn private_windows_and_storage_never_replace_durable_session() {
    let profile = Profile::new();
    let store = Store::new(profile.0.clone()).unwrap();
    store
        .update(|state| {
            state.windows.insert(
                "main".into(),
                WindowSession { tabs: vec![tab("old", "A/Before")], active_tab: Some("old".into()), geometry: None, native_tab: None, active_surface: default_active_surface() },
            );
            state.reader_storage.insert(
                "missing-archive".into(),
                serde_json::json!({"cookie":"before"}),
            );
            Ok(())
        })
        .unwrap();
    store
        .update(|state| {
            state.settings.private_mode = true;
            Ok(())
        })
        .unwrap();
    let original = fs::read(profile.0.join("state.json")).unwrap();
    store
        .update(|state| {
            state.windows.insert(
                "main".into(),
                WindowSession { tabs: vec![tab("private", "A/Secret")], active_tab: Some("private".into()), geometry: None, native_tab: None, active_surface: default_active_surface() },
            );
            state.reader_storage.insert(
                "missing-archive".into(),
                serde_json::json!({"cookie":"secret"}),
            );
            Ok(())
        })
        .unwrap();
    assert_eq!(fs::read(profile.0.join("state.json")).unwrap(), original);
    assert_eq!(store.read().unwrap().windows["main"].tabs[0].id, "private");
    store
        .update(|state| {
            state.settings.zoom = 1.5;
            Ok(())
        })
        .unwrap();
    let disk: State =
        serde_json::from_slice(&fs::read(profile.0.join("state.json")).unwrap()).unwrap();
    assert_eq!(disk.windows["main"].tabs[0].id, "old");
    assert_eq!(disk.reader_storage["missing-archive"]["cookie"], "before");
    store
        .update(|state| {
            state.settings.private_mode = false;
            Ok(())
        })
        .unwrap();
    assert_eq!(store.read().unwrap().windows["main"].tabs[0].id, "old");
}

#[test]
fn corrupt_or_future_state_is_not_reset() {
    let profile = Profile::new();
    drop(Store::new(profile.0.clone()).unwrap());
    for bytes in [b"{broken".as_slice(), br#"{"version":999}"#.as_slice()] {
        fs::write(profile.0.join("state.json"), bytes).unwrap();
        assert!(Store::new(profile.0.clone()).is_err());
        assert_eq!(fs::read(profile.0.join("state.json")).unwrap(), bytes);
    }
}

#[test]
fn unreadable_state_is_quarantined_byte_for_byte() {
    let profile = Profile::new();
    fs::create_dir_all(&profile.0).unwrap();
    let bytes = b"{\"version\": 2, \"books\": [";
    fs::write(profile.0.join("state.json"), bytes).unwrap();
    let kept = quarantine_state(&profile.0).unwrap();
    assert!(!profile.0.join("state.json").exists());
    assert_eq!(fs::read(&kept).unwrap(), bytes);
    assert!(kept.file_name().unwrap().to_string_lossy().starts_with("state.unreadable-"));
}

#[test]
fn reset_saved_windows_clears_layout_and_preserves_the_library() {
    let profile = Profile::new();
    fs::create_dir_all(&profile.0).unwrap();
    let mut state = State::default();
    state.books.push(fixture_book());
    state.windows.insert("main".into(), WindowSession { tabs: vec![Tab { restricted: false, id: "tab".into(), book_id: Some(state.books[0].metadata.id.clone()), path: "Main_Page".into(), title: "Archive".into(), scroll: 0.0, back: Vec::new(), forward: Vec::new() }], active_tab: Some("tab".into()), geometry: None, native_tab: None, active_surface: "reader".into() });
    state.tabs.push(Tab { restricted: false, id: "tab".into(), book_id: None, path: String::new(), title: String::new(), scroll: 0.0, back: Vec::new(), forward: Vec::new() });
    state.active_tab = Some("tab".into());
    let zoom = state.settings.zoom;
    fs::write(profile.0.join("state.json"), serde_json::to_vec(&state).unwrap()).unwrap();
    reset_saved_windows(&profile.0).unwrap();
    let reloaded: State = serde_json::from_slice(&fs::read(profile.0.join("state.json")).unwrap()).unwrap();
    assert_eq!(reloaded.books.len(), 1);
    assert!(reloaded.windows.is_empty());
    assert!(reloaded.tabs.is_empty());
    assert_eq!(reloaded.active_tab, None);
    assert_eq!(reloaded.settings.zoom, zoom);
}

#[test]
fn reset_saved_windows_refuses_a_saved_read_only_profile_before_touching_it() {
    let profile = Profile::new();
    fs::create_dir_all(&profile.0).unwrap();
    let mut state = State::default();
    state.settings.read_only = true;
    state.windows.insert("main".into(), WindowSession { tabs: Vec::new(), active_tab: None, geometry: None, native_tab: None, active_surface: "reader".into() });
    let bytes = serde_json::to_vec(&state).unwrap();
    fs::write(profile.0.join("state.json"), &bytes).unwrap();
    assert!(reset_saved_windows(&profile.0).is_err());
    // Neither the state nor a new process lock may be created.
    assert_eq!(fs::read(profile.0.join("state.json")).unwrap(), bytes);
    assert!(!profile.0.join("store.lock").exists());
}

#[cfg(mobile)]
fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        let metadata = entry.metadata().unwrap();
        if metadata.is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
            fs::set_permissions(&target, metadata.permissions()).unwrap();
        }
    }
    fs::set_permissions(destination, fs::metadata(source).unwrap().permissions()).unwrap();
}

#[cfg(mobile)]
#[test]
fn mobile_unreadable_state_is_quarantined_and_starts_empty_with_notice() {
    let profile = Profile::new();
    let store = Store::new_mobile(profile.0.clone(), false).unwrap();
    store.update(|state| { state.books.push(fixture_book()); Ok(()) }).unwrap();
    drop(store);
    let bytes = b"{not json".as_slice();
    fs::write(profile.0.join("state.json"), bytes).unwrap();
    let store = Store::new_mobile(profile.0.clone(), false).unwrap();
    assert!(store.read().unwrap().books.is_empty());
    assert!(!store.read_only().unwrap());
    assert!(store.load_warnings().iter().any(|warning| warning == crate::mobile_rebase::STATE_QUARANTINED));
    let kept: Vec<_> = fs::read_dir(&profile.0).unwrap()
        .map(Result::unwrap)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("state.unreadable-"))
        .collect();
    assert_eq!(kept.len(), 1);
    assert_eq!(fs::read(profile.0.join(&kept[0])).unwrap(), bytes);
    assert!(fs::read(profile.0.join("state.json")).is_ok());
    drop(store);
}

#[cfg(mobile)]
#[test]
fn mobile_malformed_edition_security_state_never_starts_an_empty_writable_profile() {
    for (key, malformed) in [
        ("editionIntents", serde_json::json!({"not": "an array"})),
        ("editionTransactions", serde_json::json!([{"id": "incomplete"}])),
        ("referenceRevision", serde_json::json!(1)),
    ] {
        let profile = Profile::new();
        let store = Store::new_mobile(profile.0.clone(), false).unwrap();
        store.update(|state| {
            state.books.push(fixture_book());
            state.consumed_acquisitions.insert("owned-download".into());
            Ok(())
        }).unwrap();
        drop(store);
        let path = profile.0.join("state.json");
        let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        value[key] = malformed;
        let original = serde_json::to_vec(&value).unwrap();
        fs::write(&path, &original).unwrap();
        assert!(Store::new_mobile(profile.0.clone(), false).is_err(), "{key}");
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(!fs::read_dir(&profile.0).unwrap().map(Result::unwrap)
            .any(|entry| entry.file_name().to_string_lossy().starts_with("state.unreadable-")));
    }
}

#[cfg(mobile)]
#[test]
fn mobile_corrupt_auxiliary_record_does_not_block_first_startup() {
    let profile = Profile::new();
    fs::create_dir_all(profile.0.join("downloads")).unwrap();
    fs::write(profile.0.join("downloads/downloads.json"), b"{not json").unwrap();
    let store = Store::new_mobile(profile.0.clone(), false).unwrap();
    assert!(profile.0.join("mobile-root-v1/root.json").is_file());
    assert_eq!(fs::read(profile.0.join("downloads/downloads.json")).unwrap(), b"{not json");
    drop(store);
}

/// A restored/copied container changes the profile directory identities but
/// the marker and the archives travel with the data. Startup must re-anchor
/// the library instead of refusing every saved archive.
#[cfg(mobile)]
#[test]
fn mobile_restored_container_reanchors_managed_book_and_opens() {
    let original = Profile::new();
    let restored = Profile::new();
    let store = Store::new_mobile(original.0.clone(), false).unwrap();
    let old_root = store.data_dir().to_path_buf();
    let archive = old_root.join("archives/book.zim");
    fs::create_dir_all(archive.parent().unwrap()).unwrap();
    fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim"), &archive).unwrap();
    let mut book = fixture_book();
    book.path = archive.to_string_lossy().into_owned();
    book.managed = true;
    let id = book.metadata.id.clone();
    store.update(|state| { state.books.push(book); Ok(()) }).unwrap();
    drop(store);
    copy_tree(&original.0, &restored.0);
    // The marker file travels, but its recorded directory identities are
    // those of the previous container.
    let marker_path = restored.0.join("mobile-root-v1/root.json");
    let mut marker: serde_json::Value = serde_json::from_slice(&fs::read(&marker_path).unwrap()).unwrap();
    marker["profile"] = serde_json::json!({"device": 1, "inode": 1});
    marker["capability"] = serde_json::json!({"device": 1, "inode": 1});
    fs::write(&marker_path, serde_json::to_vec(&marker).unwrap()).unwrap();
    let store = Store::new_mobile(restored.0.clone(), false).unwrap();
    let root = store.data_dir().to_path_buf();
    let expected = root.join("archives/book.zim");
    assert_eq!(store.read().unwrap().books[0].path, expected.to_string_lossy());
    assert!(expected.is_file());
    assert!(store.load_warnings().iter().any(|warning| warning.contains("app container")));
    drop(store);
    // The saved path is inside managed storage, and the archive still opens
    // by its durable identity.
    let service = crate::service::Service::new(restored.0.clone(), false, false).unwrap();
    assert_eq!(service.archive(&id).unwrap().metadata().unwrap().id, id);
}

#[cfg(mobile)]
#[test]
fn mobile_saved_readonly_starts_without_creating_marker_and_can_be_disabled() {
    let profile = Profile::new();
    let store = Store::new(profile.0.clone()).unwrap();
    store.update(|state| {
        state.settings.read_only = true;
        Ok(())
    }).unwrap();
    drop(store);
    let path = profile.0.join("state.json");
    let original = fs::read(&path).unwrap();
    let original_modified = fs::metadata(&path).unwrap().modified().unwrap();
    let store = Store::new_mobile(profile.0.clone(), false).unwrap();
    assert!(store.read_only().unwrap());
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), original_modified);
    assert!(!profile.0.join("mobile-root-v1").exists());
    store.update(|state| {
        state.settings.read_only = false;
        Ok(())
    }).unwrap();
    assert!(!store.read_only().unwrap());
    assert!(profile.0.join("mobile-root-v1/root.json").is_file());
    let durable: State = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert!(!durable.settings.read_only);
}

#[cfg(mobile)]
#[test]
fn mobile_saved_readonly_requires_existing_lock_without_repair() {
    let profile = Profile::new();
    let store = Store::new(profile.0.clone()).unwrap();
    store.update(|state| {
        state.settings.read_only = true;
        Ok(())
    }).unwrap();
    drop(store);
    fs::remove_file(profile.0.join("store.lock")).unwrap();
    let original = fs::read(profile.0.join("state.json")).unwrap();
    assert!(Store::new_mobile(profile.0.clone(), false).is_err());
    assert!(!profile.0.join("store.lock").exists());
    assert!(!profile.0.join("mobile-root-v1").exists());
    assert_eq!(fs::read(profile.0.join("state.json")).unwrap(), original);
}

#[cfg(mobile)]
#[test]
fn mobile_readonly_relocation_rebases_visible_and_rollback_state_before_cutover() {
    let original = Profile::new();
    let restored = Profile::new();
    let store = Store::new_mobile(original.0.clone(), false).unwrap();
    let old_root = store.data_dir().to_path_buf();
    let archive = old_root.join("archives/book.zim");
    fs::create_dir_all(archive.parent().unwrap()).unwrap();
    fs::write(&archive, b"retained archive").unwrap();
    store.update(|state| {
        let mut book = fixture_book();
        book.path = archive.to_string_lossy().into_owned();
        book.managed = true;
        state.books.push(book);
        state.settings.download_directory = old_root.join("archives").to_string_lossy().into_owned();
        state.settings.private_mode = true;
        state.settings.read_only = true;
        Ok(())
    }).unwrap();
    drop(store);
    fs::rename(&original.0, &restored.0).unwrap();
    let state_path = restored.0.join("state.json");
    let before = fs::read(&state_path).unwrap();
    let marker_path = restored.0.join("mobile-root-v1/root.json");
    let marker_before = fs::read(&marker_path).unwrap();
    let store = Store::new_mobile(restored.0.clone(), false).unwrap();
    let new_root = store.data_dir().to_path_buf();
    let expected_archive = new_root.join("archives/book.zim").to_string_lossy().into_owned();
    assert_eq!(store.read().unwrap().books[0].path, expected_archive);
    assert_eq!(store.lock().unwrap().persisted().books[0].path, expected_archive);
    assert_eq!(fs::read(&state_path).unwrap(), before);
    assert_eq!(fs::read(&marker_path).unwrap(), marker_before);
    let mut view: serde_json::Value = serde_json::from_slice(&before).unwrap();
    assert!(store.mobile_rebase_saved_json(crate::mobile_rebase::RecordKind::State, &mut view).unwrap());
    assert_eq!(view["books"][0]["path"], expected_archive);
    store.update(|state| {
        state.settings.read_only = false;
        state.settings.private_mode = false;
        Ok(())
    }).unwrap();
    let durable: State = serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    assert_eq!(durable.books[0].path, expected_archive);
    assert_eq!(durable.settings.download_directory, new_root.join("archives").to_string_lossy());
    assert!(!durable.settings.read_only);
    assert_eq!(fs::read(new_root.join("archives/book.zim")).unwrap(), b"retained archive");
}

fn exported_history(id: &str, book_id: &str, path: &str, title: &str, visited_at: &str, scroll: f64) -> HistoryEntry {
    HistoryEntry {
        id: id.into(),
        book_id: book_id.into(),
        path: path.into(),
        title: title.into(),
        visited_at: visited_at.into(),
        scroll,
    }
}

#[test]
fn history_export_is_versioned_lossless_and_deterministically_ordered() {
    let state = State { history: vec![
        exported_history("id-old", "book-b", "A/Old", "Old", "2026-01-01T00:00:00+00:00", 0.0),
        // Ties use book ID, then path, then ID so one saved state has one export order.
        exported_history("id-b", "book-b", "A/Tie", "Tie B", "2026-02-01T00:00:00+00:00", 1.5),
        exported_history("id-a2", "book-a", "A/Tie?q=%41", "Tie A2", "2026-02-01T00:00:00+00:00", 12.25),
        exported_history("id-a1", "book-a", "A/Tie", "Tie A1", "2026-02-01T00:00:00+00:00", 2.0),
        exported_history("id-new", "book-a", "A/New%23?x=1#frag", "Nouveau 😀", "2026-03-01T00:00:00+00:00", 0.75),
        exported_history("id-tie-id", "book-a", "A/Tie", "Tie A1 duplicate path", "2026-02-01T00:00:00+00:00", 3.0),
    ], ..State::default() };
    let original = state.history.clone();

    let text = export_history(&state).unwrap();
    assert_eq!(text, export_history(&state).unwrap(), "repeated exports must be identical");
    assert_eq!(state.history.len(), original.len(), "export must not mutate history");
    for (saved, before) in state.history.iter().zip(&original) {
        assert_eq!(saved.id, before.id);
        assert_eq!(saved.visited_at, before.visited_at);
    }

    let document: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(document["version"], serde_json::json!(HISTORY_EXPORT_VERSION));
    let entries = document["history"].as_array().unwrap();
    let ids: Vec<&str> = entries.iter().map(|entry| entry["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["id-new", "id-a1", "id-tie-id", "id-a2", "id-b", "id-old"]);

    let newest = &entries[0];
    assert_eq!(newest["bookId"], "book-a");
    assert_eq!(newest["path"], "A/New%23?x=1#frag");
    assert_eq!(newest["title"], "Nouveau 😀");
    assert_eq!(newest["visitedAt"], "2026-03-01T00:00:00+00:00");
    assert_eq!(newest["scroll"], serde_json::json!(0.75));
    let scroll_of = |id: &str| {
        entries
            .iter()
            .find(|entry| entry["id"] == id)
            .expect("exported entry")["scroll"]
            .clone()
    };
    assert_eq!(scroll_of("id-a1"), serde_json::json!(2.0));
    assert_eq!(scroll_of("id-a2"), serde_json::json!(12.25));
    assert!(newest.get("legacyBook").is_none());
}

#[test]
fn empty_history_exports_an_empty_versioned_document() {
    let document: serde_json::Value =
        serde_json::from_str(&export_history(&State::default()).unwrap()).unwrap();
    assert_eq!(document["version"], serde_json::json!(HISTORY_EXPORT_VERSION));
    assert_eq!(document["history"], serde_json::json!([]));
}

#[test]
fn history_export_refuses_an_invalid_reading_position_instead_of_writing_null() {
    let state = State { history: vec![exported_history(
        "id-nan", "book", "A/Nan", "Nan", "2026-01-01T00:00:00+00:00", f64::NAN,
    )], ..State::default() };
    assert!(export_history(&state).is_err());
}
