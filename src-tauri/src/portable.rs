// SPDX-License-Identifier: GPL-3.0-or-later
//! Desktop portable profiles are selected only by a real `.portable` file.
#![forbid(unsafe_code)]
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use std::{collections::HashMap, fs, io::{Read, Write}, path::{Component, Path, PathBuf}};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use crate::profile_paths::RecordKind;

#[derive(Clone, Debug)]
pub struct PortableProfile {
    pub data: PathBuf,
}

fn real_directory(path: &Path) -> Result<Dir, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Cannot inspect portable directory {}: {error}", path.display()))?;
    if metadata.is_symlink() || !metadata.is_dir() {
        return Err(format!("Portable directory must not be a symbolic link: {}", path.display()));
    }
    Dir::open_ambient_dir(path, cap_std::ambient_authority()).map_err(|error| error.to_string())
}

fn distribution_root(executable: &Path) -> Result<PathBuf, String> {
    #[cfg(target_os = "linux")]
    if let Some(image) = std::env::var_os("APPIMAGE") {
        let image = PathBuf::from(image);
        if image.is_absolute() {
            let metadata = fs::symlink_metadata(&image)
                .map_err(|error| format!("Cannot inspect APPIMAGE {}: {error}", image.display()))?;
            if metadata.is_file() && !metadata.is_symlink() {
                return image.parent().map(Path::to_path_buf).ok_or("APPIMAGE has no distribution directory".into());
            }
        }
        return Err("APPIMAGE must name the absolute regular, non-symlink distributed AppImage".into());
    }
    let parent = executable.parent().ok_or("Executable has no distribution directory")?;
    #[cfg(target_os = "macos")]
    if parent.file_name().is_some_and(|name| name == "MacOS") {
        if let Some(contents) = parent.parent().filter(|path| path.file_name().is_some_and(|name| name == "Contents")) {
            if let Some(bundle) = contents.parent().filter(|path| path.extension().is_some_and(|extension| extension == "app")) {
                return bundle.parent().map(Path::to_path_buf).ok_or("App bundle has no distribution directory".into());
            }
        }
    }
    Ok(parent.to_path_buf())
}

impl PortableProfile {
    pub fn detect() -> Result<Option<Self>, String> {
        // The macOS App Sandbox cannot see or write beside the app bundle, so
        // portable mode is unavailable there; the profile lives in the container.
        if crate::sandbox::is_sandboxed() {
            return Ok(None);
        }
        let executable = std::env::current_exe().map_err(|error| format!("Cannot locate executable: {error}"))?;
        let root = distribution_root(&executable)?;
        Self::at_root(&root)
    }

