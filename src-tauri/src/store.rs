// SPDX-License-Identifier: GPL-3.0-or-later
//! Versioned user state. Missing archives are retained; only explicit service commands remove books.
//! `store.lock` is held for this object's lifetime, including headless/read-only instances.
//! Never delete a lock file to recover a running instance: locks belong to the open file, not its name.
//! Private mode keeps browsing changes in memory, preserving (not erasing) the pre-private session.
//! Read-only mode here covers this store only; the service must also disable downloads/log/profile writes.
//! Mobile startup inspects the saved policy before creating anything and rebases restored
//! container paths under the process lock. Saved read-only policy remains user-toggleable.

use crate::archive::ArchiveMetadata;
use crate::fs_ext::FileExt;
use serde::{Deserialize, Serialize};
use serde::de::DeserializeOwned;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Read, Write};
#[cfg(desktop)]
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

pub use crate::migration::{
    export_bookmarks, export_bookmarks_xml, import_bookmarks, legacy_books, legacy_paths,
};

pub const STATE_VERSION: u32 = 2;
const MAX_STATE_BYTES: u64 = 128 * 1024 * 1024;

pub(crate) fn new_id() -> String {
    Uuid::new_v4().to_string()
}

/// App-owned authentication principal for one installed archive, never supplied by ZIM metadata.
pub fn new_installation_id() -> String {
    Uuid::new_v4().to_string()
}
pub(crate) fn now() -> String {
    crate::timestamps::now()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Book {
    #[serde(flatten)]
    pub metadata: ArchiveMetadata,
    /// Immutable for an installation's lifetime; explicit relinking creates a new principal.
    /// Empty is accepted only while decoding the version-1 profile migration.
    #[serde(default)]
    pub installation_id: String,
    pub path: String,
    /// Native-owned provider capability, never a filesystem-path exception.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_reference: Option<crate::provider_reference::ProviderReference>,
    pub added_at: String,
    pub available: bool,
    #[serde(default)]
    pub managed: bool,
    #[serde(default)]
    pub integrity: Option<crate::integrity::IntegrityRecord>,
}

impl Book {
    /// Shell payload for one library entry.
    ///
    /// `mainPath` is a libzim path, which may contain a literal `?` or `#`; the
    /// shell uses it as an archive-relative location. Encode it exactly once here
    /// so every consumer treats it as an opaque location instead of re-splitting
    /// it as a URL and opening the prefix article. The in-memory and persisted
    /// `main_path` stays raw for libzim lookups.
    pub fn shell_value(&self) -> serde_json::Value {
        let mut value = serde_json::to_value(self).expect("Book serializes to JSON");
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "mainPath".into(),
                serde_json::Value::String(crate::server::encode_path(&self.metadata.main_path)),
            );
        }
        value
    }
}

/// One library entry as the shell receives it.
///
/// The wire field `mainPath` is an encoded archive location, not a libzim path.
/// This wrapper is the only reader of that payload, and it exposes the value as a
/// location, so no caller can put it back into [`Book::metadata`] and encode it a
/// second time. [`Book::shell_value`] stays the single raw-to-location boundary.
#[cfg(any(not(mobile), test))]
pub struct ShellBook<'a> {
    value: &'a serde_json::Value,
}

#[cfg(any(not(mobile), test))]
impl<'a> ShellBook<'a> {
    pub fn new(value: &'a serde_json::Value) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or("Library entry in shell payload is not an object")?;
        for key in ["id", "title", "mainPath"] {
            if !object.get(key).is_some_and(serde_json::Value::is_string) {
                return Err(format!("Library entry in shell payload is missing {key}"));
            }
        }
        Ok(Self { value })
    }
    fn string(&self, key: &str) -> &'a str {
        self.value[key].as_str().unwrap_or_default()
    }
    pub fn id(&self) -> &'a str {
        self.string("id")
    }
    pub fn title(&self) -> &'a str {
        self.string("title")
    }
    /// Encoded archive location of the archive's main document.
    pub fn main_location(&self) -> &'a str {
        self.string("mainPath")
    }
    /// Launch location for this entry, using `path` when the caller chose one.
    pub fn tab_location(&self, path: Option<&str>) -> TabLocation {
        TabLocation {
            book_id: self.id().to_owned(),
            path: path.unwrap_or_else(|| self.main_location()).to_owned(),
            title: self.title().to_owned(),
            scroll: 0.0,
        }
    }
}