    fn at_root(root: &Path) -> Result<Option<Self>, String> {
        let directory = real_directory(root)?;
        match directory.symlink_metadata(".portable") {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("Cannot inspect portable marker: {error}")),
            Ok(metadata) if !metadata.is_file() || metadata.is_symlink() => {
                return Err("Portable marker .portable must be a regular file, not a directory or symbolic link".into());
            }
            Ok(_) => (),
        }
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let marker = directory.open_with(".portable", &options).map_err(|error| format!("Cannot open portable marker: {error}"))?;
        if !marker.metadata().map_err(|error| error.to_string())?.is_file() {
            return Err("Portable marker changed while opening".into());
        }
        let root = fs::canonicalize(root).map_err(|error| error.to_string())?;
        Ok(Some(Self { data: root.join("data") }))
    }

    pub fn matches(&self, requested: &Path) -> Result<bool, String> {
        let absolute = if requested.is_absolute() { requested.to_path_buf() } else {
            std::env::current_dir().map_err(|error| error.to_string())?.join(requested)
        };
        let mut normalized = PathBuf::new();
        for component in absolute.components() {
            match component {
                Component::CurDir => (),
                Component::ParentDir => { normalized.pop(); }
                component => normalized.push(component.as_os_str()),
            }
        }
        if normalized == self.data { return Ok(true); }
        match fs::symlink_metadata(&normalized) {
            Ok(metadata) if metadata.is_symlink() => Ok(false),
            Ok(_) => fs::canonicalize(&normalized).map(|path| path == self.data)
                .map_err(|error| format!("Cannot identify requested portable data directory: {error}")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let Some(parent) = normalized.parent() else { return Ok(false); };
                let Some(name) = normalized.file_name() else { return Ok(false); };
                match fs::canonicalize(parent) {
                    Ok(parent) => Ok(parent.join(name) == self.data),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
                    Err(error) => Err(error.to_string()),
                }
            }
            Err(error) => Err(error.to_string()),
        }
    }

    pub fn saved_read_only(&self) -> Result<bool, String> {
        let parent = real_directory(self.data.parent().ok_or("Portable data has no parent")?)?;
        match parent.symlink_metadata("data") {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.to_string()),
            Ok(metadata) if !metadata.is_dir() || metadata.is_symlink() => return Err("Portable data must be a real non-symlink directory".into()),
            Ok(_) => (),
        }
        let directory = parent.open_dir_nofollow("data").map_err(|error| error.to_string())?;
        let Some(bytes) = read_regular(&directory, "state.json", 128 * 1024 * 1024)? else { return Ok(false); };
        #[derive(Default, Deserialize)]
        #[serde(default, rename_all = "camelCase")]
        struct Policy { read_only: bool }
        #[derive(Deserialize)]
        struct Saved { #[serde(default)] settings: Policy }
        Ok(serde_json::from_slice::<Saved>(&bytes).map_err(|error| format!("Cannot inspect portable saved policy: {error}"))?.settings.read_only)
    }

    pub fn prepare(&self, read_only: bool) -> Result<(), String> {
        let parent = self.data.parent().ok_or("Portable data has no parent")?;
        let directory = real_directory(parent)?;
        match directory.symlink_metadata("data") {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && !read_only => {
                directory.create_dir("data").map_err(|error| format!("Cannot create portable data directory: {error}"))?;
            }
            Err(error) => return Err(format!("Cannot open portable data directory: {error}")),
            Ok(metadata) if !metadata.is_dir() || metadata.is_symlink() => return Err("Portable data must be a real directory, not a symbolic link".into()),
            Ok(_) => (),
        }
        let data = directory.open_dir_nofollow("data").map_err(|error| error.to_string())?;
        // Existing app-owned automatic roots may not redirect writes elsewhere.
        for name in ["downloads", "maps", "torrents", "localswarm", "spelling", "webview", "profile", "library-migrations-v1", "attachment-exports-v1"] {
            match data.symlink_metadata(name) {
                Ok(metadata) if !metadata.is_dir() || metadata.is_symlink() => return Err(format!("Portable app-owned directory {name} must be a real directory")),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                Err(error) => return Err(error.to_string()),
                Ok(_) => (),
            }
        }
        for (root, children) in [("profile", &["caches", "logs"][..]), ("maps", &["map-parts"][..])] {
            let directory = match data.open_dir_nofollow(root) {
                Ok(directory) => directory,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.to_string()),
            };
            for child in children {
                match directory.symlink_metadata(child) {
                    Ok(metadata) if !metadata.is_dir() || metadata.is_symlink() => return Err(format!("Portable automatic directory {root}/{child} must be a real directory")),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                    Err(error) => return Err(error.to_string()),
                    Ok(_) => (),
                }
            }
        }
        for name in ["state.json", "store.lock", "diagnostics.jsonl", "diagnostics.previous.jsonl", ".portable-roots.json"] {
            match data.symlink_metadata(name) {
                Ok(metadata) if !metadata.is_file() || metadata.is_symlink() => return Err(format!("Portable app-owned file {name} must be a regular non-symlink file")),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                Err(error) => return Err(error.to_string()),
                Ok(_) => (),
            }
        }
        if !read_only {
            let name = format!(".portable-write-{}", uuid::Uuid::new_v4());
            let mut options = OpenOptions::new();
            options.write(true).create_new(true).follow(FollowSymlinks::No);
            let file = data.open_with(&name, &options).map_err(|error| format!("Portable data directory is not writable: {error}"))?;
            drop(file);
            data.remove_file(&name).map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    pub fn webview_directory(&self, label: &str) -> Result<PathBuf, String> {
        if label.is_empty() || !label.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-') {
            return Err("Invalid portable webview label".into());
        }
        self.prepare(true)?;
        let webview = self.data.join("webview");
        match fs::symlink_metadata(&webview) {
            Ok(metadata) if !metadata.is_dir() || metadata.is_symlink() => return Err("Portable webview root must be a real directory".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.to_string()),
            Ok(_) => (),
        }
        let path = webview.join(label);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if !metadata.is_dir() || metadata.is_symlink() => return Err("Portable webview profile must be a real directory".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.to_string()),
            Ok(_) => (),
        }
        Ok(path)
    }
}

/// An append-only history of roots, committed before any current-root job can be
/// saved. Retaining old roots makes interrupted writes idempotent without changing
/// the jobs' file identities, hashes, or destructive-recovery evidence.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortablePaths {
    version: u32,
    owner: String,
    roots: Vec<String>,
    #[serde(skip)]
    current: PathBuf,
    #[serde(skip)]
    receipts: HashMap<String, String>,
}

impl PortablePaths {
    pub fn initialize(profile: &PortableProfile, read_only: bool) -> Result<Self, String> {
        let directory = real_directory(&profile.data)?;
        let bytes = read_regular(&directory, ".portable-roots.json", 1024 * 1024)?;
        let mut paths = match bytes {
            Some(bytes) => serde_json::from_slice::<Self>(&bytes).map_err(|error| format!("Invalid portable root history; original retained: {error}"))?,
            None => Self { version: 1, owner: uuid::Uuid::new_v4().to_string(), roots: Vec::new(), current: PathBuf::new(), receipts: HashMap::new() },
        };
        if paths.version != 1 || uuid::Uuid::parse_str(&paths.owner).map_err(|error| error.to_string())?.to_string() != paths.owner || paths.roots.len() > 1024 {
            return Err("Unsupported or invalid portable root history".into());
        }
        for root in &paths.roots { validate_absolute(root)?; }
        paths.current = profile.data.clone();
        let current = paths.current.to_str().ok_or("Portable directory must be valid Unicode")?.to_owned();
        if !paths.roots.contains(&current) {
            if paths.roots.len() == 1024 { return Err("Portable root history has reached its safety limit".into()); }
            paths.roots.push(current);
            if !read_only {
                let bytes = serde_json::to_vec(&paths).map_err(|error| error.to_string())?;
                if bytes.len() > 1024 * 1024 { return Err("Portable root history exceeds its safety size limit".into()); }
                let temporary = format!(".portable-roots-{}.tmp", uuid::Uuid::new_v4());
                let commit = (|| -> Result<(), String> {
                    let mut options = OpenOptions::new();
                    options.write(true).create_new(true).follow(FollowSymlinks::No);
                    let mut file = directory.open_with(&temporary, &options).map_err(|error| error.to_string())?;
                    file.write_all(&bytes).map_err(|error| error.to_string())?;
                    file.sync_all().map_err(|error| error.to_string())?;
                    drop(file);
                    crate::store::replace_file(&profile.data.join(&temporary), &profile.data.join(".portable-roots.json")).map_err(|error| format!("Cannot commit portable root history: {error}"))?;
                    #[cfg(unix)]
                    crate::storage::sync_cap_directory(&directory).map_err(|error| format!("Cannot synchronize portable root history: {error}"))?;
                    Ok(())
                })();
                if commit.is_err() { let _ = directory.remove_file(&temporary); }
                commit?;
            }
        }
        // Receipt IDs hash the exact completed pathname, not just the job ID.
        // Correlate every historical root, including multiple interrupted moves.
        let torrents = match directory.symlink_metadata("torrents") {
            Ok(metadata) if metadata.is_dir() && !metadata.is_symlink() => Some(directory.open_dir_nofollow("torrents").map_err(|error| error.to_string())?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.to_string()),
            Ok(_) => return Err("Portable torrent root must be a real directory".into()),
        };
        if let Some(torrents) = torrents {
            if let Some(bytes) = read_regular(&torrents, "jobs.json", 128 * 1024 * 1024)? {
                // The journal may reach 128 MiB; decode only the correlation
                // fields instead of materializing a full `Value` tree for it.
                #[derive(Deserialize)]
                struct SavedTorrents {
                    #[serde(default)]
                    jobs: Option<Vec<SavedTorrentRow>>,
                }
                #[derive(Deserialize)]
                struct SavedTorrentRow {
                    #[serde(default)]
                    job: Option<SavedTorrentJob>,
                }
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct SavedTorrentJob {
                    #[serde(default)]
                    id: Option<Value>,
                    #[serde(default)]
                    completed_paths: Option<Value>,
                }
                // A syntax error keeps its JSON position; a structurally
                // complete document missing these fields is reported as damage.
                let saved: SavedTorrents = serde_json::from_slice(&bytes).map_err(|error| {
                    match error.classify() {
                        serde_json::error::Category::Syntax | serde_json::error::Category::Eof => error.to_string(),
                        _ => "Saved transfer history is damaged. Restore a backup".to_string(),
                    }
                })?;
                drop(bytes);
                for row in saved.jobs.as_deref().ok_or("Saved transfer history is damaged. Restore a backup")? {
                    let job = row.job.as_ref().ok_or("Saved transfer history is damaged. Restore a backup")?;
                    let id = job.id.as_ref().and_then(Value::as_str).ok_or("Torrent job has no ID")?;
                    let suffix = id.strip_prefix("torrent:").ok_or("Invalid torrent job ID")?;
                    if uuid::Uuid::parse_str(suffix).map_err(|error| error.to_string())?.to_string() != suffix { return Err("Invalid torrent job ID".into()); }
                    let completed = job.completed_paths.as_ref().and_then(Value::as_array)
                        .ok_or("Torrent job has no completed paths")?;
                    for path in completed {
                        let original = path.as_str().ok_or("Torrent completed path is not a string")?;
                        let relative = paths.relative(original)?.ok_or("Portable torrent completed path is outside portable data")?;
                        let current = paths.resolve(&relative)?;
                        let replacement = receipt(id, &current);
                        for root in &paths.roots {
                            let separator = if windows_absolute(root) { "\\" } else { "/" };
                            let old = if relative.is_empty() { root.clone() } else { format!("{}{separator}{}", root.trim_end_matches('/'), relative.join(separator)) };
                            let source = receipt(id, &old);
                            if let Some(previous) = paths.receipts.insert(source, replacement.clone()) {
                                if previous != replacement { return Err("Conflicting portable torrent receipt correlations".into()); }
                            }
                        }
                        let source = receipt(id, original);
                        if let Some(previous) = paths.receipts.insert(source, replacement.clone()) {
                            if previous != replacement { return Err("Conflicting portable torrent receipt correlations".into()); }
                        }
                    }
                }
            }
        }
        Ok(paths)
    }