impl PartialEq for Book {
    fn eq(&self, other: &Self) -> bool {
        let a = &self.metadata;
        let b = &other.metadata;
        self.path == other.path
            && self.installation_id == other.installation_id
            && self.source_reference == other.source_reference
            && self.added_at == other.added_at
            && self.available == other.available
            && self.managed == other.managed
            && self.integrity == other.integrity
            && a.id == b.id
            && a.title == b.title
            && a.description == b.description
            && a.language == b.language
            && a.creator == b.creator
            && a.publisher == b.publisher
            && a.date == b.date
            && a.name == b.name
            && a.tags == b.tags
            && a.size == b.size
            && a.article_count == b.article_count
            && a.media_count == b.media_count
            && a.has_fulltext_index == b.has_fulltext_index
            && a.has_title_index == b.has_title_index
            && a.main_path == b.main_path
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct LegacyBookmarkBook {
    pub id: String,
    pub title: String,
    pub name: String,
    pub flavour: String,
    pub language: String,
    pub date: String,
}

/// A text quote anchored to UTF-16 offsets in one archive document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Highlight {
    pub id: String,
    pub document_path: String,
    pub exact: String,
    pub prefix: String,
    pub suffix: String,
    pub start: u32,
    pub end: u32,
    pub comment: String,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum HighlightChange {
    Add { highlight: Highlight },
    Comment { id: String, comment: String },
    Remove { id: String },
}

impl HighlightChange {
    pub(crate) fn requires_existing(&self) -> bool {
        !matches!(self, Self::Add { .. })
    }

    pub(crate) fn apply(self, highlights: &mut Vec<Highlight>) -> Result<(), String> {
        match self {
            Self::Add { highlight } => highlights.push(highlight),
            Self::Comment { id, comment } => {
                validate_highlight_id(&id)?;
                let highlight = highlights
                    .iter_mut()
                    .find(|highlight| highlight.id.eq_ignore_ascii_case(&id))
                    .ok_or("The highlight no longer exists")?;
                highlight.comment = comment;
            }
            Self::Remove { id } => {
                validate_highlight_id(&id)?;
                highlights.retain(|highlight| !highlight.id.eq_ignore_ascii_case(&id));
            }
        }
        validate_highlights(highlights)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Bookmark {
    #[serde(default = "new_id")]
    pub id: String,
    pub book_id: String,
    pub path: String,
    pub title: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub highlights: Vec<Highlight>,
    #[serde(default = "now")]
    pub created_at: String,
    /// Original libkiwix book descriptors survive imports even when its archive is missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_book: Option<LegacyBookmarkBook>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoryEntry {
    pub id: String,
    pub book_id: String,
    pub path: String,
    pub title: String,
    pub visited_at: String,
    pub scroll: f64,
}

/// Visit-history interchange version. Independent of the native profile schema.
pub const HISTORY_EXPORT_VERSION: u32 = 1;

#[derive(Serialize)]
struct BorrowedHistoryExport<'a> {
    version: u32,
    history: Vec<&'a HistoryEntry>,
}

/// Lossless visit-history export: every stored field, newest first, with
/// identity tiebreakers so repeated exports of the same state are identical.
/// Nothing is written to history and there is no matching import.
pub fn export_history(state: &State) -> Result<String, String> {
    for entry in &state.history {
        validate_location(&entry.book_id, &entry.path, entry.scroll)?;
    }
    let mut history: Vec<&HistoryEntry> = state.history.iter().collect();
    history.sort_by(|left, right| {
        right
            .visited_at
            .cmp(&left.visited_at)
            .then_with(|| left.book_id.cmp(&right.book_id))
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.id.cmp(&right.id))
    });
    serde_json::to_string_pretty(&BorrowedHistoryExport {
        version: HISTORY_EXPORT_VERSION,
        history,
    })
    .map_err(|error| format!("Cannot export history: {error}"))
}

/// Matches Collections::recentHistoryEntries: ignore literal fragments, decode
/// the pathname once, but preserve query text, case and invalid UTF-8 identity.
pub fn history_resource_identity(path: &str) -> (bool, std::borrow::Cow<'_, str>, &str) {
    let resource = path.split_once('#').map_or(path, |(resource, _)| resource);
    let (pathname, query) = resource
        .find('?')
        .map_or((resource, ""), |index| (&resource[..index], &resource[index..]));
    match percent_encoding::percent_decode_str(pathname).decode_utf8() {
        Ok(decoded) => (true, decoded, query),
        Err(_) => (false, std::borrow::Cow::Borrowed(pathname), query),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabLocation {
    pub book_id: String,
    pub path: String,
    pub title: String,
    pub scroll: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tab {
    pub id: String,
    pub book_id: Option<String>,
    pub path: String,
    pub title: String,
    pub scroll: f64,
    #[serde(default)]
    pub restricted: bool,
    #[serde(default)]
    pub back: Vec<TabLocation>,
    #[serde(default)]
    pub forward: Vec<TabLocation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeWindowTab {
    pub group: String,
    pub index: usize,
    pub selected: bool,
    pub focused: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowSession {
    pub tabs: Vec<Tab>,
    pub active_tab: Option<String>,
    pub geometry: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_tab: Option<NativeWindowTab>,
    #[serde(default = "default_active_surface")]
    pub active_surface: String,
}

fn default_active_surface() -> String { "library".into() }

pub(crate) fn validate_active_surface(surface: &str) -> Result<(), String> {
    if matches!(surface, "library" | "catalog" | "downloads" | "nearby" | "maps" | "bookmarks" | "history" | "settings" | "reader") {
        Ok(())
    } else {
        Err("Unknown window surface".into())
    }
}

impl Default for WindowSession {
    fn default() -> Self {
        Self { tabs: Vec::new(), active_tab: None, geometry: None, native_tab: None, active_surface: default_active_surface() }
    }
}

/// Durable "previous session" used only by the explicit restore command.
///
/// `state.json` remains the live session (and is rewritten by the running
/// shell), so a separate snapshot is what survives a relaunch that started
/// with startup restore disabled. It stores browsing locations only: window
/// labels, reader tabs, scroll positions, active surfaces and geometry. It is
/// never written in private or read-only mode.
#[cfg(any(not(mobile), test))]
pub const SESSION_SNAPSHOT_VERSION: u32 = 1;
#[cfg(any(not(mobile), test))]
pub const SESSION_SNAPSHOT_FILE: &str = "last-session.json";
#[cfg(any(not(mobile), test))]
const MAX_SESSION_BYTES: u64 = 8 * 1024 * 1024;

#[cfg(any(not(mobile), test))]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    pub version: u32,
    pub windows: HashMap<String, WindowSession>,
    #[serde(default)]
    pub reference_revision: u64,
}

#[cfg(any(not(mobile), test))]
impl SessionSnapshot {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != SESSION_SNAPSHOT_VERSION {
            return Err("Unsupported saved-session version".into());
        }
        if self.windows.len() > 16 {
            return Err("The saved session has too many windows".into());
        }
        if !self.windows.contains_key("main") {
            return Err("The saved session has no main window".into());
        }
        for (label, window) in &self.windows {
            if label != "main"
                && (!label.starts_with("window-")
                    || label.len() > 80
                    || label.chars().any(char::is_control))
            {
                return Err("The saved session names an unknown window".into());
            }
            validate_tabs(&window.tabs, window.active_tab.as_deref())?;
            validate_active_surface(&window.active_surface)?;
        }
        Ok(())
    }
    pub fn has_content(&self) -> bool {
        self.windows.values().any(|window| !window.tabs.is_empty())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct LibraryFilters {
    pub languages: Vec<String>,
    pub categories: Vec<String>,
    pub content: Vec<String>,
}
impl LibraryFilters {
    fn validate(&self) -> Result<(), String> {
        for choices in [&self.languages, &self.categories, &self.content] {
            if choices.len() > 256
                || choices.iter().any(|value| {
                    value.is_empty() || value.len() > 1024 || value.chars().any(char::is_control)
                })
            {
                return Err("Library filter selections are invalid or too large".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpeechPreference {
    pub voice: String,
    pub rate: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpeechPreferencePatch {
    pub voice: Option<String>,
    pub rate: Option<f64>,
}

// Only saved State accepts the retired global fields. IPC Settings does not.
fn deserialize_saved_settings<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Settings, D::Error> {
    let mut value = serde_json::Value::deserialize(deserializer)?;
    if let Some(object) = value.as_object_mut() {
        let rate = object.remove("ttsRate");
        let voice = object.remove("ttsVoice");
        if !object.contains_key("speechPreferences") && (rate.is_some() || voice.is_some()) {
            object.insert("speechPreferences".into(), serde_json::json!({
                "*": {"voice": voice.unwrap_or(serde_json::json!("")), "rate": rate.unwrap_or(serde_json::json!(1.0))}
            }));
        }
    }
    serde_json::from_value(value).map_err(serde::de::Error::custom)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct Settings {
    pub theme: String,
    pub locale: String,
    pub last_import_directory: Option<String>,
    pub last_save_directory: Option<String>,
    pub download_directory: String,
    pub monitor_directories: Vec<String>,
    /// Runtime projection only; never trust a client-supplied portability flag.
    #[serde(default, deserialize_with = "deserialize_runtime_portable_mode")]
    pub portable_mode: bool,
    pub restore_session: bool,
    /// Reader link actions open a new tab without leaving the current article.
    pub open_new_tab_in_background: bool,
    pub link_previews: bool,
    pub zoom: f64,
    pub book_zoom: HashMap<String, f64>,
    pub library_filters: LibraryFilters,
    pub catalog_filters: LibraryFilters,
    /// True once the user edits Discover filters; suppresses the device-language default.
    pub catalog_filters_customized: bool,
    pub trash_deleted_files: bool,
    pub spelling_enabled: bool,
    pub show_search_snippets: bool,
    /// Render recoverable technical failures as bands. Blocking failures
    /// always render; recoverable ones are recorded either way.
    pub show_technical_errors: bool,
    pub font_size: f64,
    pub font_family: String,
    /// Column width for article text: the archive's own layout, a readable
    /// measure, or the full window.
    pub reader_width: String,
    pub reader_theme: String,
    pub expand_sections: bool,
    pub catalog_url: String,
    pub map_catalog_url: String,
    pub map_proxy_url: String,
    pub external_links: String,
    pub history_enabled: bool,
    pub speech_preferences: HashMap<String, SpeechPreference>,
    pub private_mode: bool,
    pub read_only: bool,
    /// Bytes/second; zero means unlimited.
    pub download_limit: u64,
    pub upload_limit: u64,
    /// Apple-only Nearby preference: try the AWDL/peer-to-peer route before
    /// infrastructure. Off keeps the historical infrastructure-first order.
    pub direct_connection: bool,
    pub auto_resume_downloads: bool,
    /// Native completion notices are opt-in and require an explicit OS permission action.
    pub completion_notifications: bool,
    /// Mobile Internet acquisition must be admitted by native OS network capabilities.
    pub allow_metered_downloads: bool,
    /// iOS final app-owned archive leaves only; Android backup exclusion is static.
    pub backup_archives: bool,
    pub opds_sources: Vec<String>,
    pub sharing_book_ids: Vec<String>,
    /// Library archive IDs pinned to the top, in pin order. At most
    /// [`Settings::MAX_PINNED_BOOKS`]; stale IDs are pruned by the UI and by
    /// library removal.
    pub pinned_book_ids: Vec<String>,
    /// Optional public display hint for newly started Nearby shares.
    pub sharing_name: String,
    /// Opt-in local diagnostics; never permission for network telemetry.
    pub diagnostics_enabled: bool,
    /// Desktop agent connector (kiwix-mcp): local AI agents may search and
    /// read the library. Off by default; never active in private mode.
    pub agent_connector: bool,
    pub log_level: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            locale: "en".into(),
            last_import_directory: None,
            last_save_directory: None,
            download_directory: if cfg!(mobile) {
                String::new()
            } else if crate::sandbox::is_sandboxed() {
                crate::sandbox::real_downloads_dir()
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_default()
            } else {
                directories::UserDirs::new()
                    .and_then(|dirs| dirs.download_dir().map(|p| p.to_string_lossy().into_owned()))
                    .unwrap_or_default()
            },
            monitor_directories: Vec::new(),
            portable_mode: false,
            restore_session: true,
            open_new_tab_in_background: true,
            link_previews: true,
            zoom: 1.0,
            book_zoom: HashMap::new(),
            library_filters: LibraryFilters::default(),
            catalog_filters: LibraryFilters::default(),
            catalog_filters_customized: false,
            trash_deleted_files: cfg!(desktop),
            spelling_enabled: false,
            show_search_snippets: true,
            show_technical_errors: false,
            font_size: 18.0,
            font_family: "original".into(),
            reader_width: "original".into(),
            reader_theme: "original".into(),
            expand_sections: false,
            catalog_url: "https://library.kiwix.org/catalog/v2/entries".into(),
            map_catalog_url: crate::maps::DEFAULT_CATALOG_URL.into(),
            map_proxy_url: crate::maps::DEFAULT_PROXY_URL.into(),
            external_links: "ask".into(),
            history_enabled: true,
            speech_preferences: HashMap::new(),
            private_mode: false,
            read_only: false,
            download_limit: 0,
            upload_limit: 0,
            direct_connection: false,
            auto_resume_downloads: false,
            completion_notifications: false,
            allow_metered_downloads: false,
            backup_archives: false,
            opds_sources: Vec::new(),
            sharing_book_ids: Vec::new(),
            pinned_book_ids: Vec::new(),
            sharing_name: String::new(),
            diagnostics_enabled: false,
            agent_connector: false,
            log_level: "warn".into(),
        }
    }
}

impl Settings {
    /// The local library surfaces at most this many pinned archives.
    pub const MAX_PINNED_BOOKS: usize = 3;

    /// Move a pinned archive to its replacement identity without changing pin
    /// order. A replacement that is already pinned elsewhere keeps its own slot.
    pub fn remap_pinned_book(&mut self, old: &str, new: &str) {
        if old == new {
            return;
        }
        let Some(index) = self.pinned_book_ids.iter().position(|id| id == old) else {
            return;
        };
        if self.pinned_book_ids.iter().any(|id| id == new) {
            self.pinned_book_ids.remove(index);
        } else {
            self.pinned_book_ids[index] = new.to_string();
        }
    }

    pub fn update_speech_preference(&mut self, language: &str, patch: SpeechPreferencePatch) -> Result<(), String> {
        if language == "*" || (patch.voice.is_none() && patch.rate.is_none()) {
            return Err("A speech edit requires a language and an edited voice or speed".into());
        }
        let default = self.speech_preferences.get("*").cloned()
            .unwrap_or(SpeechPreference { voice: String::new(), rate: 1.0 });
        let preference = self.speech_preferences.entry(language.into()).or_insert(default);
        if let Some(voice) = patch.voice { preference.voice = voice; }
        if let Some(rate) = patch.rate { preference.rate = rate; }
        self.validate()
    }

    pub fn validate(&self) -> Result<(), String> {
        if !crate::publisher::valid_name(&self.sharing_name) {
            return Err("Sharing name must be at most 160 UTF-8 bytes, without surrounding whitespace, controls or direction overrides".into());
        }
        for (name, value, choices) in [
            (
                "theme",
                self.theme.as_str(),
                &["system", "light", "dark"][..],
            ),
            (
                "fontFamily",
                self.font_family.as_str(),
                &["original", "serif", "sans"][..],
            ),
            (
                "readerTheme",
                self.reader_theme.as_str(),
                &["original", "light", "dark", "sepia"][..],
            ),
            (
                "readerWidth",
                self.reader_width.as_str(),
                &["original", "comfortable", "full"][..],
            ),
            (
                "externalLinks",
                self.external_links.as_str(),
                &["ask", "deny"][..],
            ),
            (
                "logLevel",
                self.log_level.as_str(),
                &["off", "error", "warn", "info", "debug", "trace"][..],
            ),
        ] {
            if !choices.contains(&value) {
                return Err(format!("Invalid {name}: {value:?}"));
            }
        }
        for (name, value, min, max) in [
            ("zoom", self.zoom, 0.25, 5.0),
            ("fontSize", self.font_size, 8.0, 72.0),
        ] {
            if !value.is_finite() || !(min..=max).contains(&value) {
                return Err(format!(
                    "{name} must be a finite number from {min} to {max}"
                ));
            }
        }
        self.library_filters.validate()?;
        self.catalog_filters.validate()?;
        if self.pinned_book_ids.len() > Self::MAX_PINNED_BOOKS {
            return Err(format!(
                "Pin at most {} library archives at a time",
                Self::MAX_PINNED_BOOKS
            ));
        }
        for (index, id) in self.pinned_book_ids.iter().enumerate() {
            if id.is_empty() || id.len() > 1024 || id.chars().any(char::is_control) {
                return Err("Pinned archives require a nonempty archive identity".into());
            }
            if self.pinned_book_ids[..index].contains(id) {
                return Err("Pinned archives must be unique".into());
            }
        }
        if self.speech_preferences.len() > 1024 {
            return Err("Too many speech language preferences".into());
        }
        for (language, preference) in &self.speech_preferences {
            if language.is_empty() || language.len() > 128
                || (language != "*" && !language.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'))
                || preference.voice.len() > 4096 || preference.voice.chars().any(char::is_control)
                || !preference.rate.is_finite() || !(0.25..=2.0).contains(&preference.rate)
            {
                return Err("Speech preferences require a language, an installed voice identifier and a speed from 0.25 to 2".into());
            }
        }
        for (id, zoom) in &self.book_zoom {
            if id.is_empty()
                || id.chars().any(char::is_control)
                || !zoom.is_finite()
                || !(0.25..=5.0).contains(zoom)
            {
                return Err(
                    "Book zoom requires an archive identity and a value from 25% to 500%".into(),
                );
            }
        }
        if self.locale.is_empty()
            || self.locale.len() > 128
            || self.locale.chars().any(char::is_control)
        {
            return Err("locale must be a nonempty language tag of at most 128 bytes".into());
        }
        for value in [
            &self.catalog_url,
            &self.map_catalog_url,
            &self.map_proxy_url,
        ]
        .into_iter()
        .chain(self.opds_sources.iter())
        {
            let url = url::Url::parse(value)
                .map_err(|e| format!("Invalid catalog/proxy URL {value:?}: {e}"))?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Err(
                    "Catalog/proxy URLs must be HTTP(S) URLs without embedded credentials".into(),
                );
            }
        }
        for path in std::iter::once(&self.download_directory).chain(self.monitor_directories.iter())
        {
            if path.contains('\0') {
                return Err("Directory paths cannot contain NUL bytes".into());
            }
        }
        for path in self.last_import_directory.iter().chain(self.last_save_directory.iter()) {
            let bytes = path.as_bytes();
            let absolute = path.starts_with('/') || path.starts_with("\\\\")
                || (bytes.get(1) == Some(&b':') && matches!(bytes.get(2), Some(b'/') | Some(b'\\')));
            if path.len() > 32768 || path.contains('\0') || !absolute {
                return Err("Remembered picker folders must be absolute paths of at most 32768 bytes without NUL".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub version: u32,
    #[serde(default)]
    pub books: Vec<Book>,
    #[serde(default)]
    pub bookmarks: Vec<Bookmark>,
    #[serde(default)]
    pub history: Vec<HistoryEntry>,
    #[serde(default)]
    pub recent_searches: Vec<String>,
    #[serde(default)]
    pub tabs: Vec<Tab>,
    #[serde(default)]
    pub active_tab: Option<String>,
    #[serde(default)]
    pub windows: HashMap<String, WindowSession>,
    #[serde(default, deserialize_with = "deserialize_saved_settings")]
    pub settings: Settings,
    /// Keys are app-owned installation IDs, never archive-supplied UUIDs or loopback origins.
    #[serde(default)]
    pub reader_storage: HashMap<String, serde_json::Value>,
    /// Transport job IDs acknowledged after verified library import, not browsing state.
    #[serde(default)]
    pub consumed_acquisitions: HashSet<String>,
    #[serde(default)]
    pub edition_intents: Vec<crate::edition_updates::EditionIntent>,
    #[serde(default)]
    pub edition_transactions: Vec<crate::edition_updates::EditionTransaction>,
    #[serde(default)]
    pub edition_bindings: Vec<crate::edition_updates::EditionBinding>,
    #[serde(default)]
    pub reference_transitions: Vec<crate::edition_updates::ReferenceTransition>,
    #[serde(default)]
    pub reference_revision: u64,
    /// Portable causal reading data; never native trust or archive authority.
    #[serde(default)]
    pub reading_sync: crate::reading_data::ReadingSync,
    /// Native enrolled public identities and explicit device consent.
    #[serde(default)]
    pub(crate) private_sync: crate::private_sync::Settings,
}

impl Default for State {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            books: Vec::new(),
            bookmarks: Vec::new(),
            history: Vec::new(),
            recent_searches: Vec::new(),
            tabs: Vec::new(),
            active_tab: None,
            windows: HashMap::new(),
            settings: Settings::default(),
            reader_storage: HashMap::new(),
            consumed_acquisitions: HashSet::new(),
            edition_intents: Vec::new(),
            edition_transactions: Vec::new(),
            edition_bindings: Vec::new(),
            reference_transitions: Vec::new(),
            reference_revision: 0,
            reading_sync: crate::reading_data::ReadingSync::default(),
            private_sync: crate::private_sync::Settings::default(),
        }
    }
}

impl State {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != STATE_VERSION {
            return Err(format!("This profile was saved by a different version of Kiwix (version {}; this app reads version {}). Open it with that version, or choose another profile. Your saved file was not changed.", self.version, STATE_VERSION));
        }
        if self.recent_searches.len() > 100
            || self
                .recent_searches
                .iter()
                .any(|query| query.is_empty() || query.len() > 65536 || query.contains('\0'))
        {
            return Err("Recent searches exceed the supported history bounds".into());
        }
        self.settings.validate()?;
        crate::reading_data::validate(&self.reading_sync)?;
        self.private_sync.validate()?;
        unique_ids(self.books.iter().map(|b| b.metadata.id.as_str()), "book")?;
        unique_ids(
            self.books.iter().map(|book| book.installation_id.as_str()),
            "book installation",
        )?;
        unique_ids(self.bookmarks.iter().map(|b| b.id.as_str()), "bookmark")?;
        unique_ids(self.history.iter().map(|h| h.id.as_str()), "history entry")?;
        validate_tabs(&self.tabs, self.active_tab.as_deref())?;
        unique_ids(self.windows.keys().map(String::as_str), "window")?;
        for window in self.windows.values() {
            validate_tabs(&window.tabs, window.active_tab.as_deref())?;
            validate_active_surface(&window.active_surface)?;
        }
        for book in &self.books {
            let installation = Uuid::parse_str(&book.installation_id)
                .map_err(|_| "A saved library entry is damaged. Remove that archive from your library, then add it again".to_string())?;
            if installation.get_version_num() != 4
                || installation.to_string() != book.installation_id
            {
                return Err("A saved library entry is damaged. Remove that archive from your library, then add it again".into());
            }
            if let Some(source) = &book.source_reference { source.validate_book(book)?; }
            if book.path.is_empty() || book.path.contains('\0') {
                return Err("Book path must be nonempty and contain no NUL".into());
            }
        }
        for bookmark in &self.bookmarks {
            validate_bookmark(bookmark)?;
        }
        for entry in &self.history {
            validate_location(&entry.book_id, &entry.path, entry.scroll)?;
        }
        crate::edition_updates::validate_state(self)?;
        Ok(())
    }
}

/// Resolve a possibly stale archive reference to the current library entry.
/// Edition transitions remap reading references, so a delayed shell action can
/// still address the installed edition.
pub(crate) fn current_reading_reference(state: &State, id: &str) -> String {
    if state.books.iter().any(|book| book.metadata.id == id) {
        return id.to_owned();
    }
    let mut current = id.to_owned();
    for transition in &state.reference_transitions {
        if current == transition.from_book_id {
            current.clone_from(&transition.to_book_id);
        }
    }
    current
}

/// A saved record that cannot be decoded or validated is skipped, never
/// allowed to deny the whole profile. Warnings stay content-free: they name
/// the record class, not paths, titles, or identifiers.
fn note_skipped_record(warnings: &mut Vec<String>, kind: &str) {
    if warnings.len() < 32 {
        warnings.push(format!("A saved {kind} item could not be read and was skipped."));
    }
}

fn decode_records<T: DeserializeOwned>(
    value: Option<&serde_json::Value>,
    kind: &str,
    warnings: &mut Vec<String>,
    mut validate: impl FnMut(&T) -> Result<(), String>,
) -> Vec<T> {
    let Some(value) = value else {
        return Vec::new();
    };
    let Some(items) = value.as_array() else {
        note_skipped_record(warnings, kind);
        return Vec::new();
    };
    let mut kept = Vec::new();
    for item in items {
        // Deserialize straight from the borrowed value; only kept records are cloned.
        let Ok(record) = T::deserialize(item) else {
            note_skipped_record(warnings, kind);
            continue;
        };
        if validate(&record).is_err() {
            note_skipped_record(warnings, kind);
            continue;
        }
        kept.push(record);
    }
    kept
}

fn valid_window_label(label: &str) -> bool {
    label == "main"
        || (label.starts_with("window-")
            && label.len() <= 80
            && !label.chars().any(char::is_control))
}

fn validate_book_record(book: &Book, version: u32) -> Result<(), String> {
    if book.metadata.id.is_empty() || book.metadata.id.chars().any(char::is_control) {
        return Err("Book identity is invalid".into());
    }
    if version >= STATE_VERSION {
        let installation = Uuid::parse_str(&book.installation_id)
            .map_err(|_| "A saved library entry is damaged. Remove that archive from your library, then add it again".to_string())?;
        if installation.get_version_num() != 4
            || installation.to_string() != book.installation_id
        {
            return Err("A saved library entry is damaged. Remove that archive from your library, then add it again".into());
        }
    }
    if let Some(source) = &book.source_reference {
        source.validate_book(book)?;
    }
    if book.path.is_empty() || book.path.contains('\0') {
        return Err("Book path must be nonempty and contain no NUL".into());
    }
    Ok(())
}

fn validate_tab_record(tab: &Tab) -> Result<(), String> {
    if tab.id.is_empty() || tab.id.chars().any(char::is_control) {
        return Err("Tab identity is invalid".into());
    }
    validate_scroll(tab.scroll)?;
    if tab.path.contains('\0') {
        return Err("Tab paths cannot contain NUL".into());
    }
    if let Some(book_id) = &tab.book_id {
        validate_location(book_id, &tab.path, tab.scroll)?;
    }
    for location in tab.back.iter().chain(&tab.forward) {
        validate_location(&location.book_id, &location.path, location.scroll)?;
    }
    Ok(())
}

/// Decode `state.json` one record at a time. Global invariants (JSON shape,
/// version, settings) remain fatal; a single bad book, bookmark, history
/// entry, tab, or saved window is dropped and reported.
pub(crate) fn state_from_value(value: serde_json::Value) -> Result<(State, Vec<String>, bool), String> {
    let Some(object) = value.as_object() else {
        return Err("State file must be a JSON object".into());
    };
    let mut normalized = false;
    let version = object
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .ok_or("State file is missing its version")?;
    if version != 1 && version != STATE_VERSION as u64 {
        return Err(format!(
            "This profile was saved by a different version of Kiwix (version {version}; this app reads version {}). Open it with that version, or choose another profile. Your saved file was not changed.",
            STATE_VERSION
        ));
    }
    let version = version as u32;
    let mut warnings = Vec::new();
    // Keep the saved-settings decoder (including retired field migration) by
    // decoding through State instead of Settings directly.
    let settings = match object.get("settings") {
        Some(settings) => serde_json::from_value::<State>(serde_json::json!({
            "version": version,
            "settings": settings.clone(),
        }))
        .map_err(|error| format!("Invalid settings: {error}"))?
        .settings,
        None => Settings::default(),
    };
    settings.validate().map_err(|error| format!("Invalid settings: {error}"))?;
    let mut state = State {
        version,
        settings,
        ..State::default()
    };
    // Never salvage damaged causal/trust metadata record-by-record: dropping a
    // tombstone or reusing a counter could silently resurrect or overwrite data.
    if let Some(value) = object.get("readingSync") {
        state.reading_sync = serde_json::from_value(value.clone()).map_err(|error| format!("Invalid reading sync state: {error}"))?;
        crate::reading_data::validate(&state.reading_sync)?;
    }
    if let Some(value) = object.get("privateSync") {
        state.private_sync = serde_json::from_value(value.clone()).map_err(|error| format!("Invalid connected private devices: {error}"))?;
        state.private_sync.validate()?;
    }

    let mut book_ids = HashSet::new();
    let mut installation_ids = HashSet::new();
    state.books = decode_records::<Book>(object.get("books"), "book", &mut warnings, |book| {
        validate_book_record(book, version)?;
        if !book_ids.insert(book.metadata.id.clone()) {
            return Err("Duplicate book identity".into());
        }
        if version >= STATE_VERSION && !installation_ids.insert(book.installation_id.clone()) {
            return Err("Duplicate book installation".into());
        }
        Ok(())
    });

    let mut bookmark_ids = HashSet::new();
    state.bookmarks = decode_records::<Bookmark>(object.get("bookmarks"), "bookmark", &mut warnings, |bookmark| {
        validate_bookmark(bookmark)?;
        if !bookmark_ids.insert(bookmark.id.clone()) {
            return Err("Duplicate bookmark identity".into());
        }
        Ok(())
    });
    if crate::reading_data::has_bookmark_history(&state.reading_sync)
        && object.get("bookmarks").and_then(serde_json::Value::as_array)
            .is_none_or(|records| records.len() != state.bookmarks.len()) {
        return Err("The saved reading list is damaged. Its causal sync history was preserved; restore or repair this profile before syncing so recovery cannot become a deletion".into());
    }

    let mut history_ids = HashSet::new();
    state.history = decode_records::<HistoryEntry>(object.get("history"), "history entry", &mut warnings, |entry| {
        if entry.id.is_empty() || entry.id.chars().any(char::is_control) {
            return Err("History identity is invalid".into());
        }
        validate_location(&entry.book_id, &entry.path, entry.scroll)?;
        if !history_ids.insert(entry.id.clone()) {
            return Err("Duplicate history identity".into());
        }
        Ok(())
    });

    if let Some(value) = object.get("recentSearches") {
        if let Some(items) = value.as_array() {
            for item in items {
                match item.as_str() {
                    Some(query)
                        if !query.is_empty()
                            && query.len() <= 65536
                            && !query.contains('\0')
                            && (!query.contains('\u{1f}')
                                || query.split_once('\u{1f}').is_some_and(|(id, text)| {
                                    !text.is_empty()
                                        && !text.contains('\u{1f}')
                                        && book_ids.contains(id)
                                })) =>
                    {
                        state.recent_searches.push(query.to_string());
                    }
                    _ => note_skipped_record(&mut warnings, "recent search"),
                }
            }
            if state.recent_searches.len() > 100 {
                state.recent_searches.truncate(100);
                note_skipped_record(&mut warnings, "recent search");
            }
        } else {
            note_skipped_record(&mut warnings, "recent search");
        }
    }

    let mut tab_ids = HashSet::new();
    state.tabs = decode_records::<Tab>(object.get("tabs"), "tab", &mut warnings, |tab| {
        validate_tab_record(tab)?;
        if !tab_ids.insert(tab.id.clone()) {
            return Err("Duplicate tab identity".into());
        }
        Ok(())
    });
    state.active_tab = object
        .get("activeTab")
        .and_then(serde_json::Value::as_str)
        .and_then(|active| {
            state
                .tabs
                .iter()
                .any(|tab| tab.id == active)
                .then(|| active.to_string())
        });
    if object.get("activeTab").and_then(serde_json::Value::as_str).is_some()
        && state.active_tab.is_none()
    {
        note_skipped_record(&mut warnings, "active tab");
    }

    if let Some(value) = object.get("windows") {
        if let Some(windows) = value.as_object() {
            for (label, item) in windows {
                if !valid_window_label(label) {
                    note_skipped_record(&mut warnings, "window");
                    continue;
                }
                // Window records predate several fields. Decode each field with
                // its default instead of rejecting the whole record for an
                // unknown field, missing geometry, or an older surface name.
                let Some(item) = item.as_object() else {
                    note_skipped_record(&mut warnings, "window");
                    continue;
                };
                let mut ids = HashSet::new();
                let tabs = decode_records::<Tab>(item.get("tabs"), "tab", &mut warnings, |tab| {
                    validate_tab_record(tab)?;
                    if !ids.insert(tab.id.clone()) {
                        return Err("Duplicate tab identity".into());
                    }
                    Ok(())
                });
                let active_tab = item
                    .get("activeTab")
                    .and_then(serde_json::Value::as_str)
                    .filter(|active| tabs.iter().any(|tab| tab.id == *active))
                    .map(str::to_owned)
                    .or_else(|| tabs.first().map(|tab| tab.id.clone()));
                let geometry = item
                    .get("geometry")
                    .filter(|geometry| geometry.is_object() || geometry.is_null())
                    .cloned();
                let active_surface = item
                    .get("activeSurface")
                    .and_then(serde_json::Value::as_str)
                    .filter(|surface| validate_active_surface(surface).is_ok())
                    .unwrap_or("library")
                    .to_string();
                let session = WindowSession {
                    tabs,
                    active_tab,
                    geometry,
                    native_tab: item.get("nativeTab")
                        .and_then(|value| serde_json::from_value(value.clone()).ok()),
                    active_surface,
                };
                if serde_json::to_value(&session).ok()
                    != Some(serde_json::Value::Object(item.clone()))
                {
                    normalized = true;
                }
                state.windows.insert(label.clone(), session);
            }
        } else {
            note_skipped_record(&mut warnings, "window");
        }
    }

    if let Some(value) = object.get("readerStorage") {
        if let Some(storage) = value.as_object() {
            for (key, entry) in storage {
                if key.is_empty() || key.len() > 128 || key.chars().any(char::is_control) {
                    note_skipped_record(&mut warnings, "reader storage");
                } else {
                    state.reader_storage.insert(key.clone(), entry.clone());
                }
            }
        } else {
            note_skipped_record(&mut warnings, "reader storage");
        }
    }

    if let Some(value) = object.get("consumedAcquisitions") {
        if let Some(items) = value.as_array() {
            for item in items {
                match item.as_str() {
                    Some(id) if !id.is_empty() && !id.chars().any(char::is_control) => {
                        state.consumed_acquisitions.insert(id.to_string());
                    }
                    _ => note_skipped_record(&mut warnings, "acquisition receipt"),
                }
            }
        } else {
            note_skipped_record(&mut warnings, "acquisition receipt");
        }
    }

    // Dropping a malformed edition ownership record could let an ordinary
    // acquisition poll auto-register its output. Unlike browsing records these
    // security/transaction records therefore fail closed, preserving the file.
    if let Some(value) = object.get("editionIntents") {
        state.edition_intents = serde_json::from_value(value.clone())
            .map_err(|error| format!("Cannot decode saved edition intents: {error}"))?;
    }
    if let Some(value) = object.get("editionTransactions") {
        state.edition_transactions = serde_json::from_value(value.clone())
            .map_err(|error| format!("Cannot decode saved edition transactions: {error}"))?;
    }
    if let Some(value) = object.get("editionBindings") {
        state.edition_bindings = serde_json::from_value(value.clone())
            .map_err(|error| format!("Cannot decode saved catalog associations: {error}"))?;
    }
    if let Some(value) = object.get("referenceTransitions") {
        state.reference_transitions = serde_json::from_value(value.clone())
            .map_err(|error| format!("Cannot decode saved reference transitions: {error}"))?;
    }
    if let Some(value) = object.get("referenceRevision") {
        state.reference_revision = value.as_u64().ok_or("Invalid saved reference revision")?;
    }

    if state.version == 1 {
        migrate_installation_principals(&mut state)?;
    }
    state
        .validate()
        .map_err(|error| format!("Invalid state: {error}"))?;
    Ok((state, warnings.clone(), normalized && warnings.is_empty()))
}

fn validate_tabs(tabs: &[Tab], active_tab: Option<&str>) -> Result<(), String> {
    unique_ids(tabs.iter().map(|tab| tab.id.as_str()), "tab")?;
    for tab in tabs {
        validate_scroll(tab.scroll)?;
        if tab.path.contains('\0') {
            return Err("Tab paths cannot contain NUL".into());
        }
        if let Some(book_id) = &tab.book_id {
            validate_location(book_id, &tab.path, tab.scroll)?;
        }
        for location in tab.back.iter().chain(&tab.forward) {
            validate_location(&location.book_id, &location.path, location.scroll)?;
        }
    }
    if let Some(active) = active_tab {
        if !tabs.iter().any(|tab| tab.id == active) {
            return Err("activeTab must identify an existing tab in its window".into());
        }
    }
    Ok(())
}

fn unique_ids<'a>(values: impl Iterator<Item = &'a str>, kind: &str) -> Result<(), String> {
    let mut seen = HashSet::new();
    for id in values {
        if id.is_empty() || id.chars().any(char::is_control) || !seen.insert(id) {
            return Err(format!("Invalid or duplicate {kind} ID {id:?}"));
        }
    }
    Ok(())
}
fn validate_scroll(scroll: f64) -> Result<(), String> {
    if !scroll.is_finite() || scroll < 0.0 {
        return Err("Scroll position must be a finite nonnegative number".into());
    }
    Ok(())
}
fn validate_location(book_id: &str, path: &str, scroll: f64) -> Result<(), String> {
    if book_id.is_empty() || book_id.chars().any(char::is_control) || path.contains('\0') {
        return Err("Article identity requires a nonempty book ID and a path without NUL".into());
    }
    validate_scroll(scroll)
}
fn validate_highlight_id(id: &str) -> Result<uuid::Uuid, String> {
    if id.len() != 36 {
        return Err("A saved highlight is damaged. Remove the highlight, then add it again".into());
    }
    uuid::Uuid::parse_str(id).map_err(|_| "A saved highlight is damaged. Remove the highlight, then add it again".into())
}

pub(crate) fn validate_highlights(highlights: &[Highlight]) -> Result<(), String> {
    if highlights.len() > 200 {
        return Err("A bookmark may contain at most 200 highlights".into());
    }
    let mut ids = HashSet::new();
    for highlight in highlights {
        let id = validate_highlight_id(&highlight.id)?;
        if !ids.insert(id) {
            return Err("A bookmark contains the same highlight twice. Remove one and save again".into());
        }
        let exact_len = highlight.exact.encode_utf16().count();
        if exact_len == 0
            || exact_len > 16_384
            || highlight.prefix.encode_utf16().count() > 128
            || highlight.suffix.encode_utf16().count() > 128
            || highlight.comment.encode_utf16().count() > 16_384
        {
            return Err("Highlight text exceeds its allowed bounds".into());
        }
        if highlight.start > 100_000_000
            || highlight.end > 100_000_000
            || highlight.end <= highlight.start
            || (highlight.end - highlight.start) as usize != exact_len
        {
            return Err("Highlight offsets must match the UTF-16 quote length".into());
        }
        let (pathname, _) = crate::server::split_location(&highlight.document_path);
        if highlight.document_path.is_empty()
            || highlight.document_path.len() > 16_384
            || highlight.document_path.chars().any(char::is_control)
            || pathname.is_empty()
            || pathname.starts_with('/')
            || pathname
                .split('/')
                .next()
                .is_some_and(|part| part.contains(':'))
        {
            return Err(
                "Highlight documentPath must be a bounded archive-relative location".into(),
            );
        }
    }
    Ok(())
}

pub(crate) fn validate_bookmark(bookmark: &Bookmark) -> Result<(), String> {
    validate_location(&bookmark.book_id, &bookmark.path, 0.0)?;
    validate_highlights(&bookmark.highlights)?;
    if bookmark.id.is_empty() || bookmark.id.chars().any(char::is_control) {
        return Err("Bookmark ID must be nonempty and contain no control characters".into());
    }
    if crate::timestamps::parse(&bookmark.created_at).is_err() {
        return Err(format!(
            "Invalid bookmark createdAt {:?}; expected RFC3339",
            bookmark.created_at
        ));
    }
    Ok(())
}

struct Inner {
    state: State,
    /// Last durable snapshot. `None` while it is identical to `state` (the normal
    /// non-private steady state), so an ordinary update keeps a single copy.
    persisted: Option<State>,
    /// Never included in an update candidate or any persisted State.
    provider_books: Vec<Book>,
    /// Shell retirements ordered with native capture and session persistence.
    /// This avoids acquiring the service's window-operation lock from AppKit.
    #[cfg(desktop)]
    retired_window_captures: HashSet<String>,
    runtime_private: bool,
    runtime_read_only: bool,
    durability_error: Option<String>,
    /// A browsing-telemetry delta is applied in memory but not yet written.
    /// `persisted` then holds the last durable content, so private rollback and
    /// no-op detection keep comparing against the file.
    browsing_pending: bool,
    /// When the profile was last written, for the coalescing window.
    last_persist: Option<Instant>,
    /// The profile loaded read-only with skipped records, so no copy of the
    /// original was kept; the commit that leaves read-only keeps it first.
    skipped_unpreserved: bool,
    /// The skipped-records notice as it stands now: the load notice, until
    /// leaving read-only mode keeps the copy it asked the person to make.
    notice: Option<String>,
    #[cfg(mobile)]
    mobile_relocation: Option<crate::mobile_rebase::Relocation>,
}

impl Inner {
    /// The last durable state, whether or not it is shared with `state`.
    fn persisted(&self) -> &State {
        self.persisted.as_ref().unwrap_or(&self.state)
    }

    /// Mutate the live state without persisting it. The durable snapshot is
    /// materialized first, so private-mode rollback and no-op detection still
    /// observe the last written browsing state.
    fn diverge(&mut self) -> &mut State {
        if self.persisted.is_none() {
            self.persisted = Some(self.state.clone());
        }
        &mut self.state
    }
}

/// Browsing telemetry is written at most once per this window. A crash can
/// therefore lose at most this much scroll position; a new history entry, a
/// bookmark or any settings edit still writes through immediately, and the
/// next transaction of any kind persists a pending delta with the rest.
const BROWSING_COALESCE_INTERVAL: Duration = Duration::from_secs(5);

/// Whether a transaction must be durable before it returns.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Persist {
    /// Explicit user edits, navigation and lifecycle captures.
    Durable,
    /// Repeatable browsing telemetry (reading-position refreshes).
    Coalesced,
}

pub struct Store {
    data_dir: PathBuf,
    // Retain the descriptor, never unlink the lock file while this descriptor is held.
    _lock_file: File,
    inner: Mutex<Inner>,
    /// Records skipped while decoding the saved profile, content-free, for
    /// Troubleshooting. The load itself does not rewrite the file, but the
    /// next ordinary write saves the profile without them; a writable profile
    /// therefore keeps a copy of the original bytes first (`load_notice`).
    load_warnings: Vec<String>,
    /// The one startup notice for a load that skipped records: where the
    /// original bytes were kept, or why they could not be.
    load_notice: Option<String>,
    #[cfg(desktop)]
    portable_paths: Option<crate::portable::PortablePaths>,
    #[cfg(mobile)]
    mobile_profile: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AcquisitionPathRoot {
    Relative,
    Unix,
    Drive(u8),
    Unc,
}

/// Purely lexical: publication capabilities already resolved filesystem aliases before
/// taking the store lock. Fold case conservatively even on case-sensitive volumes.
fn acquisition_path_root(path: &str) -> (AcquisitionPathRoot, &str, bool) {
    let separator = |byte| byte == b'/' || byte == b'\\';
    let bytes = path.as_bytes();
    // Only the exact backslash question-mark prefix disables Win32
    // normalization. Dot-device and slash variants still name drive/UNC roots.
    let verbatim = bytes.starts_with(br"\\?\");
    let device = bytes.len() >= 4
        && separator(bytes[0])
        && separator(bytes[1])
        && matches!(bytes[2], b'?' | b'.')
        && separator(bytes[3]);
    let path = if device {
        let path = &path[4..];
        if path.len() >= 4
            && path.as_bytes()[..3].eq_ignore_ascii_case(b"UNC")
            && separator(path.as_bytes()[3])
        {
            return (AcquisitionPathRoot::Unc, &path[4..], verbatim);
        }
        path
    } else {
        path
    };
    let bytes = path.as_bytes();
    if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && separator(bytes[2])
    {
        return (
            AcquisitionPathRoot::Drive(bytes[0].to_ascii_lowercase()),
            &path[3..],
            verbatim,
        );
    }
    if bytes.len() >= 2
        && separator(bytes[0])
        && separator(bytes[1])
        && (cfg!(windows) || bytes[0] == b'\\' || bytes[1] == b'\\')
    {
        (AcquisitionPathRoot::Unc, &path[2..], verbatim)
    } else if bytes.first().copied().is_some_and(separator) {
        (AcquisitionPathRoot::Unix, &path[1..], verbatim)
    } else {
        (AcquisitionPathRoot::Relative, path, verbatim)
    }
}

fn normalize_acquisition_component(component: &str, trim: bool, final_component: bool) -> &str {
    if !trim || matches!(component, "." | "..") {
        component
    } else if final_component {
        component.trim_end_matches(['.', ' '])
    } else if !component.ends_with("..") {
        component.strip_suffix('.').unwrap_or(component)
    } else {
        component
    }
}

fn acquisition_path_components<'a>(
    tail: &'a str,
    base: &'a str,
    trim_tail: bool,
    trim_base: bool,
) -> impl Iterator<Item = &'a str> {
    // Win32 removes one final dot from intermediate components, but preserves
    // their spaces and multi-dot suffixes. Only the complete path's final
    // component loses every trailing dot/space; verbatim paths lose neither.
    let mut parents = 0usize;
    tail.rsplit(['/', '\\'])
        .enumerate()
        .map(move |(index, component)| normalize_acquisition_component(component, trim_tail, index == 0))
        .chain(
            base.rsplit(['/', '\\'])
                .map(move |component| normalize_acquisition_component(component, trim_base, false)),
        )
        .filter(move |component| match *component {
            "" | "." => false,
            ".." => {
                parents = parents.saturating_add(1);
                false
            }
            _ if parents != 0 => {
                parents -= 1;
                false
            }
            _ => true,
        })
}

pub(crate) fn acquisition_paths_equal(left: &str, right: &str, working_directory: &str) -> bool {
    let (base_root, base_tail, base_verbatim) = acquisition_path_root(working_directory);
    let (left_root, left_tail, left_verbatim) = acquisition_path_root(left);
    let (right_root, right_tail, right_verbatim) = acquisition_path_root(right);
    let (left_root, left_base) = if left_root == AcquisitionPathRoot::Relative {
        (base_root, base_tail)
    } else {
        (left_root, "")
    };
    let (right_root, right_base) = if right_root == AcquisitionPathRoot::Relative {
        (base_root, base_tail)
    } else {
        (right_root, "")
    };
    if left_root != right_root {
        return false;
    }
    let win32 = |root, path: &str| {
        cfg!(windows)
            || matches!(
                root,
                AcquisitionPathRoot::Drive(_) | AcquisitionPathRoot::Unc
            )
            || path.contains('\\')
    };
    let trim_base = !base_verbatim && win32(base_root, working_directory);
    let left = acquisition_path_components(
        left_tail,
        left_base,
        !left_verbatim && win32(left_root, left),
        trim_base,
    );
    let right = acquisition_path_components(
        right_tail,
        right_base,
        !right_verbatim && win32(right_root, right),
        trim_base,
    );
    acquisition_components_equal(left, right)
}

fn acquisition_components_equal<'a, 'b>(
    mut left: impl Iterator<Item = &'a str>,
    mut right: impl Iterator<Item = &'b str>,
) -> bool {
    loop {
        match (left.next(), right.next()) {
            (None, None) => return true,
            (Some(a), Some(b)) if a == b || folded_nfd_eq(a, b) => {}
            _ => return false,
        }
    }
}

/// NFD, full case fold, NFD: the alias model for one path component.
fn folded_nfd_eq(left: &str, right: &str) -> bool {
    // ASCII is NFD-stable and folds to its lowercase, so this is exact for the
    // common case. A non-ASCII side can still equal an ASCII one (`ß`, Kelvin
    // sign, U+1DF95), so other pairs take the full path below. It allocates
    // per component and cannot exit early: about 1 µs, ~10x the old lazy
    // comparison, or about 1 ms per check at 1,000 non-ASCII book paths,
    // which is fine for these user-triggered checks.
    if left.is_ascii() && right.is_ascii() {
        return left.eq_ignore_ascii_case(right);
    }
    let (left, right): (String, String) = (left.nfd().collect(), right.nfd().collect());
    crate::casefold::fold(&left).nfd().eq(crate::casefold::fold(&right).nfd())
}

/// Compare validated absolute destinations without allocating a joined path.
pub(crate) fn acquisition_path_parts_equal(left: &str, directory: &str, names: &[&str]) -> bool {
    let (left_root, left_tail, left_verbatim) = acquisition_path_root(left);
    let (right_root, right_tail, right_verbatim) = acquisition_path_root(directory);
    if left_root != right_root || left_root == AcquisitionPathRoot::Relative ||
        names.iter().any(|name| acquisition_path_root(name).0 != AcquisitionPathRoot::Relative) {
        return false;
    }
    let win32 = |root, path: &str| cfg!(windows) ||
        matches!(root, AcquisitionPathRoot::Drive(_) | AcquisitionPathRoot::Unc) || path.contains('\\');
    let left = acquisition_path_components(left_tail, "", !left_verbatim && win32(left_root, left), false);
    let trim = !right_verbatim && (win32(right_root, directory) || names.iter().any(|name| name.contains('\\')));
    let mut parents = 0usize;
    let right = names.iter().rev().flat_map(|name| name.rsplit(['/', '\\']))
        .chain(right_tail.rsplit(['/', '\\']))
        .enumerate()
        .map(move |(index, component)| normalize_acquisition_component(component, trim, index == 0))
        .filter(move |component| match *component {
            "" | "." => false,
            ".." => { parents = parents.saturating_add(1); false }
            _ if parents != 0 => { parents -= 1; false }
            _ => true,
        });
    acquisition_components_equal(left, right)
}

fn deserialize_runtime_portable_mode<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    let _ = bool::deserialize(deserializer)?;
    Ok(false)
}

#[cfg(desktop)]
fn apply_portable_settings(directory: &Path, settings: &mut Settings) -> Result<(), String> {
    let root = directory.to_str().ok_or("Portable data path is not Unicode")?;
    settings.download_directory = root.to_owned();
    settings.monitor_directories = vec![root.to_owned()];
    Ok(())
}

/// Why the profile's process lock could not be retained.
///
/// Callers that run before a native window exists use this to tell "another
/// live instance owns the profile" (recoverable: activate it or explain the
/// conflict) from "this profile cannot be opened at all" (a startup failure).
#[cfg(any(not(mobile), test, all(feature = "android-evaluation", target_os = "android", debug_assertions)))]
#[derive(Debug)]
pub(crate) enum ProfileLockError {
    Directory { path: PathBuf, source: std::io::Error },
    Open { path: PathBuf, source: std::io::Error },
    /// The lock call itself failed for a reason other than "already held".
    /// Treating this as `Busy` would tell the user another instance owns the
    /// profile and exit successfully when the profile simply cannot be locked.
    Lock { path: PathBuf, source: std::io::Error },
    Busy { path: PathBuf, source: std::io::Error },
}

#[cfg(any(not(mobile), test, all(feature = "android-evaluation", target_os = "android", debug_assertions)))]
impl std::fmt::Display for ProfileLockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Directory { path, source } => {
                write!(f, "Cannot create state directory {}: {source}", path.display())
            }
            Self::Open { path, source } => write!(
                f,
                "Cannot open process lock {}: {source}. Read-only startup requires an existing initialized profile.",
                path.display()
            ),
            Self::Lock { path, source } => write!(
                f,
                "Cannot lock {}: {source}. The profile filesystem may not support advisory file locks (for example an NFS/SMB data directory); choose a profile on local storage.",
                path.display()
            ),
            Self::Busy { path, source } => write!(
                f,
                "Cannot exclusively lock {}: {source}. Another native/headless instance may be using this profile; close it or select a different data directory. Do not delete its lock file.",
                path.display()
            ),
        }
    }
}

/// Why a desktop startup could not open the profile.
///
/// Only `Busy` is recoverable: it means another live process owns the profile,
/// which the launch path answers by activating the owner or explaining the
/// conflict, never by aborting. Keeping it typed is what lets startup tell that
/// case apart from a real open failure; collapsing it into a string is what
/// turned the original second-instance launch into `Failed to setup app` and
/// SIGABRT.
#[cfg(desktop)]
#[derive(Debug)]
pub(crate) enum StoreOpenError {
    /// Another live process holds `store.lock`.
    Busy(ProfileLockError),
    /// This profile cannot be opened at all.
    Fatal(String),
}

#[cfg(desktop)]
impl std::fmt::Display for StoreOpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy(error) => write!(f, "{error}"),
            Self::Fatal(error) => f.write_str(error),
        }
    }
}

#[cfg(desktop)]
impl From<ProfileLockError> for StoreOpenError {
    fn from(error: ProfileLockError) -> Self {
        match error {
            ProfileLockError::Busy { .. } => Self::Busy(error),
            other => Self::Fatal(other.to_string()),
        }
    }
}

/// Take the profile's process lock before anything else touches the profile.
///
/// The returned descriptor is the ownership token: `Store` adopts it instead of
/// opening a second descriptor, so a second instance is decided before Tauri
/// creates a window and can never race between a probe and setup. `read_only`
/// mirrors strict read-only startup: never create the directory or lock file.
#[cfg(any(not(mobile), test, all(feature = "android-evaluation", target_os = "android", debug_assertions)))]
pub(crate) fn claim_profile_lock(
    data_dir: &Path,
    read_only: bool,
) -> Result<File, ProfileLockError> {
    if !read_only {
        crate::fs_ext::create_private_dir_all(data_dir).map_err(|source| ProfileLockError::Directory {
            path: data_dir.to_path_buf(),
            source,
        })?;
    }
    let lock_path = data_dir.join("store.lock");
    let mut options = OpenOptions::new();
    options.read(true);
    if !read_only {
        options.write(true).create(true);
    }
    let file = options
        .open(&lock_path)
        .map_err(|source| ProfileLockError::Open {
            path: lock_path.clone(),
            source,
        })?;
    FileExt::try_flock_exclusive(&file).map_err(|source| lock_failure(&lock_path, source))?;
    Ok(file)
}

/// A held lock is the only recoverable outcome. Everything else (a filesystem
/// without advisory locks, an I/O error, a bad descriptor) is a startup
/// failure whose real cause must be reported instead of "another instance".
#[cfg(any(not(mobile), test, all(feature = "android-evaluation", target_os = "android", debug_assertions)))]
fn lock_failure(lock_path: &Path, source: std::io::Error) -> ProfileLockError {
    let path = lock_path.to_path_buf();
    if crate::fs_ext::is_lock_contention(&source) {
        ProfileLockError::Busy { path, source }
    } else {
        ProfileLockError::Lock { path, source }
    }
}

/// Legacy profiles may quarantine corruption for local recovery. Profiles that
/// contain ownership, causal or trust metadata instead fail closed: replacing
/// them could forget deletion history or native device authorization.
#[cfg(any(mobile, test))]
fn load_mobile_state(
    data_dir: &Path,
    path: &Path,
    read_only: bool,
    relocation: Option<&crate::mobile_rebase::Relocation>,
) -> Result<(State, Vec<String>, Option<String>), String> {
    // If bytes cannot be inspected, absence of causal/ownership metadata
    // cannot be established. Never replace an unreadable owned profile.
    let bytes = state_bytes(path)?;
    let Some(bytes) = bytes else {
        if read_only {
            return Err(format!("Cannot open {}: read-only startup requires an existing initialized profile", path.display()));
        }
        let state = State::default();
        persist(data_dir, &state, false).map_err(|e| e.message)?;
        return Ok((state, Vec::new(), None));
    };
    // Establish protection before parsing/relocation, including malformed JSON
    // carrying sync keys. False positives fail closed rather than lose history.
    let mut has_owned_security = [b"\"readingSync\"".as_slice(), b"\"privateSync\"".as_slice()]
        .iter().any(|key| bytes.windows(key.len()).any(|window| window == *key));
    // A profile another schema version saved is not damage: quarantining it
    // would start this older (or newer) app empty and leave the user's
    // library behind under another name. Refuse it unchanged instead.
    let mut foreign_version = false;
    let decoded = (|| -> Result<(State, Vec<String>, bool), String> {
        let parse_error = |error| format!("Cannot parse {}: {error}", path.display());
        let mut value: serde_json::Value = serde_json::from_slice(&bytes).map_err(parse_error)?;
        if let Some(error) = foreign_version_error(&value) {
            foreign_version = true;
            return Err(error);
        }
        has_owned_security |= value.get("readingSync").is_some() || value.get("privateSync").is_some()
            || ["editionIntents", "editionTransactions", "editionBindings", "referenceTransitions"]
                .iter().any(|key| value.get(*key).is_some_and(|records|
                    records.as_array().is_none_or(|records| !records.is_empty())))
            || value.get("referenceRevision").is_some_and(|revision| revision.as_u64() != Some(0));
        if let Some(relocation) = relocation {
            relocation.rebase_json(crate::mobile_rebase::RecordKind::State, &mut value)?;
        }
        let migrated = value.get("version").and_then(serde_json::Value::as_u64) == Some(1);
        let (state, warnings, normalized) = state_from_value(value)
            .map_err(|error| format!("Invalid {}: {error}", path.display()))?;
        Ok((state, warnings, migrated || normalized))
    })();
    match decoded {
        Ok((state, warnings, write)) => {
            let writable = !read_only && !state.settings.read_only;
            let notice = (!warnings.is_empty()).then(|| skipped_records_notice(data_dir, &bytes, writable));
            if write && !read_only {
                persist(data_dir, &state, false).map_err(|e| e.message)?;
            }
            Ok((state, warnings, notice))
        }
        Err(error) if has_owned_security || foreign_version => Err(error),
        Err(error) if !read_only => {
            eprintln!("Kiwix: {error}");
            let kept = quarantine_state(data_dir)?;
            eprintln!("Kiwix: kept the unreadable state as {}", kept.display());
            let state = State::default();
            persist(data_dir, &state, false).map_err(|e| e.message)?;
            Ok((state, vec![crate::mobile_rebase::STATE_QUARANTINED.to_owned()], None))
        }
        Err(error) => Err(error),
    }
}

/// A profile saved under another schema version, which this app must leave
/// unchanged. `None` for versions 1 and 2, and when no version is readable.
#[cfg(any(mobile, test))]
fn foreign_version_error(value: &serde_json::Value) -> Option<String> {
    let version = value.get("version").and_then(serde_json::Value::as_u64)?;
    (version != 1 && version != u64::from(STATE_VERSION)).then(|| format!(
        "Kiwix cannot open your saved library because a different version of Kiwix saved it (profile version {version}; this version reads version {STATE_VERSION}). Install the latest version of Kiwix to open it. Your saved library was not changed."
    ))
}

/// Preserve an unreadable state.json under a new name in the same directory.
/// The original bytes are never rewritten, truncated, or deleted.
#[cfg(any(mobile, test))]
fn quarantine_state(data_dir: &Path) -> Result<PathBuf, String> {
    let source = data_dir.join("state.json");
    let kept = data_dir.join(format!("state.unreadable-{}.json", Uuid::new_v4()));
    fs::rename(&source, &kept)
        .map_err(|error| format!("Cannot preserve unreadable {}: {error}", source.display()))?;
    Ok(kept)
}

/// The startup notice for a profile that loaded with skipped records. A
/// writable profile keeps the original bytes first: the load does not rewrite
/// `state.json`, but the next ordinary save would replace it without them.
/// Failing to keep the copy never stops the profile from opening.
fn skipped_records_notice(directory: &Path, bytes: &[u8], writable: bool) -> String {
    const SKIPPED: &str = "Some saved items could not be read by this version of Kiwix and are not shown.";
    let original = crate::storage::display_path(&directory.join("state.json"));
    if !writable {
        return format!("{SKIPPED} The profile is read-only, so they were left in {original} unchanged. Back up that file before you leave read-only mode: the next save does not include them.");
    }
    match preserve_skipped_original(directory, bytes) {
        Ok(kept) => kept_copy_notice(&kept),
        Err(error) => format!(
            "{SKIPPED} Kiwix could not keep a copy of the saved profile ({error}). Back up {original} now: the next save does not include them."
        ),
    }
}

/// The notice once the original of a profile with skipped records is kept.
fn kept_copy_notice(kept: &Path) -> String {
    format!(
        "Some saved items could not be read by this version of Kiwix and are not shown. A copy of the saved profile that still contains them was kept at {}. Putting that copy in place of state.json lets a version of Kiwix that can read them show them again.",
        crate::storage::display_path(kept)
    )
}

/// Keep `bytes` beside `state.json` under a name derived from their content,
/// so relaunching before the next save reuses the copy instead of adding one.
/// An existing file is never replaced or followed; one with other content
/// (an interrupted earlier copy) makes this write a uniquely named copy.
fn preserve_skipped_original(directory: &Path, bytes: &[u8]) -> Result<PathBuf, String> {
    use sha2::Digest;
    let digest = hex::encode(sha2::Sha256::digest(bytes));
    let named = directory.join(format!("state.skipped-{}.json", &digest[..16]));
    match write_new_private_file(directory, &named, bytes) {
        Ok(()) => return Ok(named),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let same = fs::symlink_metadata(&named)
                .is_ok_and(|metadata| metadata.file_type().is_file() && metadata.len() == bytes.len() as u64)
                && fs::read(&named).is_ok_and(|existing| existing == bytes);
            if same {
                return Ok(named);
            }
        }
        Err(error) => return Err(error.to_string()),
    }
    let unique = directory.join(format!("state.skipped-{}.json", Uuid::new_v4()));
    write_new_private_file(directory, &unique, bytes)
        .map(|()| unique)
        .map_err(|error| error.to_string())
}

/// Create `path` (never an existing file or link), owner-only, with `bytes`
/// synchronized to disk, then synchronize its directory where the platform
/// allows it. A partial file is removed. A directory synchronization failure
/// keeps the complete copy and is not reported as a missing one.
fn write_new_private_file(directory: &Path, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    #[cfg(test)]
    if FAIL_NEXT_PRESERVED_COPY.with(|fail| fail.replace(false)) {
        return Err(std::io::Error::other("Injected preserved-copy failure"));
    }
    #[cfg(not(windows))]
    let parent = File::open(directory)?;
    #[cfg(windows)]
    let _ = directory;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    if let Err(error) = written {
        let _ = fs::remove_file(path);
        return Err(error);
    }
    #[cfg(not(windows))]
    if let Err(error) = parent.sync_all() {
        eprintln!("Kiwix: the copy of the saved profile may not survive a power loss: {error}");
    }
    Ok(())
}

/// Reset only the saved window/tab layout, keeping the library, bookmarks,
/// history, and settings intact. Used by the native startup-failure screen when
/// a restored layout keeps the shell from starting. Returns an error while
/// another process holds the profile or the profile is saved read-only.
#[cfg(any(mobile, test))]
pub(crate) fn reset_saved_windows(data_dir: &Path) -> Result<(), String> {
    // Saved read-only mode forbids every profile write, including this one;
    // refuse before creating or locking the profile.
    #[cfg(mobile)]
    let saved_read_only = saved_read_only(data_dir)?;
    #[cfg(desktop)]
    let saved_read_only = saved_desktop_read_only(data_dir)?;
    if saved_read_only {
        return Err("Read-only mode is enabled; the saved window layout was not reset".into());
    }
    let lock_path = data_dir.join("store.lock");
    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|error| format!("Cannot open process lock {}: {error}", lock_path.display()))?;
    FileExt::try_flock_exclusive(&lock_file)
        .map_err(|error| format!("Another Kiwix instance is using this profile: {error}"))?;
    let path = data_dir.join("state.json");
    let Some(bytes) = state_bytes(&path)? else { return Ok(()) };
    let mut value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Saved settings are unreadable and cannot be edited: {error}"))?;
    // Rewriting another version's document through this one would drop or
    // reorder what that version saved, and opening it would still fail.
    if let Some(error) = foreign_version_error(&value) {
        return Err(error);
    }
    let object = value.as_object_mut().ok_or("Saved settings are not a JSON object")?;
    object.insert("windows".into(), serde_json::Value::Object(Default::default()));
    object.insert("tabs".into(), serde_json::Value::Array(Vec::new()));
    object.insert("activeTab".into(), serde_json::Value::Null);
    write_state_value(data_dir, &value)
}

/// Replace `state.json` atomically with an edited JSON document, matching the
/// durability rules of [`persist`] without re-decoding it through `State`.
#[cfg(any(mobile, test))]
fn write_state_value(directory: &Path, value: &serde_json::Value) -> Result<(), String> {
    let destination = directory.join("state.json");
    let temporary = directory.join(format!(".state-{}.tmp", Uuid::new_v4()));
    let failure = |error: std::io::Error| format!("Cannot save {}: {error}. The previous state was retained.", destination.display());
    #[cfg(not(windows))]
    let parent = File::open(directory).map_err(failure)?;
    let outcome = (|| -> Result<(), String> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut writer = BufWriter::new(options.open(&temporary).map_err(failure)?);
        serde_json::to_writer_pretty(&mut writer, value).map_err(|error| format!("Cannot serialize state: {error}; previous state retained"))?;
        writer.write_all(b"\n").map_err(failure)?;
        writer.flush().map_err(failure)?;
        if writer.get_ref().metadata().map_err(failure)?.len() > MAX_STATE_BYTES {
            return Err("State exceeds 128 MiB; transaction was not committed".into());
        }
        writer.get_ref().sync_all().map_err(failure)?;
        drop(writer);
        replace_file(&temporary, &destination).map_err(failure)?;
        #[cfg(not(windows))]
        parent.sync_all().map_err(failure)?;
        Ok(())
    })();
    if outcome.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    outcome
}

impl Store {
    #[cfg(test)]
    pub fn new(data_dir: PathBuf) -> Result<Self, String> {
        Self::new_with_mode(data_dir, false)
    }