    fn relative(&self, path: &str) -> Result<Option<Vec<String>>, String> {
        if path.contains('\0') { return Err("Portable filesystem path contains NUL".into()); }
        let windows = windows_absolute(path);
        let normalized = if windows { path.replace('\\', "/") } else { path.to_owned() };
        if normalized.split('/').any(|part| part == ".." || part == ".") { return Err("Portable filesystem path contains traversal".into()); }
        if !is_absolute(&normalized) && !windows {
            // Only tagged component records are data-relative. Bare legacy
            // strings retain their existing meaning; never guess their base.
            return Ok(None);
        }
        let mut best: Option<Vec<String>> = None;
        let mut best_length = 0;
        for root in &self.roots {
            if windows_absolute(root) != windows { continue; }
            let root = if windows { root.replace('\\', "/") } else { root.clone() };
            let root = root.trim_end_matches('/');
            let relative = if normalized == root { Some("") } else {
                normalized.strip_prefix(root).and_then(|suffix| suffix.strip_prefix('/'))
            };
            if let Some(relative) = relative {
                if root.len() >= best_length {
                    best = Some(relative.split('/').filter(|part| !part.is_empty()).map(str::to_owned).collect());
                    best_length = root.len();
                }
            }
        }
        Ok(best)
    }

    fn resolve(&self, relative: &[String]) -> Result<String, String> {
        let mut path = self.current.clone();
        for component in relative {
            validate_component(component)?;
            path.push(component);
        }
        path.into_os_string().into_string().map_err(|_| "Portable filesystem path is not Unicode".into())
    }

    pub fn rebase_json(&self, kind: RecordKind, value: &mut Value) -> Result<bool, String> {
        let mut changed = crate::profile_paths::rewrite(kind, value, true, |value, role| {
            if value.is_null() { return Ok(false); }
            if value.is_object() && matches!(kind, RecordKind::State) {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct RelativeArchive {
                    #[serde(rename = "portableComponents")]
                    components: Vec<String>,
                }
                let relative: RelativeArchive = serde_json::from_value(value.take())
                    .map_err(|error| format!("Invalid portable archive pathname: {error}"))?;
                *value = Value::String(self.resolve(&relative.components)?);
                return Ok(true);
            }
            let original = value.as_str().ok_or("Persisted filesystem path is not a string")?;
            if original.is_empty() { return Ok(false); }
            let Some(relative) = self.relative(original)? else {
                // Imported originals and explicit user-selected external archives
                // remain external. Automatic transfer destinations never may.
                if matches!(kind, RecordKind::State) || role == crate::profile_paths::PathRole::ExplicitSource { return Ok(false); }
                return Err(format!("Saved automatic job points outside portable data: {original}"));
            };
            let replacement = self.resolve(&relative)?;
            if replacement == original { return Ok(false); }
            *value = Value::String(replacement);
            Ok(true)
        })?;
        if matches!(kind, RecordKind::State) {
            if let Some(receipts) = value.get_mut("consumedAcquisitions").and_then(Value::as_array_mut) {
                let mut receipts_changed = false;
                for value in receipts.iter_mut() {
                    let source = value.as_str().ok_or("Consumed acquisition receipt is not a string")?;
                    if let Some(replacement) = self.receipts.get(source) {
                        if replacement != source { *value = Value::String(replacement.clone()); receipts_changed = true; }
                    }
                }
                if receipts_changed { receipts.sort_unstable_by(|a, b| a.as_str().cmp(&b.as_str())); receipts.dedup(); changed = true; }
            }
            if let Some(intents) = value.get_mut("editionIntents").and_then(Value::as_array_mut) {
                for intent in intents {
                    if let Some(receipt) = intent.get_mut("receipt").filter(|receipt| !receipt.is_null()) {
                        let source = receipt.as_str().ok_or("Edition acquisition receipt is not a string")?;
                        if let Some(replacement) = self.receipts.get(source).filter(|replacement| replacement.as_str() != source) {
                            *receipt = Value::String(replacement.clone());
                            changed = true;
                        }
                    }
                }
            }
        }
        Ok(changed)
    }
}

fn receipt(id: &str, path: &str) -> String {
    use sha2::Digest;
    format!("{id}:{}", hex::encode(sha2::Sha256::digest(path.as_bytes())))
}
fn is_absolute(path: &str) -> bool {
    path.starts_with('/') || (path.as_bytes().get(1) == Some(&b':') && path.as_bytes().get(2) == Some(&b'/'))
}
fn validate_absolute(path: &str) -> Result<(), String> {
    let normalized = if windows_absolute(path) { path.replace('\\', "/") } else { path.to_owned() };
    if !is_absolute(&normalized) || normalized.contains('\0') || normalized.split('/').any(|part| part == "." || part == "..") {
        return Err("Portable root history contains an invalid absolute path".into());
    }
    Ok(())
}