    /// Strict runtime read-only startup requires an existing directory, lock file and state file.
    /// It neither creates nor repairs any file, even if initialization fails.
    ///
    /// Startup paths claim the profile before opening it (`new_with_mode_checked`
    /// or `new_with_claimed_lock`); only tests and the Android evaluation
    /// harness open a profile directly.
    #[cfg(any(
        test,
        all(feature = "android-evaluation", target_os = "android", debug_assertions)
    ))]
    pub fn new_with_mode(data_dir: PathBuf, read_only: bool) -> Result<Self, String> {
        Self::open_profile(data_dir, read_only,
            #[cfg(desktop)]
            crate::portable::PortableProfile::detect()?,
            #[cfg(desktop)]
            None,
            None,
        )
    }

    /// Desktop startup that has not claimed the profile yet. The claim happens
    /// here, before anything is written, and its outcome stays typed so the
    /// caller can activate the owner instead of failing startup.
    #[cfg(desktop)]
    pub(crate) fn new_with_mode_checked(
        data_dir: PathBuf,
        read_only: bool,
        preloaded: Option<Vec<u8>>,
    ) -> Result<Self, StoreOpenError> {
        let portable = crate::portable::PortableProfile::detect().map_err(StoreOpenError::Fatal)?;
        // A saved read-only policy is a lock constraint too: claiming
        // read-write would create a lock file the profile forbids.
        let lock_read_only = match &portable {
            Some(profile) => {
                read_only || profile.saved_read_only().map_err(StoreOpenError::Fatal)?
            }
            None => read_only || saved_desktop_read_only(&data_dir).map_err(StoreOpenError::Fatal)?,
        };
        let lock = claim_profile_lock(&data_dir, lock_read_only)?;
        Self::open_profile(data_dir, read_only, portable, Some(lock), preloaded)
            .map_err(StoreOpenError::Fatal)
    }

    /// Desktop startup that already claimed the profile lock before any window
    /// existed. The store adopts that descriptor and never locks a second one.
    /// The error stays typed so a busy outcome can never reach setup as an
    /// opaque string that aborts the process.
    #[cfg(all(desktop, unix))]
    pub(crate) fn new_with_claimed_lock(
        data_dir: PathBuf,
        read_only: bool,
        portable: Option<crate::portable::PortableProfile>,
        lock_file: File,
        preloaded: Option<Vec<u8>>,
    ) -> Result<Self, StoreOpenError> {
        Self::open_profile(data_dir, read_only, portable, Some(lock_file), preloaded)
            .map_err(StoreOpenError::Fatal)
    }

    #[cfg(any(not(mobile), test, all(feature = "android-evaluation", target_os = "android", debug_assertions)))]
    fn open_profile(
        data_dir: PathBuf,
        read_only: bool,
        #[cfg(desktop)] portable: Option<crate::portable::PortableProfile>,
        #[cfg(desktop)] lock_file: Option<File>,
        preloaded: Option<Vec<u8>>,
    ) -> Result<Self, String> {
        #[cfg(desktop)]
        let saved_read_only = if let Some(profile) = &portable {
            if !profile.matches(&data_dir)? {
                return Err("Portable mode requires the adjacent data directory; conflicting --data-dir is not permitted".into());
            }
            let saved = profile.saved_read_only()?;
            profile.prepare(read_only || saved)?;
            saved
        } else {
            false
        };
        #[cfg(desktop)]
        let initialization_read_only = read_only || saved_read_only;
        #[cfg(not(desktop))]
        let initialization_read_only = read_only;
        // Desktop startup passes the descriptor it claimed before Tauri created
        // any window; that claim is the authority, so it is never re-locked here.
        #[cfg(desktop)]
        let lock_file = match lock_file {
            Some(lock_file) => lock_file,
            None => claim_profile_lock(&data_dir, initialization_read_only)
                .map_err(|error| error.to_string())?,
        };
        #[cfg(not(desktop))]
        let lock_file = claim_profile_lock(&data_dir, initialization_read_only)
            .map_err(|error| error.to_string())?;
        // Resolve the trusted profile root only after retaining its process
        // lock. Descendant no-follow checks must not mistake platform aliases
        // such as macOS /tmp for a caller-supplied child symlink.
        let data_dir = fs::canonicalize(&data_dir)
            .map_err(|error| format!("Cannot resolve state directory {}: {error}", data_dir.display()))?;
        #[cfg(desktop)]
        let portable_paths = portable.as_ref().map(|profile| {
            crate::portable::PortablePaths::initialize(profile, initialization_read_only)
        }).transpose()?;
        #[cfg(desktop)]
        let portable_mode = portable_paths.is_some();
        #[cfg(not(desktop))]
        let portable_mode = false;
        let path = data_dir.join("state.json");
        let decode = |bytes: &[u8]| -> Result<(State, Vec<String>, Option<String>), String> {
            let decode_error = |error| format!("Cannot parse {}: {error}. The original is untouched; restore a known-good backup or select another profile, not an empty replacement.", path.display());
            let value: serde_json::Value = serde_json::from_slice(bytes).map_err(decode_error)?;
            #[cfg(desktop)]
            let mut value = value;
            #[cfg(desktop)]
            if let Some(paths) = &portable_paths {
                paths.rebase_json(crate::profile_paths::RecordKind::State, &mut value)?;
            }
            let migrated = value.get("version").and_then(serde_json::Value::as_u64) == Some(1);
            let (state, warnings, normalized) = state_from_value(value)
                .map_err(|error| format!("Invalid {}: {error}", path.display()))?;
            #[cfg(desktop)]
            let mut state = state;
            #[cfg(desktop)]
            if portable_paths.is_some() {
                apply_portable_settings(&data_dir, &mut state.settings)?;
                state
                    .validate()
                    .map_err(|error| format!("Invalid {}: {error}", path.display()))?;
            }
            let writable = !initialization_read_only && !state.settings.read_only;
            // Before anything can save over the skipped records, including the
            // migration write just below.
            let notice = (!warnings.is_empty()).then(|| skipped_records_notice(&data_dir, bytes, writable));
            if (migrated || normalized) && writable {
                persist(&data_dir, &state, portable_mode).map_err(|e| e.message)?;
            }
            Ok((state, warnings, notice))
        };
        // Desktop startup already inspected these bytes for the saved read-only
        // policy and the first-paint theme, so the profile is read once.
        let (state, load_warnings, load_notice) = match preloaded {
            Some(bytes) => decode(&bytes)?,
            None => match File::open(&path) {
                Ok(file) => {
                    let mut bytes = Vec::new();
                    file.take(MAX_STATE_BYTES + 1)
                        .read_to_end(&mut bytes)
                        .map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
                    if bytes.len() as u64 > MAX_STATE_BYTES {
                        return Err(format!("{} exceeds the 128 MiB state limit; preserve the file and export/reduce it with a compatible application", path.display()));
                    }
                    decode(&bytes)?
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound && !initialization_read_only => {
                    let state = State::default();
                    #[cfg(desktop)]
                    let mut state = state;
                    #[cfg(desktop)]
                    if portable_paths.is_some() { apply_portable_settings(&data_dir, &mut state.settings)?; }
                    persist(&data_dir, &state, portable_mode).map_err(|e| e.message)?;
                    (state, Vec::new(), None)
                }
                Err(e) => return Err(format!("Cannot open {}: {e}", path.display())),
            },
        };
        // A read-only load kept no copy of what it skipped (see `update_inner`).
        let skipped_unpreserved = !load_warnings.is_empty() && (initialization_read_only || state.settings.read_only);
        let notice = load_notice.clone();
        let persisted = state.settings.private_mode.then(|| state.clone());
        let mut visible = state;
        if persisted.is_some() {
            clear_browsing(&mut visible);
        }
        Ok(Self {
            data_dir,
            _lock_file: lock_file,
            load_warnings,
            load_notice,
            #[cfg(desktop)]
            portable_paths,
            #[cfg(mobile)]
            mobile_profile: false,
            inner: Mutex::new(Inner {
                state: visible,
                persisted,
                provider_books: Vec::new(),
                #[cfg(desktop)]
                retired_window_captures: HashSet::new(),
                runtime_private: false,
                runtime_read_only: read_only,
                durability_error: None,
                browsing_pending: false,
                last_persist: None,
                skipped_unpreserved,
                notice,
                #[cfg(mobile)]
                mobile_relocation: None,
            }),
        })
    }

    /// Unlike a CLI override, a saved read-only preference must not permanently lock
    /// the policy toggle. Inspect it before even creating the profile or its lock.
    #[cfg(mobile)]
    pub fn new_mobile(data_dir: PathBuf, runtime_read_only: bool) -> Result<Self, String> {
        let read_only = runtime_read_only || saved_read_only(&data_dir)?;
        if !read_only {
            crate::fs_ext::create_private_dir_all(&data_dir).map_err(|e| {
                format!("Cannot create state directory {}: {e}", data_dir.display())
            })?;
        }
        let lock_path = data_dir.join("store.lock");
        let mut options = OpenOptions::new();
        options.read(true);
        if !read_only {
            options.write(true).create(true);
        }
        let lock_file = options.open(&lock_path).map_err(|e| format!("Cannot open process lock {}: {e}. Read-only startup requires an existing initialized profile.", lock_path.display()))?;
        FileExt::try_flock_exclusive(&lock_file).map_err(|e| format!("Cannot exclusively lock {}: {e}. Another native/headless instance may be using this profile; close it or select a different data directory. Do not delete its lock file.", lock_path.display()))?;
        // Canonicalization is read-only and occurs only after retaining the lock. In
        // particular, Apple /var aliases must agree with the engine's trusted root.
        let data_dir = fs::canonicalize(&data_dir)
            .map_err(|e| format!("Cannot resolve state directory {}: {e}", data_dir.display()))?;
        let read_only = runtime_read_only || saved_read_only(&data_dir)?;
        let mut initialized = crate::mobile_rebase::initialize(&data_dir, read_only)?;
        let relocation = initialized.relocation.take();
        let path = data_dir.join("state.json");
        let (state, mut load_warnings, load_notice) =
            load_mobile_state(&data_dir, &path, read_only, relocation.as_ref())?;
        // A read-only load kept no copy of what it skipped (see `update_inner`).
        let skipped_unpreserved = !load_warnings.is_empty() && (read_only || state.settings.read_only);
        let notice = load_notice.clone();
        load_warnings.extend(initialized.notices);
        // Both views use the current root: private-mode rollback must never restore
        // old-container paths even though the read-only disk state stays untouched.
        let persisted = state.settings.private_mode.then(|| state.clone());
        let mut visible = state;
        if persisted.is_some() {
            clear_browsing(&mut visible);
        }
        Ok(Self {
            data_dir,
            _lock_file: lock_file,
            load_warnings,
            load_notice,
            mobile_profile: true,
            inner: Mutex::new(Inner {
                state: visible,
                persisted,
                provider_books: Vec::new(),
                runtime_private: false,
                runtime_read_only,
                durability_error: None,
                browsing_pending: false,
                last_persist: None,
                skipped_unpreserved,
                notice,
                mobile_relocation: relocation,
            }),
        })
    }

    #[cfg(mobile)]
    fn mobile_complete_relocation_locked(&self, inner: &mut Inner) -> Result<(), String> {
        if !self.mobile_profile {
            return Ok(());
        }
        if inner.runtime_read_only {
            return Err("This profile was opened read-only; state changes are disabled".into());
        }
        if let Some(error) = &inner.durability_error {
            return Err(error.clone());
        }
        // Startup already rebased both in-memory views. Only the durable records
        // and root marker remain to be committed at this explicit policy boundary.
        let initialized = crate::mobile_rebase::initialize(&self.data_dir, false)?;
        if !initialized.durable {
            return Err("The profile relocation is incomplete. Keep the profile writable and try again after the remaining files are repaired.".into());
        }
        inner.mobile_relocation = None;
        Ok(())
    }

    /// Apply the same trusted, memory-only view to saved transfer records before
    /// read-only managers inspect them. This never repairs or saves the source file.
    #[cfg(mobile)]
    pub fn mobile_rebase_saved_json(
        &self,
        kind: crate::mobile_rebase::RecordKind,
        value: &mut serde_json::Value,
    ) -> Result<bool, String> {
        let inner = self.lock()?;
        match &inner.mobile_relocation {
            Some(relocation) => relocation.rebase_json(kind, value),
            None => Ok(false),
        }
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn portable_mode(&self) -> bool {
        #[cfg(desktop)]
        { self.portable_paths.is_some() }
        #[cfg(not(desktop))]
        { false }
    }

    #[cfg(desktop)]
    pub fn portable_rebase_saved_json(
        &self,
        kind: crate::profile_paths::RecordKind,
        value: &mut serde_json::Value,
    ) -> Result<bool, String> {
        match &self.portable_paths {
            Some(paths) => paths.rebase_json(kind, value),
            None => Ok(false),
        }
    }

    pub fn validate_portable_settings(&self, settings: &Settings) -> Result<(), String> {
        if self.portable_mode() {
            let root = self.data_dir.to_str().ok_or("Portable data path is not Unicode")?;
            if settings.download_directory != root || settings.monitor_directories != [root] {
                return Err("Portable mode fixes download and monitored folders to the adjacent data directory".into());
            }
        }
        Ok(())
    }

    #[cfg(any(not(mobile), test))]
    pub fn portable_data_dir(&self) -> Option<&Path> {
        self.portable_mode().then_some(&self.data_dir)
    }

    /// Automatic acquisitions cannot override the portable download root through
    /// command arguments, independently of the Settings API.
    pub fn validate_portable_directory(&self, directory: &Path) -> Result<(), String> {
        #[cfg(desktop)]
        if self.portable_mode() {
            let profile = crate::portable::PortableProfile { data: self.data_dir.clone() };
            if !profile.matches(directory)? {
                return Err("Portable downloads must use the adjacent data directory".into());
            }
            profile.prepare(true)?;
            self.acquisition_profile_directory()?;
        }
        #[cfg(not(desktop))]
        let _ = directory;
        Ok(())
    }

    /// Admit a profile capability only against the process lock we already own.
    /// Canonical aliases are allowed; a replacement profile is never adopted.
    pub(crate) fn acquisition_profile_directory(
        &self,
    ) -> Result<(PathBuf, cap_std::fs::Dir), String> {
        use cap_fs_ext::{FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
        let path = fs::canonicalize(&self.data_dir).map_err(|e| e.to_string())?;
        let directory = cap_std::fs::Dir::open_ambient_dir(&path, cap_std::ambient_authority())
            .map_err(|e| e.to_string())?;
        let retained = cap_std::fs::Metadata::from_file(&self._lock_file).map_err(|e| e.to_string())?;
        let mut options = cap_std::fs::OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let opened = directory.open_with("store.lock", &options).map_err(|e| e.to_string())?;
        for current in [
            directory.symlink_metadata("store.lock").map_err(|e| e.to_string())?,
            opened.metadata().map_err(|e| e.to_string())?,
        ] {
            if !current.is_file() || current.is_symlink() || current.nlink() != 1
                || current.dev() != retained.dev() || current.ino() != retained.ino()
            {
                return Err("Profile no longer identifies this application's retained process lock".into());
            }
        }
        Ok((path, directory))
    }

    fn lock(&self) -> Result<MutexGuard<'_, Inner>, String> {
        self.inner.lock().map_err(|_| "State transaction panicked; restart the app before making further changes. The last durable state remains on disk.".into())
    }

    pub fn read(&self) -> Result<State, String> {
        let inner = self.lock()?;
        let mut state = inner.state.clone();
        state.books.extend(inner.provider_books.iter().cloned());
        state.settings.private_mode |= inner.runtime_private;
        state.settings.read_only |= inner.runtime_read_only;
        state.settings.portable_mode = self.portable_mode();
        Ok(state)
    }

    /// Clone only the book records, in `read` order (library, then session
    /// providers), so listing paths never clone bookmarks, history or settings.
    pub fn books(&self) -> Result<Vec<Book>, String> {
        let inner = self.lock()?;
        let mut books = inner.state.books.clone();
        books.extend(inner.provider_books.iter().cloned());
        Ok(books)
    }

    /// Whether the live library still holds the same owner record as `expected`.
    /// Compared in place so streaming checks never clone a `Book`.
    pub fn owner_matches(&self, expected: &Book) -> Result<bool, String> {
        let inner = self.lock()?;
        Ok(inner.state.books.iter().chain(inner.provider_books.iter()).any(|current|
            current.metadata.id == expected.metadata.id && current.available
                && current.installation_id == expected.installation_id
                && current.path == expected.path
                && current.source_reference == expected.source_reference))
    }

    /// Content-free notices for records dropped while opening the profile.
    pub fn load_warnings(&self) -> &[String] {
        &self.load_warnings
    }

    /// The startup notice for records skipped while opening the profile,
    /// naming the preserved copy of the original. `None` when nothing was skipped.
    pub(crate) fn load_notice(&self) -> Option<&str> {
        self.load_notice.as_deref()
    }

    /// The skipped-records notice as it stands now. It differs from
    /// `load_notice` once leaving read-only mode has kept the copy that the
    /// read-only notice asked the person to make, and then names that copy.
    pub(crate) fn current_load_notice(&self) -> Result<Option<String>, String> {
        Ok(self.lock()?.notice.clone())
    }

    /// Scalar reading-sync consent, without cloning the sync document.
    pub(crate) fn reading_sync_enabled(&self) -> Result<bool, String> {
        Ok(self.lock()?.state.reading_sync.enabled)
    }

    /// Highest local causal counter, without cloning the sync document.
    pub(crate) fn reading_sync_counter(&self) -> Result<u64, String> {
        Ok(self.lock()?.state.reading_sync.counter)
    }

    /// Listening port saved for private device sync.
    pub(crate) fn private_sync_listen_port(&self) -> Result<u16, String> {
        Ok(self.lock()?.state.private_sync.listen_port)
    }

    /// One connected private device by ID. `enabled` remains the caller's policy.
    pub(crate) fn private_sync_peer(&self, id: &str) -> Result<Option<crate::private_sync::Peer>, String> {
        let inner = self.lock()?;
        Ok(inner.state.private_sync.peers.iter().find(|peer| peer.id == id).cloned())
    }

    pub(crate) fn check_private_peer_capacity(&self, id: &str) -> Result<(), String> {
        self.lock()?.state.private_sync.check_capacity(id)
    }

    /// Devices marked for automatic reconciliation; callers still apply `enabled`.
    pub(crate) fn private_sync_automatic_peers(&self) -> Result<Vec<crate::private_sync::Peer>, String> {
        let inner = self.lock()?;
        Ok(inner.state.private_sync.peers.iter().filter(|peer| peer.automatic).cloned().collect())
    }

    pub fn settings(&self) -> Result<Settings, String> {
        let inner = self.lock()?;
        let mut settings = inner.state.settings.clone();
        settings.private_mode |= inner.runtime_private;
        settings.read_only |= inner.runtime_read_only;
        settings.portable_mode = self.portable_mode();
        Ok(settings)
    }

    /// Atomically add or remove one persisted library pin. The caller supplies
    /// only intent, never a whole pinned array, so a second window or a rapid
    /// second click cannot overwrite a pin saved from a stale snapshot.
    pub fn set_book_pinned(&self, id: &str, pinned: bool) -> Result<(bool, Vec<String>), String> {
        self.update(|state| {
            let id = current_reading_reference(state, id);
            if !state.books.iter().any(|book| book.metadata.id == id) {
                return Err("Archive is not in the library".into());
            }
            let before = state.settings.pinned_book_ids.clone();
            // A pinned record skipped while loading the profile has no row to
            // unpin in the UI. Drop such ids before the cap check so they can
            // never permanently consume one of the three slots.
            let available: std::collections::HashSet<&str> = state
                .books
                .iter()
                .map(|book| book.metadata.id.as_str())
                .collect();
            state
                .settings
                .pinned_book_ids
                .retain(|value| available.contains(value.as_str()));
            if pinned {
                if !state.settings.pinned_book_ids.iter().any(|value| value == &id) {
                    if state.settings.pinned_book_ids.len() >= Settings::MAX_PINNED_BOOKS {
                        return Err(format!(
                            "Pin at most {} library archives at a time",
                            Settings::MAX_PINNED_BOOKS
                        ));
                    }
                    state.settings.pinned_book_ids.push(id);
                }
            } else {
                state.settings.pinned_book_ids.retain(|value| value != &id);
            }
            let after = state.settings.pinned_book_ids.clone();
            Ok((after != before, after))
        })
    }

    /// Snapshot one installation's storage and its write policy under one lock.
    pub(crate) fn reader_storage_snapshot(
        &self,
        installation_id: &str,
    ) -> Result<(serde_json::Value, bool), String> {
        let inner = self.lock()?;
        let values = inner.state.reader_storage.get(installation_id)
            .cloned().unwrap_or_else(|| serde_json::json!({}));
        Ok((values, inner.state.settings.read_only || inner.runtime_read_only))
    }

    pub fn read_only(&self) -> Result<bool, String> {
        let inner = self.lock()?;
        Ok(inner.state.settings.read_only || inner.runtime_read_only)
    }

    /// Admit filesystem side effects only while profile mutations remain possible.
    /// A durability failure stays latched until restart, independently of read-only mode.
    pub(crate) fn require_writable(&self) -> Result<(), String> {
        let inner = self.lock()?;
        if inner.state.settings.read_only || inner.runtime_read_only {
            return Err("The application is in strict read-only mode".into());
        }
        if let Some(error) = &inner.durability_error {
            return Err(error.clone());
        }
        Ok(())
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn retire_window_capture(&self, label: &str) -> Result<(), String> {
        self.lock()?.retired_window_captures.insert(label.to_owned());
        Ok(())
    }

    /// Persist the last session for the explicit restore command. `open` limits
    /// the snapshot to windows that were actually open, so an entry left behind
    /// by an earlier run cannot reappear. Private and read-only profiles write
    /// nothing; the previous snapshot is retained untouched.
    #[cfg(desktop)]
    pub fn save_session_snapshot(&self, open: &HashSet<String>) -> Result<bool, String> {
        let inner = self.lock()?;
        if inner.runtime_private
            || inner.runtime_read_only
            || inner.state.settings.private_mode
            || inner.state.settings.read_only
        {
            return Ok(false);
        }
        let mut windows = HashMap::new();
        for label in open.difference(&inner.retired_window_captures) {
            windows.insert(
                label.clone(),
                inner.state.windows.get(label).cloned().unwrap_or_default(),
            );
        }
        let main_open = windows.contains_key("main");
        let main = windows.entry("main".into()).or_default();
        // Keep an empty `main` entry because validation requires one, but a
        // closed main window must not inherit the live tab list: otherwise the
        // window the user closed reappears with stale tabs on restore.
        if main_open && main.tabs.is_empty() {
            main.tabs.clone_from(&inner.state.tabs);
            main.active_tab.clone_from(&inner.state.active_tab);
        }
        let snapshot = SessionSnapshot { version: SESSION_SNAPSHOT_VERSION, windows, reference_revision: inner.state.reference_revision };
        snapshot.validate()?;
        // An empty live session must not erase the last session worth
        // restoring; startup restore keeps using state.json instead.
        if !snapshot.has_content() {
            return Ok(false);
        }
        persist_session(&self.data_dir, &snapshot)?;
        Ok(true)
    }

    /// The last saved session, if the explicit restore command has one. The
    /// caller re-checks private/read-only policy before applying it.
    #[cfg(desktop)]
    pub fn saved_session(&self) -> Result<Option<SessionSnapshot>, String> {
        let path = self.data_dir.join(SESSION_SNAPSHOT_FILE);
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("Cannot open {}: {error}", path.display())),
        };
        if file.metadata().map_err(|e| e.to_string())?.len() > MAX_SESSION_BYTES {
            return Err("The saved session exceeds the supported size".into());
        }
        let mut snapshot: SessionSnapshot =
            serde_json::from_reader(BufReader::new(file).take(MAX_SESSION_BYTES + 1))
                .map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
        snapshot.validate()?;
        let state = self.read()?;
        for window in snapshot.windows.values_mut() {
            state.translate_reference_tabs(&mut window.tabs, snapshot.reference_revision)?;
        }
        snapshot.reference_revision = state.reference_revision;
        Ok(Some(snapshot))
    }

    /// Serialize cache filesystem access with effective mode transitions.
    /// The closure must not call locking Store methods or perform network I/O.
    pub(crate) fn with_catalog_cache_policy<T>(
        &self,
        work: impl FnOnce(bool, bool) -> Result<T, String>,
    ) -> Result<T, String> {
        let inner = self.lock()?;
        work(
            inner.state.settings.private_mode || inner.runtime_private,
            inner.state.settings.read_only || inner.runtime_read_only,
        )
    }

    /// Commit a prepared acquisition only while its destination is not reserved by any
    /// installed book, including unavailable books. Metadata/UUID equality is irrelevant.
    ///
    /// `work` runs with the library mutation mutex held. It must be a short synchronous
    /// verified filesystem commit: never call Store methods, await, perform network I/O,
    /// or do archive verification/canonicalization here. Explicit library moves/relinks
    /// use their own transactions and intentionally do not use this acquisition guard.
    pub fn with_acquisition_destination<T>(
        &self,
        path: &Path,
        work: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        self.with_acquisition_destinations(&[path], work)
    }

    /// Reserve an entire acquisition set under the same library mutation lock.
    pub fn with_acquisition_destinations<P: AsRef<Path>, T>(
        &self,
        paths: &[P],
        work: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        self.with_acquisition_destinations_available(paths, work)?.map_err(|path| format!(
            "Acquisition destination {} is reserved by an installed library book or retained archive edition; choose another destination",
            path.display()
        ))
    }

    /// A reserved library name is a collision, not a filesystem failure. Callers
    /// selecting another name must retry this guard, never bypass its mutex.
    pub(crate) fn with_acquisition_destinations_available<P: AsRef<Path>, T>(
        &self,
        paths: &[P],
        work: impl FnOnce() -> Result<T, String>,
    ) -> Result<Result<T, PathBuf>, String> {
        self.with_acquisition_destinations_available_for_http(paths, None, work)
    }

    /// Full HTTP editions use the normal downloader. Only the exact
    /// native-selected artifact URL may publish into its durable reservation;
    /// this grants no import/cutover or file-deletion authority.
    pub(crate) fn with_http_acquisition_destinations<P: AsRef<Path>, T>(
        &self, paths: &[P], url: &str, work: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        self.with_acquisition_destinations_available_for_http(paths, Some(url), work)?
            .map_err(|path| format!("Acquisition destination {} is reserved by an installed library book or retained archive edition; choose another destination", path.display()))
    }

    fn with_acquisition_destinations_available_for_http<P: AsRef<Path>, T>(
        &self, paths: &[P], http_url: Option<&str>, work: impl FnOnce() -> Result<T, String>,
    ) -> Result<Result<T, PathBuf>, String> {
        if paths.is_empty() || paths.iter().any(|path| !path.as_ref().is_absolute()) {
            return Err("Acquisition destination must be an absolute verified path".into());
        }
        for path in paths {
            path.as_ref().to_str()
                .filter(|path| !path.contains('\0'))
                .ok_or("Acquisition destination must be valid Unicode without NUL")?;
        }
        // Legacy records may be relative. Resolve their lexical base before locking;
        // neither the reservation check nor its normalization touches the filesystem.
        let working_directory = std::env::current_dir()
            .map_err(|error| format!("Cannot resolve legacy library paths: {error}"))?;
        let working_directory = working_directory
            .to_str()
            .ok_or("Cannot resolve legacy library paths outside a Unicode working directory")?;
        let inner = self.lock()?;
        if inner.runtime_read_only || inner.state.settings.read_only {
            return Err("Leave read-only mode before publishing an acquisition".into());
        }
        if let Some(path) = paths.iter().map(|path| -> &Path { path.as_ref() }).find(|destination| {
            let destination = destination.to_str().expect("validated Unicode destination");
            let matches = |path: &str| acquisition_paths_equal(destination, path, working_directory);
            inner.state.books.iter().any(|book| matches(&book.path))
                // An original the person deleted no longer reserves its
                // name, through its transaction or through the intents that
                // targeted it.
                || inner.state.edition_transactions.iter().any(|transaction|
                    (!transaction.original_discarded && matches(&transaction.original.path))
                        || matches(&transaction.replacement.path)
                        || transaction.rollback_book.as_ref().is_some_and(|book| matches(&book.path)))
                || inner.state.edition_intents.iter().any(|intent|
                    (!inner.state.original_discarded(&intent.target) && matches(&intent.target.path))
                        || (matches(&intent.destination)
                        && (!matches!(intent.status.as_str(), "reserved" | "downloading" | "ready")
                            || http_url != Some(intent.selection.entry.url.as_str()))))
        }) {
            return Ok(Err(path.to_path_buf()));
        }
        work().map(Ok)
    }

    pub fn private_mode(&self) -> Result<bool, String> {
        let inner = self.lock()?;
        Ok(inner.state.settings.private_mode || inner.runtime_private)
    }

    pub fn book(&self, id: &str) -> Result<Option<Book>, String> {
        let inner = self.lock()?;
        Ok(inner.state.books.iter().chain(inner.provider_books.iter())
            .find(|book| book.metadata.id == id).cloned())
    }

    pub(crate) fn has_books(&self) -> Result<bool, String> {
        let inner = self.lock()?;
        Ok(!inner.state.books.is_empty() || !inner.provider_books.is_empty())
    }

    /// Path-only membership check for background discovery. Avoids cloning the
    /// whole state for every settled file while holding the same lock.
    #[cfg(any(not(mobile), test))]
    pub(crate) fn has_book_at_path(&self, path: &Path) -> Result<bool, String> {
        let path = path.to_str().ok_or("Archive path is not Unicode")?;
        let inner = self.lock()?;
        Ok(inner
            .state
            .books
            .iter()
            .chain(inner.provider_books.iter())
            .any(|book| acquisition_paths_equal(&book.path, path, "")))
    }

    /// Short, synchronous export handoff/publication under the library mutation
    /// mutex. The action must not call Store methods or await native UI results.
    pub(crate) fn with_current_installation<T>(
        &self,
        id: &str,
        installation: &str,
        action: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let inner = self.lock()?;
        if inner.runtime_read_only || inner.state.settings.read_only {
            return Err("Read-only mode prevents saving files".into());
        }
        if !inner.state.books.iter().chain(inner.provider_books.iter()).any(|book|
            book.metadata.id == id && book.installation_id == installation && book.available)
        {
            return Err("The archive installation changed or is unavailable; open it again".into());
        }
        action()
    }

    /// Explicit protected-mode provider admission, without touching the profile.
    #[cfg(any(mobile, test))]
    pub fn register_provider_book(&self, book: Book) -> Result<(), String> {
        let source = book.source_reference.as_ref().ok_or("Provider source required")?;
        source.validate_book(&book)?;
        if source.persistent { return Err("This archive can only be used for the current session. Reopen it from its original app".into()); }
        let mut inner = self.lock()?;
        if !(inner.runtime_private || inner.runtime_read_only ||
            inner.state.settings.private_mode || inner.state.settings.read_only) {
            return Err("Transient provider admission requires private or read-only mode".into());
        }
        if inner.provider_books.len() >= 32 { return Err("Close an external archive before opening another (32 session archives maximum)".into()); }
        if inner.state.books.iter().chain(inner.provider_books.iter()).any(|current|
            current.metadata.id == book.metadata.id || current.installation_id == book.installation_id ||
            current.source_reference.as_ref().is_some_and(|value| value.reference == source.reference)) {
            return Err("This archive or installation already exists in the library".into());
        }
        let mut checked = State::default();
        checked.books.push(book.clone());
        checked.validate()?;
        inner.provider_books.push(book);
        Ok(())
    }

    /// Atomically reconnect an ephemeral archive, never a durable installation.
    #[cfg(any(mobile, test))]
    pub fn replace_provider_book(&self, expected_installation: &str, replacement: Book) -> Result<(), String> {
        let source = replacement.source_reference.as_ref().ok_or("Provider source required")?;
        source.validate_book(&replacement)?;
        if source.persistent { return Err("This archive can only be used for the current session. Reopen it from its original app".into()); }
        let mut checked = State::default();
        checked.books.push(replacement.clone());
        checked.validate()?;
        let mut inner = self.lock()?;
        if !(inner.runtime_private || inner.runtime_read_only ||
            inner.state.settings.private_mode || inner.state.settings.read_only) {
            return Err("Transient provider replacement requires private or read-only mode".into());
        }
        let index = inner.provider_books.iter().position(|book|
            book.metadata.id == replacement.metadata.id && book.installation_id == expected_installation)
            .ok_or("The session archive changed or is no longer available")?;
        if replacement.installation_id == expected_installation ||
            inner.provider_books[index].source_reference.as_ref()
                .is_some_and(|value| value.reference == source.reference) ||
            inner.state.books.iter().chain(inner.provider_books.iter().enumerate()
                .filter_map(|(other, book)| (other != index).then_some(book))).any(|book|
                    book.metadata.id == replacement.metadata.id ||
                    book.installation_id == replacement.installation_id ||
                    book.source_reference.as_ref().is_some_and(|value| value.reference == source.reference)) {
            return Err("Replacement must create a unique new archive installation".into());
        }
        let old = std::mem::replace(&mut inner.provider_books[index], replacement);
        remove_provider_activity(inner.diverge(), &old);
        Ok(())
    }

    /// Caller retires reader/search/cache capabilities before releasing authority.
    #[cfg(any(mobile, test))]
    pub fn remove_provider_book(&self, id: &str) -> Result<Option<Book>, String> {
        let mut inner = self.lock()?;
        let removed = inner.provider_books.iter().position(|book| book.metadata.id == id)
            .map(|index| inner.provider_books.remove(index));
        if let Some(book) = &removed { remove_provider_activity(inner.diverge(), book); }
        Ok(removed)
    }

    /// Called under the Service policy writer after all windows are retired.
    /// Returned references must be forgotten natively; live Archive clones keep
    /// independent scopes until their last libzim/descriptors have dropped.
    #[cfg(any(mobile, test))]
    pub fn clear_provider_books(&self) -> Result<Vec<Book>, String> {
        let mut inner = self.lock()?;
        let books = std::mem::take(&mut inner.provider_books);
        for book in &books { remove_provider_activity(inner.diverge(), book); }
        Ok(books)
    }

    pub fn locale(&self) -> Result<String, String> {
        Ok(self.lock()?.state.settings.locale.clone())
    }

    pub fn acquisition_consumed(&self, id: &str) -> Result<bool, String> {
        Ok(self.lock()?.state.consumed_acquisitions.contains(id))
    }

    /// Historical edition originals retain their provider capability even when
    /// they are no longer the installed library row.
    #[cfg(any(mobile, test))]
    pub(crate) fn retains_provider_reference(&self, reference: &str) -> Result<bool, String> {
        let inner = self.lock()?;
        let matches = |book: &Book| book.source_reference.as_ref()
            .is_some_and(|source| source.reference == reference);
        // A discarded original has given its capability up, as has every
        // intent that targeted it.
        Ok(inner.state.edition_intents.iter().any(|intent|
                !inner.state.original_discarded(&intent.target) && matches(&intent.target))
            || inner.state.edition_transactions.iter().any(|transaction|
                (!transaction.original_discarded && matches(&transaction.original))
                    || matches(&transaction.replacement)
                    || transaction.rollback_book.as_ref().is_some_and(matches)))
    }

    /// Runtime overrides are not saved as preferences. Disabling private mode discards its
    /// transient browsing state rather than accidentally flushing private activity later.
    pub fn set_runtime_mode(&self, private: bool, read_only: bool) -> Result<(), String> {
        let mut inner = self.lock()?;
        let was_private = inner.runtime_private || inner.state.settings.private_mode;
        let is_private = private || inner.state.settings.private_mode;
        if !was_private && is_private && inner.browsing_pending {
            // A coalesced pre-private scroll delta is acknowledged activity; make
            // it durable before the private cutover clears live browsing.
            let candidate = inner.state.clone();
            if let Err(error) = persist(&self.data_dir, &candidate, self.portable_mode()) {
                if error.committed {
                    inner.persisted = Some(candidate);
                    inner.browsing_pending = false;
                    inner.last_persist = Some(Instant::now());
                    inner.durability_error = Some(error.message.clone());
                }
                return Err(error.message);
            }
            inner.persisted = Some(candidate);
            inner.browsing_pending = false;
            inner.last_persist = Some(Instant::now());
        }
        if !was_private && is_private {
            clear_browsing(inner.diverge());
        }
        if was_private && !is_private {
            let persisted = inner.persisted().clone();
            copy_browsing(&persisted, inner.diverge());
        }
        inner.runtime_private = private;
        inner.runtime_read_only = read_only;
        Ok(())
    }

    /// Publish only for the same installation/content under effective policy.
    /// Runtime-private and read-only checks must never rewrite the profile.
    pub(crate) fn commit_integrity(
        &self, expected: &Book, record: &crate::integrity::IntegrityRecord,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<bool, String> {
        let mut inner = self.lock()?;
        if inner.runtime_private || inner.runtime_read_only
            || inner.state.settings.private_mode || inner.state.settings.read_only
            || cancel.is_cancelled() { return Ok(false); }
        if let Some(error) = &inner.durability_error { return Err(error.clone()); }
        let mut candidate = inner.state.clone();
        let book = candidate.books.iter_mut().find(|book| book.metadata.id == expected.metadata.id)
            .ok_or("Archive no longer in library")?;
        if book.path != expected.path || book.installation_id != expected.installation_id
            || !record.matches(book) { return Err("Archive changed during integrity check".into()); }
        if cancel.is_cancelled() { return Ok(false); }
        book.integrity = Some(record.clone());
        // Only reachable outside private mode, so the durable copy equals the
        // candidate: store it once and share it with the live state.
        if let Err(error) = persist(&self.data_dir, &candidate, self.portable_mode()) {
            if error.committed {
                inner.state = candidate;
                inner.persisted = None;
                inner.durability_error = Some(error.message.clone());
                inner.browsing_pending = false;
                inner.last_persist = Some(Instant::now());
            }
            return Err(error.message);
        }
        inner.state = candidate;
        inner.persisted = None;
        inner.browsing_pending = false;
        inner.last_persist = Some(Instant::now());
        Ok(true)
    }

    /// The closure executes against an isolated clone under one mutex. Closure/validation/
    /// pre-publication I/O failures leave both the current state and the old file intact.
    /// Do not call other Store methods from inside the closure (the mutex is non-reentrant).
    pub fn update<T>(
        &self,
        change: impl FnOnce(&mut State) -> Result<T, String>,
    ) -> Result<T, String> {
        self.update_inner(true, Persist::Durable, |_, state| change(state))
    }

    /// Browsing telemetry (a reading-position refresh for an already-recorded
    /// visit) applies atomically in memory and writes at most once per
    /// [`BROWSING_COALESCE_INTERVAL`]. A crash or a killed process can lose only
    /// the scroll offsets recorded inside that window: every new history entry,
    /// bookmark and settings edit still writes through, and every other
    /// transaction persists the pending delta with the rest of the state.
    ///
    /// Only a change confined to `history`/`recent_searches` is deferred; any
    /// other field writes through exactly as [`Store::update`] does. Profiles
    /// that track reading positions materialize the new offset into
    /// `reading_sync`, so they keep the write-through path.
    pub(crate) fn update_browsing<T>(
        &self,
        change: impl FnOnce(&mut State) -> Result<T, String>,
    ) -> Result<T, String> {
        self.update_inner(true, Persist::Coalesced, |_, state| change(state))
    }

    /// Write a coalesced browsing delta at a lifecycle boundary that must not
    /// leave the last scroll offset only in memory. Returns whether the profile
    /// was written.
    pub fn flush_browsing(&self) -> Result<bool, String> {
        let mut inner = self.lock()?;
        if !inner.browsing_pending {
            return Ok(false);
        }
        // Private and read-only profiles never write browsing state. The delta
        // stays in memory exactly as it would have before the coalescing window.
        if inner.runtime_private || inner.runtime_read_only
            || inner.state.settings.private_mode || inner.state.settings.read_only
        {
            return Ok(false);
        }
        if let Some(error) = &inner.durability_error {
            return Err(error.clone());
        }
        ensure_provider_overlay_excluded(&inner.state, &inner.provider_books)?;
        if let Err(error) = persist(&self.data_dir, &inner.state, self.portable_mode()) {
            if error.committed {
                inner.persisted = None;
                inner.browsing_pending = false;
                inner.last_persist = Some(Instant::now());
                inner.durability_error = Some(error.message.clone());
            }
            return Err(error.message);
        }
        inner.persisted = None;
        inner.browsing_pending = false;
        inner.last_persist = Some(Instant::now());
        Ok(true)
    }

    #[cfg(desktop)]
    pub(crate) fn update_window_capture(
        &self,
        change: impl FnOnce(&HashSet<String>, &mut State) -> Result<(), String>,
    ) -> Result<(), String> {
        self.update_inner(true, Persist::Durable, |inner, state| change(&inner.retired_window_captures, state))
    }

    /// Remote apply and explicit native resolution retain their own causal
    /// bookkeeping in the same atomic commit; they must never echo as new edits.
    pub(crate) fn update_reading_sync<T>(
        &self,
        change: impl FnOnce(&mut State) -> Result<T, String>,
    ) -> Result<T, String> {
        self.update_inner(false, Persist::Durable, |_, state| change(state))
    }

    fn update_inner<T>(
        &self,
        capture_local: bool,
        mode: Persist,
        change: impl FnOnce(&Inner, &mut State) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut inner = self.lock()?;
        if !capture_local && (inner.runtime_private || inner.runtime_read_only
            || inner.state.settings.private_mode || inner.state.settings.read_only) {
            return Err("Private reading sync is unavailable in private or read-only mode".into());
        }
        if inner.runtime_read_only {
            return Err("This profile was opened read-only; state changes are disabled".into());
        }
        if let Some(error) = &inner.durability_error {
            return Err(error.clone());
        }
        let mut candidate = inner.state.clone();
        let result = change(&inner, &mut candidate)?;
        candidate.settings.portable_mode = false;
        self.validate_portable_settings(&candidate.settings)?;
        if inner.state.settings.read_only && candidate.settings.read_only {
            return Err("Leave read-only mode before changing this profile".into());
        }
        let was_private = inner.runtime_private || inner.state.settings.private_mode;
        let is_private = inner.runtime_private || candidate.settings.private_mode;
        let pending_browsing = if !was_private && is_private && inner.browsing_pending {
            Some(candidate.clone())
        } else {
            None
        };
        if !was_private && is_private {
            clear_browsing(&mut candidate);
        }
        if was_private && !is_private {
            copy_browsing(inner.persisted(), &mut candidate);
        }
        if capture_local && !is_private && !candidate.settings.read_only {
            crate::reading_data::capture(&inner.state, &mut candidate)?;
        }
        candidate.validate()?;
        // Leaving read-only is the first write of a profile that skipped
        // records at load; nothing has replaced `state.json` since, so keep
        // its bytes before anything (including relocation below) can.
        if inner.skipped_unpreserved && inner.state.settings.read_only && !candidate.settings.read_only {
            if let Some(bytes) = state_bytes(&self.data_dir.join("state.json"))? {
                let kept = preserve_skipped_original(&self.data_dir, &bytes).map_err(|error| format!(
                    "Kiwix could not keep a copy of the saved items it cannot read before leaving read-only mode ({error}). Free some disk space, then try again."
                ))?;
                // The read-only notice asked for a backup; say where this one is.
                inner.notice = Some(kept_copy_notice(&kept));
            }
            inner.skipped_unpreserved = false;
        }
        #[cfg(mobile)]
        if self.mobile_profile && inner.state.settings.read_only && !candidate.settings.read_only {
            self.mobile_complete_relocation_locked(&mut inner)?;
        }
        #[cfg(desktop)]
        if self.portable_mode() && inner.state.settings.read_only && !candidate.settings.read_only {
            let profile = crate::portable::PortableProfile { data: self.data_dir.clone() };
            profile.prepare(false)?;
            // Record the newly writable root before any manager can persist
            // current-root paths. The immutable runtime view already includes it.
            crate::portable::PortablePaths::initialize(&profile, false)?;
        }
        // Private navigation must not even rewrite the state file or its timestamps.
        if mode == Persist::Coalesced && !is_private
            && inner.last_persist.is_some_and(|at| at.elapsed() < BROWSING_COALESCE_INTERVAL)
            && telemetry_only_change(&inner.state, &candidate)
        {
            // Keep the durable snapshot materialized: private rollback, no-op
            // detection and `diverge` all compare against the last write.
            if inner.persisted.is_none() {
                inner.persisted = Some(inner.state.clone());
            }
            inner.state = candidate;
            inner.browsing_pending = true;
            return Ok(result);
        }
        let persisted = inner.persisted();
        if is_private
            && pending_browsing.is_none()
            && candidate.version == persisted.version
            && candidate.books == persisted.books
            && candidate.bookmarks == persisted.bookmarks
            && candidate.consumed_acquisitions == persisted.consumed_acquisitions
            && candidate.edition_intents == persisted.edition_intents
            && candidate.edition_transactions == persisted.edition_transactions
            && candidate.edition_bindings == persisted.edition_bindings
            && candidate.reference_transitions == persisted.reference_transitions
            && candidate.reference_revision == persisted.reference_revision
            && candidate.reading_sync == persisted.reading_sync
            && candidate.private_sync == persisted.private_sync
            && candidate.settings == persisted.settings
        {
            inner.state = candidate;
            return Ok(result);
        }
        // Private writes preserve the old on-disk browsing state while retaining
        // unrelated library/bookmark/settings changes in the same atomic transaction.
        // Outside private mode the durable copy is the candidate itself, so it is
        // written by reference and stored once instead of cloned twice.
        let durable = if is_private {
            let mut durable = candidate.clone();
            match &pending_browsing {
                Some(pre_private) => copy_browsing(pre_private, &mut durable),
                None => copy_browsing(persisted, &mut durable),
            }
            Some(durable)
        } else {
            None
        };
        let durable_state = durable.as_ref().unwrap_or(&candidate);
        ensure_provider_overlay_excluded(durable_state, &inner.provider_books)?;
        if let Err(error) = persist(&self.data_dir, durable_state, self.portable_mode()) {
            if error.committed {
                inner.state = candidate;
                inner.persisted = durable;
                inner.durability_error = Some(error.message.clone());
                // The file was replaced, so no browsing delta is outstanding.
                inner.browsing_pending = false;
                inner.last_persist = Some(Instant::now());
            }
            return Err(error.message);
        }
        inner.state = candidate;
        inner.persisted = durable;
        inner.browsing_pending = false;
        inner.last_persist = Some(Instant::now());
        Ok(result)
    }
}

fn ensure_provider_overlay_excluded(state: &State, overlay: &[Book]) -> Result<(), String> {
    if state.books.iter().any(|book| book.source_reference.as_ref().is_some_and(|source| !source.persistent)) {
        return Err("Session-only provider references cannot be saved to the profile".into());
    }
    for book in overlay {
        let id = &book.metadata.id;
        let installation = &book.installation_id;
        let tab_mentions = |tab: &Tab| tab.book_id.as_ref() == Some(id) ||
            tab.back.iter().chain(&tab.forward).any(|location| &location.book_id == id);
        if state.books.iter().any(|saved| &saved.metadata.id == id || &saved.installation_id == installation)
            || state.bookmarks.iter().any(|bookmark| &bookmark.book_id == id)
            || state.history.iter().any(|entry| &entry.book_id == id)
            || state.tabs.iter().any(&tab_mentions)
            || state.windows.values().any(|window| window.tabs.iter().any(&tab_mentions))
            || state.settings.book_zoom.contains_key(id)
            || state.settings.sharing_book_ids.contains(id)
            || state.settings.pinned_book_ids.contains(id)
            || state.reader_storage.contains_key(installation) {
            return Err("Session-only external archive activity cannot be saved to the profile".into());
        }
    }
    Ok(())
}

#[cfg(any(mobile, test))]
fn remove_provider_activity(state: &mut State, book: &Book) {
    let id = &book.metadata.id;
    state.bookmarks.retain(|bookmark| &bookmark.book_id != id);
    state.history.retain(|entry| &entry.book_id != id);
    state.reader_storage.remove(&book.installation_id);
    state.settings.book_zoom.remove(id);
    state.settings.sharing_book_ids.retain(|value| value != id);
    state.settings.pinned_book_ids.retain(|value| value != id);
    let prefix = format!("{id}\u{1f}");
    state.recent_searches.retain(|entry| !entry.starts_with(&prefix));
    let retain_tabs = |tabs: &mut Vec<Tab>, active: &mut Option<String>| {
        tabs.retain(|tab| tab.book_id.as_ref() != Some(id) &&
            !tab.back.iter().chain(&tab.forward).any(|location| &location.book_id == id));
        if active.as_ref().is_some_and(|value| !tabs.iter().any(|tab| &tab.id == value)) {
            *active = tabs.first().map(|tab| tab.id.clone());
        }
    };
    retain_tabs(&mut state.tabs, &mut state.active_tab);
    for window in state.windows.values_mut() { retain_tabs(&mut window.tabs, &mut window.active_tab); }
}

/// The saved profile's bytes, bounded; `None` when there is none. Also used by
/// `update_inner` when leaving read-only mode keeps a copy first.
fn state_bytes(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Cannot open {}: {error}", path.display())),
    };
    let mut bytes = Vec::new();
    file.take(MAX_STATE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    if bytes.len() as u64 > MAX_STATE_BYTES {
        return Err(format!("{} exceeds the 128 MiB state limit; preserve the file and export/reduce it with a compatible application", path.display()));
    }
    Ok(Some(bytes))
}

/// Shell theme saved in a profile, read before any webview exists so the first
/// frame can match the user's choice instead of flashing the default. A
/// missing, unreadable or stale profile falls back to the system theme: this is
/// a paint hint, never authority over the settings the store later validates.
#[cfg(desktop)]
pub(crate) fn saved_theme_hint(data_dir: &Path) -> Option<String> {
    #[derive(Deserialize, Default)]
    struct HintSettings {
        #[serde(default)]
        theme: String,
    }
    #[derive(Deserialize)]
    struct Hint {
        #[serde(default)]
        settings: HintSettings,
    }
    let file = File::open(data_dir.join("state.json")).ok()?;
    let hint: Hint = serde_json::from_reader(BufReader::new(file)).ok()?;
    matches!(hint.settings.theme.as_str(), "light" | "dark" | "system")
        .then_some(hint.settings.theme)
}

#[cfg(mobile)]
fn saved_read_only(data_dir: &Path) -> Result<bool, String> {
    #[derive(Default, Deserialize)]
    #[serde(rename_all = "camelCase", default)]
    struct Policy {
        read_only: bool,
    }
    #[derive(Deserialize)]
    struct SavedPolicy {
        #[serde(default)]
        settings: Policy,
    }
    let path = data_dir.join("state.json");
    let Some(bytes) = state_bytes(&path)? else {
        return Ok(false);
    };
    // An unreadable policy cannot be honored; open writable so the profile can
    // be quarantined and repaired instead of denying startup.
    let Ok(saved) = serde_json::from_slice::<SavedPolicy>(&bytes) else {
        return Ok(false);
    };
    Ok(saved.settings.read_only)
}

/// Read `state.json` through a bounded, no-follow descriptor. `Ok(None)` means
/// the profile has no state file yet. Shared by every pre-window inspection so
/// they all see the same bytes and the same refusals.
#[cfg(desktop)]
fn read_hardened_state(data_dir: &Path) -> Result<Option<(Vec<u8>, StateIdentity)>, String> {
    let path = data_dir.join("state.json");
    let metadata = match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "Cannot inspect saved policy in {}: {error}",
                path.display()
            ))
        }
        Ok(metadata) => metadata,
    };
    if !metadata.is_file() || metadata.is_symlink() || metadata.len() > MAX_STATE_BYTES {
        return Err(format!(
            "{} must be a bounded regular non-symlink file",
            path.display()
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(&path)
        .map_err(|error| format!("Cannot read saved policy in {}: {error}", path.display()))?;
    let opened_metadata = file
        .metadata()
        .map_err(|error| format!("Cannot inspect saved policy in {}: {error}", path.display()))?;
    if !opened_metadata.is_file() {
        return Err(format!("{} must be a regular file", path.display()));
    }
    let identity = StateIdentity::of(&opened_metadata);
    let mut bytes = Vec::new();
    file.take(MAX_STATE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Cannot read saved policy in {}: {error}", path.display()))?;
    if bytes.len() as u64 > MAX_STATE_BYTES {
        return Err(format!(
            "{} exceeds the 128 MiB state limit; the original is untouched",
            path.display()
        ));
    }
    Ok(Some((bytes, identity)))
}

/// The saved read-only policy of a desktop profile, read before the profile is
/// claimed so a strict read-only profile never gains a writable lock or a
/// rendezvous socket. It applies the same bounds as portable profile records:
/// `state.json` must be a bounded regular file and never a symlink.
#[cfg(desktop)]
pub(crate) fn saved_desktop_read_only(data_dir: &Path) -> Result<bool, String> {
    #[derive(Default, Deserialize)]
    #[serde(rename_all = "camelCase", default)]
    struct Policy {
        read_only: bool,
    }
    #[derive(Deserialize)]
    struct SavedPolicy {
        #[serde(default)]
        settings: Policy,
    }
    let Some((bytes, _)) = read_hardened_state(data_dir)? else {
        return Ok(false);
    };
    let saved: SavedPolicy = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "Cannot inspect saved policy in {}: {error}. The original is untouched.",
            data_dir.join("state.json").display()
        )
    })?;
    Ok(saved.settings.read_only)
}

#[cfg(desktop)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StateIdentity {
    len: u64,
    modified: Option<std::time::SystemTime>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

#[cfg(desktop)]
impl StateIdentity {
    fn of(metadata: &std::fs::Metadata) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Self {
                len: metadata.len(),
                modified: metadata.modified().ok(),
                device: metadata.dev(),
                inode: metadata.ino(),
            }
        }
        #[cfg(not(unix))]
        Self { len: metadata.len(), modified: metadata.modified().ok() }
    }
}

#[cfg(desktop)]
pub(crate) fn state_identity(data_dir: &Path) -> Option<StateIdentity> {
    let metadata = fs::symlink_metadata(data_dir.join("state.json")).ok()?;
    if !metadata.is_file() || metadata.is_symlink() {
        return None;
    }
    Some(StateIdentity::of(&metadata))
}

/// Everything desktop startup needs from the saved profile before any window
/// exists: the first-paint theme, the read-only policy that must be honored
/// before the profile lock is claimed, and the exact bytes the store reuses, so
/// `state.json` is read once instead of once per inspection.
#[cfg(desktop)]
pub(crate) struct StartupHint {
    /// Paint hint only; never authority over the settings the store validates.
    pub theme: Option<String>,
    /// Saved policy, or why it could not be inspected.
    pub read_only: Result<bool, String>,
    /// The inspected bytes; absent when nothing could be reused.
    pub bytes: Option<Vec<u8>>,
    /// Identity of the file those bytes came from; rechecked after the lock.
    pub identity: Option<StateIdentity>,
}