fn windows_absolute(path: &str) -> bool {
    path.starts_with("\\\\") || (path.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && path.as_bytes().get(1) == Some(&b':')
        && matches!(path.as_bytes().get(2), Some(b'/') | Some(b'\\')))
}

fn validate_component(component: &str) -> Result<(), String> {
    if component.is_empty() || component.contains('\0') || component.contains('/')
        || !matches!(Path::new(component).components().next(), Some(Component::Normal(_)))
        || Path::new(component).components().count() != 1
    {
        return Err("Portable archive pathname contains an invalid native component".into());
    }
    #[cfg(windows)]
    if component.contains([':', '\\']) {
        return Err("Portable archive pathname cannot be represented on Windows".into());
    }
    Ok(())
}

fn native_relative_components(path: &str) -> Result<Vec<String>, String> {
    let mut components = Vec::new();
    for component in Path::new(path).components() {
        let Component::Normal(component) = component else { return Err("Portable path is not a normalized relative pathname".into()); };
        let component = component.to_str().ok_or("Portable path is not Unicode")?;
        validate_component(component)?;
        components.push(component.to_owned());
    }
    Ok(components)
}
fn read_regular(directory: &Dir, name: &str, limit: u64) -> Result<Option<Vec<u8>>, String> {
    match directory.symlink_metadata(name) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
        Ok(metadata) if !metadata.is_file() || metadata.is_symlink() || metadata.len() > limit => return Err("A saved file in the portable folder is not valid. Replace it from a backup".to_string()),
        Ok(_) => (),
    }
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = directory.open_with(name, &options).map_err(|error| error.to_string())?;
    if !file.metadata().map_err(|error| error.to_string())?.is_file() { return Err("A saved file changed while Kiwix was reading it. Restart the app".into()); }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > limit { return Err("A saved file is larger than expected. Restart the app".into()); }
    Ok(Some(bytes))
}