#[cfg(desktop)]
pub(crate) fn startup_hint(data_dir: &Path) -> StartupHint {
    // Deliberately the same struct names and attributes as
    // `saved_desktop_read_only`'s policy, so the read-only authority keeps its
    // exact accept/reject behavior and its exact error text.
    #[derive(Default, Deserialize)]
    #[serde(rename_all = "camelCase", default)]
    struct Policy {
        read_only: bool,
        theme: serde_json::Value,
    }
    #[derive(Deserialize)]
    struct SavedPolicy {
        #[serde(default)]
        settings: Policy,
    }
    let (bytes, identity) = match read_hardened_state(data_dir) {
        Ok(Some((bytes, identity))) => (bytes, Some(identity)),
        Ok(None) => {
            return StartupHint { theme: None, read_only: Ok(false), bytes: None, identity: None }
        }
        Err(error) => {
            return StartupHint {
                theme: saved_theme_hint(data_dir),
                read_only: Err(error),
                bytes: None,
                identity: None,
            }
        }
    };
    let hint: SavedPolicy = match serde_json::from_slice(&bytes) {
        Ok(hint) => hint,
        Err(error) => {
            // A malformed profile keeps the lenient theme reader and the
            // store's own decode path, so its bytes are not handed over.
            return StartupHint {
                theme: saved_theme_hint(data_dir),
                read_only: Err(format!(
                    "Cannot inspect saved policy in {}: {error}. The original is untouched.",
                    data_dir.join("state.json").display()
                )),
                bytes: None,
                identity: None,
            };
        }
    };
    StartupHint {
        theme: hint.settings.theme.as_str()
            .filter(|theme| matches!(*theme, "light" | "dark" | "system"))
            .map(|theme| theme.to_owned()),
        read_only: Ok(hint.settings.read_only),
        bytes: Some(bytes),
        identity,
    }
}

/// The version boundary is the only place archive UUID storage keys are interpreted.
/// Orphaned legacy keys remain preserved, but can never become a new installation's key.
fn migrate_installation_principals(state: &mut State) -> Result<bool, String> {
    if state.version != 1 {
        return Ok(false);
    }
    unique_ids(
        state.books.iter().map(|book| book.metadata.id.as_str()),
        "book",
    )?;
    for book in &mut state.books {
        book.installation_id = new_installation_id();
        if let Some(storage) = state.reader_storage.remove(&book.metadata.id) {
            state
                .reader_storage
                .insert(book.installation_id.clone(), storage);
        }
    }
    state.version = STATE_VERSION;
    Ok(true)
}

fn clear_browsing(state: &mut State) {
    state.history.clear();
    state.recent_searches.clear();
    state.tabs.clear();
    state.active_tab = None;
    state.reader_storage.clear();
    state.windows.clear();
}
fn copy_browsing(source: &State, target: &mut State) {
    target.history.clone_from(&source.history);
    target.recent_searches.clone_from(&source.recent_searches);
    target.tabs.clone_from(&source.tabs);
    target.active_tab.clone_from(&source.active_tab);
    target.windows.clone_from(&source.windows);
    target.reader_storage.clone_from(&source.reader_storage);
}