/// Only data-owned archive paths become relative on disk. Explicit external
/// originals retain their absolute paths and existing consent/ownership semantics.
pub fn relative_state(directory: &Path, state: &mut Value) -> Result<(), String> {
    fn relative_path(directory: &Path, value: &mut Value) -> Result<(), String> {
        let path = Path::new(value.as_str().ok_or("Archive path is not a string")?);
        if let Ok(relative) = path.strip_prefix(directory) {
            let components = native_relative_components(relative.to_str().ok_or("Archive path is not Unicode")?)?;
            *value = serde_json::json!({ "portableComponents": components });
        }
        Ok(())
    }
    fn book(directory: &Path, book: &mut Value) -> Result<(), String> {
        if book.is_null() || book.get("sourceReference").is_some_and(|source| !source.is_null()) { return Ok(()); }
        if let Some(path) = book.get_mut("path") { relative_path(directory, path)?; }
        Ok(())
    }
    if let Some(books) = state.get_mut("books").and_then(Value::as_array_mut) {
        for value in books { book(directory, value)?; }
    }
    for collection in ["editionIntents", "editionTransactions"] {
        if let Some(records) = state.get_mut(collection).and_then(Value::as_array_mut) {
            for record in records {
                if collection == "editionIntents" {
                    for key in ["destination", "downloadDirectory"] {
                        if let Some(path) = record.get_mut(key) { relative_path(directory, path)?; }
                    }
                }
                for key in ["target", "verified", "original", "replacement", "rollbackBook"] {
                    if let Some(value) = record.get_mut(key) { book(directory, value)?; }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Temporary(PathBuf);
    impl Temporary {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("kiwix-portable-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(fs::canonicalize(path).unwrap())
        }
        fn profile(&self, name: &str) -> PortableProfile {
            let root = self.0.join(name);
            fs::create_dir(&root).unwrap();
            fs::write(root.join(".portable"), b"").unwrap();
            let profile = PortableProfile::at_root(&root).unwrap().unwrap();
            profile.prepare(false).unwrap();
            profile
        }
    }
    impl Drop for Temporary {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
    }

    #[test]
    fn marker_must_be_a_real_regular_file_and_readonly_never_creates_data() {
        let temporary = Temporary::new();
        assert!(PortableProfile::at_root(&temporary.0).unwrap().is_none());
        fs::create_dir(temporary.0.join(".portable")).unwrap();
        assert!(PortableProfile::at_root(&temporary.0).is_err());
        fs::remove_dir(temporary.0.join(".portable")).unwrap();
        fs::write(temporary.0.join(".portable"), b"").unwrap();
        let profile = PortableProfile::at_root(&temporary.0).unwrap().unwrap();
        assert!(profile.prepare(true).is_err());
        assert!(!profile.data.exists());
        profile.prepare(false).unwrap();
        assert!(profile.matches(&profile.data).unwrap());
        assert!(!profile.matches(&temporary.0.join("elsewhere")).unwrap());
        assert!(profile.webview_directory("../escape").is_err());
        assert_eq!(profile.webview_directory("window-1").unwrap(), profile.data.join("webview/window-1"));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_marker_data_and_automatic_roots_are_rejected() {
        use std::os::unix::fs::symlink;
        let temporary = Temporary::new();
        fs::write(temporary.0.join("ordinary-file"), b"").unwrap();
        symlink("ordinary-file", temporary.0.join(".portable")).unwrap();
        assert!(PortableProfile::at_root(&temporary.0).is_err());
        fs::remove_file(temporary.0.join(".portable")).unwrap();
        fs::write(temporary.0.join(".portable"), b"").unwrap();
        fs::create_dir(temporary.0.join("foreign")).unwrap();
        symlink("foreign", temporary.0.join("data")).unwrap();
        let profile = PortableProfile::at_root(&temporary.0).unwrap().unwrap();
        assert!(profile.prepare(false).is_err());
        fs::remove_file(&profile.data).unwrap();
        profile.prepare(false).unwrap();
        symlink(temporary.0.join("foreign"), profile.data.join("downloads")).unwrap();
        assert!(profile.prepare(false).is_err());
        assert_eq!(fs::read_dir(temporary.0.join("foreign")).unwrap().count(), 0);
    }

    #[test]
    fn relative_archives_survive_move_but_external_originals_stay_external() {
        let temporary = Temporary::new();
        let original = temporary.profile("original");
        let moved = temporary.profile("moved");
        let paths = PortablePaths::initialize(&original, false).unwrap();
        let mut state = json!({"books": [
            {"path": original.data.join("direct.zim"), "managed": false},
            {"path": original.data.join("archives/downloaded.zim"), "managed": true},
            {"path": temporary.0.join("external.zim"), "managed": false}
        ], "settings": {"downloadDirectory": "/old/external", "monitorDirectories": []}});
        relative_state(&original.data, &mut state).unwrap();
        fs::copy(original.data.join(".portable-roots.json"), moved.data.join(".portable-roots.json")).unwrap();
        let before = fs::read(moved.data.join(".portable-roots.json")).unwrap();
        let moved_paths = PortablePaths::initialize(&moved, true).unwrap();
        assert!(moved_paths.rebase_json(RecordKind::State, &mut state).unwrap());
        assert_eq!(state["books"][0]["path"], moved.data.join("direct.zim").to_str().unwrap());
        assert_eq!(state["books"][1]["path"], moved.data.join("archives/downloaded.zim").to_str().unwrap());
        assert_eq!(state["books"][2]["path"], temporary.0.join("external.zim").to_str().unwrap());
        assert_eq!(fs::read(moved.data.join(".portable-roots.json")).unwrap(), before);
        assert!(paths.relative("/unknown/../escape").is_err());
    }

    #[test]
    fn moved_jobs_rebase_receipts_without_refreshing_identity_evidence() {
        let temporary = Temporary::new();
        let original = temporary.profile("original");
        let moved = temporary.profile("moved");
        PortablePaths::initialize(&original, false).unwrap();
        let id = format!("torrent:{}", uuid::Uuid::new_v4());
        let old = original.data.join("book.zim").to_string_lossy().into_owned();
        let current = moved.data.join("book.zim").to_string_lossy().into_owned();
        let mut jobs = json!({"jobs": [{
            "staging": original.data.join("torrents/staging"),
            "directory": original.data,
            "metadata": original.data.join("torrents/metadata.torrent"),
            "job": {"id": id, "destination": original.data, "completedPaths": [old]},
            "directoryIdentity": {"device": 123, "inode": 456},
            "publicationIntents": [{"fileName": "book.zim", "identity": {"device": 789, "inode": 987}}]
        }]});
        fs::create_dir(moved.data.join("torrents")).unwrap();
        fs::write(moved.data.join("torrents/jobs.json"), serde_json::to_vec(&jobs).unwrap()).unwrap();
        fs::copy(original.data.join(".portable-roots.json"), moved.data.join(".portable-roots.json")).unwrap();
        let paths = PortablePaths::initialize(&moved, true).unwrap();
        let evidence = jobs["jobs"][0]["publicationIntents"].clone();
        paths.rebase_json(RecordKind::Torrents, &mut jobs).unwrap();
        assert_eq!(jobs["jobs"][0]["job"]["completedPaths"][0], current);
        assert_eq!(jobs["jobs"][0]["publicationIntents"], evidence);
        assert_eq!(jobs["jobs"][0]["directoryIdentity"], json!({"device": 123, "inode": 456}));
        let mut state = json!({"books": [], "settings": {}, "consumedAcquisitions": [receipt(&id, &old), receipt(&id, &current)]});
        paths.rebase_json(RecordKind::State, &mut state).unwrap();
        assert_eq!(state["consumedAcquisitions"], json!([receipt(&id, &current)]));
        let mut foreign = json!({"path": "/outside/book.zim"});
        assert!(paths.rebase_json(RecordKind::Maps, &mut foreign).is_err());
    }

    #[test]
    fn windows_roots_and_traversal_are_handled_without_host_path_guessing() {
        let paths = PortablePaths {
            version: 1, owner: uuid::Uuid::new_v4().to_string(),
            roots: vec![r"C:\Kiwix\data".into(), "/new/data".into()],
            current: PathBuf::from("/new/data"), receipts: HashMap::new(),
        };
        assert_eq!(paths.relative(r"C:\Kiwix\data\book.zim").unwrap(), Some(vec!["book.zim".into()]));
        assert!(paths.relative(r"C:\Kiwix\data\..\outside.zim").is_err());
        assert_eq!(paths.relative(r"C:\Other\book.zim").unwrap(), None);
    }
}

#[cfg(test)]
mod migration_source_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn explicit_external_source_is_preserved_but_owned_migration_target_is_confined() {
        let paths = PortablePaths {
            version: 1, owner: uuid::Uuid::new_v4().to_string(),
            roots: vec!["/old/data".into(), "/new/data".into()],
            current: PathBuf::from("/new/data"), receipts: HashMap::new(),
        };
        let evidence = json!({"device": 31, "inode": 47});
        let manifest = json!([{"name": "book.zim", "source": {"device": 59, "inode": 61}, "sha256": "retained-hash", "size": 83}]);
        let mut journal = json!({
            "source_directory": "/user-chosen/downloads",
            "destination_directory": "/old/data",
            "source_directory_identity": evidence,
            "destination_directory_identity": evidence,
            "parts": manifest
        });
        assert!(paths.rebase_json(RecordKind::Migration, &mut journal).unwrap());
        assert_eq!(journal["source_directory"], "/user-chosen/downloads");
        assert_eq!(Path::new(journal["destination_directory"].as_str().unwrap()), Path::new("/new/data"));
        assert_eq!(journal["source_directory_identity"], evidence);
        assert_eq!(journal["destination_directory_identity"], evidence);
        assert_eq!(journal["parts"], manifest);
        let mut foreign_target = json!({
            "source_directory": "/user-chosen/downloads",
            "destination_directory": "/another/external-folder"
        });
        assert!(paths.rebase_json(RecordKind::Migration, &mut foreign_target).is_err());
    }
}