/// True when `after` differs from `before` only in the browsing telemetry that
/// a repeatable position refresh may write later (history and search history).
/// Every other field must still be durable before the command returns, so a
/// deferred write can never hide a bookmark, tab, settings or sync edit.
fn telemetry_only_change(before: &State, after: &State) -> bool {
    after.version == before.version
        && after.books == before.books
        && after.bookmarks == before.bookmarks
        && after.tabs == before.tabs
        && after.active_tab == before.active_tab
        && after.windows == before.windows
        && after.reader_storage == before.reader_storage
        && after.consumed_acquisitions == before.consumed_acquisitions
        && after.edition_intents == before.edition_intents
        && after.edition_transactions == before.edition_transactions
        && after.edition_bindings == before.edition_bindings
        && after.reference_transitions == before.reference_transitions
        && after.reference_revision == before.reference_revision
        && after.reading_sync == before.reading_sync
        && after.private_sync == before.private_sync
        && after.settings == before.settings
}

struct PersistError {
    message: String,
    committed: bool,
}

#[cfg(test)]
std::thread_local! {
    /// The next copy of a profile with skipped records fails, as on a full disk.
    static FAIL_NEXT_PRESERVED_COPY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
pub(crate) fn fail_next_preserved_copy_for_test() {
    FAIL_NEXT_PRESERVED_COPY.with(|fail| fail.set(true));
}

#[cfg(all(test, not(windows)))]
std::thread_local! {
    static FAIL_NEXT_DIRECTORY_SYNC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(all(test, not(windows)))]
pub(crate) fn fail_next_directory_sync_for_test() {
    FAIL_NEXT_DIRECTORY_SYNC.with(|fail| fail.set(true));
}

#[cfg(not(windows))]
fn sync_state_directory(parent: &File) -> std::io::Result<()> {
    #[cfg(test)]
    if FAIL_NEXT_DIRECTORY_SYNC.with(|fail| fail.replace(false)) {
        return Err(std::io::Error::other("Injected state-directory synchronization failure"));
    }
    parent.sync_all()
}

/// Folders outside the app that must stay readable across launches under the
/// macOS App Sandbox: those of in-place archives (including the originals and
/// targets that saved edition updates may roll back to or replace), watched
/// folders and the download folder. Archives are read through their folder,
/// so a grant for the file alone is not enough. Provider references are not
/// paths.
fn sandbox_roots(state: &State) -> Vec<crate::sandbox::Root> {
    use crate::sandbox::Root;
    let books = state.books.iter()
        .chain(state.edition_transactions.iter().flat_map(|transaction|
            [&transaction.original, &transaction.replacement].into_iter().chain(transaction.rollback_book.iter())))
        .chain(state.edition_intents.iter().flat_map(|intent| std::iter::once(&intent.target).chain(intent.verified.iter())))
        .filter(|book| book.source_reference.is_none())
        .filter_map(|book| Root::parent_of(&book.path));
    let intents = state.edition_intents.iter().flat_map(|intent|
        Root::parent_of(&intent.destination).into_iter()
            .chain(std::iter::once(Root { path: PathBuf::from(&intent.download_directory), retain: true })));
    books
        .chain(intents)
        // Paused transfers and interrupted moves may still name an earlier
        // download folder, so its grant outlives the setting.
        .chain(std::iter::once(Root { path: PathBuf::from(&state.settings.download_directory), retain: true }))
        .chain(state.settings.monitor_directories.iter().map(Root::folder))
        .collect()
}

fn persist(directory: &Path, state: &State, portable: bool) -> Result<(), PersistError> {
    ensure_provider_overlay_excluded(state, &[]).map_err(|message| PersistError { message, committed: false })?;
    let sandbox_roots = crate::sandbox::is_sandboxed().then(|| sandbox_roots(state));
    if let Some(roots) = &sandbox_roots {
        // Record access to newly referenced folders while their grant is live.
        crate::sandbox::grant(directory, roots);
    }
    let destination = directory.join("state.json");
    let temporary = directory.join(format!(".state-{}.tmp", Uuid::new_v4()));
    let before_commit = |e: std::io::Error| PersistError {
        message: format!(
            "Cannot save {}: {e}. The previous state was retained.",
            destination.display()
        ),
        committed: false,
    };
    // Open the directory before replacing the file so unsupported directory operations
    // fail before publication wherever the platform permits directory synchronization.
    #[cfg(not(windows))]
    let parent = File::open(directory).map_err(before_commit)?;
    let outcome = (|| -> Result<(), PersistError> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&temporary).map_err(before_commit)?;
        let mut writer = BufWriter::new(file);
        #[cfg(desktop)]
        if portable {
            let mut value = serde_json::to_value(state).map_err(|e| PersistError {
                message: format!("Cannot serialize state: {e}; previous state retained"), committed: false,
            })?;
            crate::portable::relative_state(directory, &mut value).map_err(|message| PersistError { message, committed: false })?;
            serde_json::to_writer_pretty(&mut writer, &value).map_err(|e| PersistError {
                message: format!("Cannot serialize state: {e}; previous state retained"), committed: false,
            })?;
        } else {
            // Compact output: this file is machine-owned and rewritten on the
            // ordinary update path, where indentation only costs bytes and time.
            serde_json::to_writer(&mut writer, state).map_err(|e| PersistError {
                message: format!("Cannot serialize state: {e}; previous state retained"), committed: false,
            })?;
        }
        #[cfg(not(desktop))]
        {
            let _ = portable;
            serde_json::to_writer(&mut writer, state).map_err(|e| PersistError {
                message: format!("Cannot serialize state: {e}; previous state retained"), committed: false,
            })?;
        }
        writer.write_all(b"\n").map_err(before_commit)?;
        writer.flush().map_err(before_commit)?;
        if writer.get_ref().metadata().map_err(before_commit)?.len() > MAX_STATE_BYTES {
            return Err(PersistError {
                message: "State exceeds 128 MiB; transaction was not committed".into(),
                committed: false,
            });
        }
        writer.get_ref().sync_all().map_err(before_commit)?;
        drop(writer);
        replace_file(&temporary, &destination).map_err(before_commit)?;
        #[cfg(not(windows))]
        sync_state_directory(&parent).map_err(|e| PersistError {
            message: format!("State replacement succeeded at {}, but directory synchronization failed: {e}. Changes are visible but power-loss durability is uncertain; further writes are disabled until restart.", destination.display()), committed: true,
        })?;
        Ok(())
    })();
    if outcome.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    if let Some(roots) = &sandbox_roots {
        // Forget folders only once a profile that no longer names them is visible.
        if outcome.is_ok() || outcome.as_ref().is_err_and(|error| error.committed) {
            crate::sandbox::release(directory, roots);
        }
    }
    outcome
}

#[cfg(desktop)]
fn persist_session(directory: &Path, snapshot: &SessionSnapshot) -> Result<(), String> {
    let destination = directory.join(SESSION_SNAPSHOT_FILE);
    let temporary = directory.join(format!(".last-session-{}.tmp", Uuid::new_v4()));
    let failure = |error: std::io::Error| {
        format!(
            "Cannot save {}: {error}. The previous saved session was retained.",
            destination.display()
        )
    };
    #[cfg(not(windows))]
    let parent = File::open(directory).map_err(failure)?;
    let outcome = (|| -> Result<(), String> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&temporary).map_err(failure)?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, snapshot)
            .map_err(|e| format!("Cannot serialize the saved session: {e}"))?;
        writer.write_all(b"\n").map_err(failure)?;
        writer.flush().map_err(failure)?;
        // The read side rejects anything above MAX_SESSION_BYTES, so a larger
        // write would only guarantee a later "exceeds the supported size".
        // Refuse it here while the previous snapshot is still intact.
        if writer.get_ref().metadata().map_err(failure)?.len() > MAX_SESSION_BYTES {
            return Err(format!(
                "The saved session exceeds the supported size of {MAX_SESSION_BYTES} bytes. The previous saved session was retained."
            ));
        }
        writer.get_ref().sync_all().map_err(failure)?;
        drop(writer);
        replace_file(&temporary, &destination).map_err(failure)?;
        #[cfg(not(windows))]
        parent.sync_all().map_err(failure)?;
        Ok(())
    })();
    if outcome.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    outcome
}

#[cfg(not(windows))]
pub(crate) fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}
#[cfg(windows)]
pub(crate) fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    // REPLACE_EXISTING | WRITE_THROUGH: no remove-then-rename data-loss window.
    // Antivirus, indexers and sync clients briefly open a just-written file
    // without delete sharing; each retry is the same move of our own file.
    crate::storage::retry_transient_sharing(|| {
        crate::storage::move_file(source, destination, crate::storage::MoveKind::ReplaceWriteThrough)
    })
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;

#[cfg(all(test, desktop))]
mod portable_policy_tests {
    use super::*;

    #[test]
    fn runtime_portable_policy_cannot_be_changed_by_settings_input() {
        let root = std::env::temp_dir().join(format!("kiwix-portable-store-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        fs::write(root.join(".portable"), b"").unwrap();
        let data = root.join("data");
        let profile = crate::portable::PortableProfile { data: data.clone() };
        let store = Store::open_profile(data.clone(), false, Some(profile), None, None).unwrap();
        assert!(store.settings().unwrap().portable_mode);
        let incoming: Settings = serde_json::from_value(serde_json::json!({"portableMode": true})).unwrap();
        assert!(!incoming.portable_mode);
        let projected = serde_json::to_value(store.settings().unwrap()).unwrap();
        let mut accepted: Settings = serde_json::from_value(projected).unwrap();
        assert!(!accepted.portable_mode);
        accepted.theme = "dark".into();
        store.update(|state| { state.settings = accepted; Ok(()) }).unwrap();
        assert!(serde_json::from_value::<Settings>(serde_json::json!({"portableMode": "true"})).is_err());
        assert!(serde_json::from_value::<Settings>(serde_json::json!({"unknownSetting": true})).is_err());
        let before = fs::read(data.join("state.json")).unwrap();
        assert!(store.update(|state| {
            state.settings.download_directory = root.join("external").to_string_lossy().into_owned();
            Ok(())
        }).is_err());
        assert!(store.update(|state| {
            state.settings.monitor_directories.clear();
            Ok(())
        }).is_err());
        assert!(store.validate_portable_directory(&root.join("external")).is_err());
        assert!(store.validate_portable_directory(&data).is_ok());
        assert_eq!(fs::read(data.join("state.json")).unwrap(), before);
        assert_eq!(store.settings().unwrap().download_directory, data.to_string_lossy());
        assert!(!root.join("external").exists());
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn normal_profile_restart_consumes_but_never_trusts_runtime_portable_flag() {
        let root = std::env::temp_dir().join(format!("kiwix-normal-runtime-{}", Uuid::new_v4()));
        let store = Store::open_profile(root.clone(), false, None, None, None).unwrap();
        assert!(!store.settings().unwrap().portable_mode);
        let mut settings = serde_json::to_value(store.settings().unwrap()).unwrap();
        settings["portableMode"] = serde_json::Value::Bool(true);
        settings["theme"] = serde_json::json!("dark");
        let settings: Settings = serde_json::from_value(settings).unwrap();
        store.update(|state| { state.settings = settings; Ok(()) }).unwrap();
        drop(store);
        let store = Store::open_profile(root.clone(), false, None, None, None).unwrap();
        assert_eq!(store.settings().unwrap().theme, "dark");
        assert!(!store.settings().unwrap().portable_mode);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn native_colon_and_backslash_archives_survive_real_profile_restart_and_move() {
        let base = std::env::temp_dir().join(format!("kiwix-native-portable-{}", Uuid::new_v4()));
        fs::create_dir(&base).unwrap();
        let base = fs::canonicalize(base).unwrap();
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim");
        let fixture_bytes = fs::read(&fixture).unwrap();
        for name in ["archive:colon.zim", r"archive\backslash.zim"] {
            let source = base.join(format!("source:{}\\folder", Uuid::new_v4()));
            fs::create_dir(&source).unwrap();
            fs::write(source.join(".portable"), b"").unwrap();
            let profile = crate::portable::PortableProfile { data: source.join("data") };
            let store = Store::open_profile(profile.data.clone(), false, Some(profile.clone()), None, None).unwrap();
            let path = profile.data.join(name);
            fs::copy(&fixture, &path).unwrap();
            let book = crate::imports::load_book(path.to_str().unwrap(), false, false).unwrap();
            let id = book.metadata.id.clone();
            let installation = book.installation_id.clone();
            store.update(|state| { state.books.push(book); Ok(()) }).unwrap();
            drop(store);
            let store = Store::open_profile(profile.data.clone(), false, Some(profile.clone()), None, None).unwrap();
            let book = store.book(&id).unwrap().unwrap();
            assert_eq!(Path::new(&book.path), path);
            assert_eq!(fs::read(&book.path).unwrap(), fixture_bytes);
            drop(store);
            let moved = base.join(format!("moved:{}\\folder", Uuid::new_v4()));
            fs::rename(&source, &moved).unwrap();
            let profile = crate::portable::PortableProfile { data: moved.join("data") };
            let before = fs::read(profile.data.join("state.json")).unwrap();
            let store = Store::open_profile(profile.data.clone(), true, Some(profile.clone()), None, None).unwrap();
            let book = store.book(&id).unwrap().unwrap();
            assert_eq!(Path::new(&book.path), profile.data.join(name));
            assert_eq!(book.installation_id, installation);
            assert_eq!(crate::imports::load_book(&book.path, false, false).unwrap().metadata.id, id);
            assert_eq!(fs::read(&book.path).unwrap(), fixture_bytes);
            drop(store);
            assert_eq!(fs::read(profile.data.join("state.json")).unwrap(), before);
            let store = Store::open_profile(profile.data.clone(), false, Some(profile.clone()), None, None).unwrap();
            store.update(|state| { state.settings.theme = "dark".into(); Ok(()) }).unwrap();
            drop(store);
            let store = Store::open_profile(profile.data.clone(), false, Some(profile), None, None).unwrap();
            assert_eq!(Path::new(&store.book(&id).unwrap().unwrap().path), moved.join("data").join(name));
            drop(store);
        }
        fs::remove_dir_all(base).unwrap();
    }

    /// A second instance must be a typed "busy" answer at every desktop
    /// boundary: the claim, the store's checked opener, and the conversion
    /// setup sees. A genuine open failure must stay fatal so startup still
    /// reports it.
    #[cfg(desktop)]
    #[test]
    fn a_live_owner_is_a_typed_busy_profile_not_a_startup_failure() {
        let root = std::env::temp_dir().join(format!("kiwix-busy-store-{}", Uuid::new_v4()));
        let owner = claim_profile_lock(&root, false).unwrap();
        let refused = claim_profile_lock(&root, false).unwrap_err();
        assert!(matches!(refused, ProfileLockError::Busy { .. }));
        assert!(matches!(
            StoreOpenError::from(refused),
            StoreOpenError::Busy(_)
        ));
        match Store::new_with_mode_checked(root.clone(), false, None) {
            Err(StoreOpenError::Busy(_)) => {}
            Err(StoreOpenError::Fatal(error)) => {
                panic!("a live owner must not be a fatal startup error: {error}")
            }
            Ok(_) => panic!("the profile is locked by its owner"),
        }
        // SIGKILL or a clean exit releases the flock; the next instance opens
        // the same profile normally. A concurrently forked child can inherit
        // the descriptor between fork and exec and hold the advisory lock for a
        // few milliseconds, so allow a short bounded settle before failing.
        drop(owner);
        let settle = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let store = loop {
            match Store::new_with_mode_checked(root.clone(), false, None) {
                Ok(store) => break store,
                Err(StoreOpenError::Busy(_)) if std::time::Instant::now() < settle => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) => panic!("the released profile did not reopen: {error:?}"),
            }
        };
        drop(store);
        // A profile that cannot be opened at all still fails startup.
        assert!(matches!(
            Store::new_with_mode_checked(root.join("absent"), true, None),
            Err(StoreOpenError::Fatal(_))
        ));
        fs::remove_dir_all(&root).unwrap();
    }
}

#[cfg(test)]
#[path = "reading_sync_store_tests.rs"]
mod reading_sync_tests;
