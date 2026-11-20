// SPDX-License-Identifier: GPL-3.0-or-later
//! macOS App Sandbox: keep user-granted folder access across launches.
//!
//! Inside the App Sandbox the app can open something outside its container
//! only while a user grant for it is live: an open/save panel, drag and drop,
//! or a Finder "Open With" in the current session. Kiwix reads every archive
//! through its folder (split parts are listed there, change detection reopens
//! it, delete stages next to it), so grants are kept per *folder*: the folder
//! of each in-place archive, watched folders, the download folder, and the
//! folders that saved edition updates still point at.
//!
//! Each such folder is recorded as a security-scoped bookmark while its grant
//! is live (`grant`, called by the store before it commits), bookmarks for
//! folders the committed profile no longer needs are dropped (`release`,
//! called after the commit), and every recorded bookmark is resolved and
//! activated at the next launch before the profile is loaded (`restore`).
//!
//! Outside the sandbox (other platforms, and unsigned development builds on
//! macOS) `restore` is never called and the other entry points do nothing.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const FILE_NAME: &str = "security-scoped-bookmarks.json";
const FORMAT_VERSION: u32 = 2;
/// Earlier download folders stay granted after the setting changes, because
/// paused transfers and interrupted moves may still name them. Keep this many.
const MAX_RETAINED: usize = 8;

/// True when this process runs inside the macOS App Sandbox.
pub fn is_sandboxed() -> bool {
    cfg!(target_os = "macos") && std::env::var_os("APP_SANDBOX_CONTAINER_ID").is_some()
}

/// The user's real Downloads folder. Inside the sandbox `$HOME` is the
/// container, whose `Downloads` is only a symbolic link to this folder, and a
/// link cannot be opened as a directory without following it.
pub(crate) fn real_downloads_dir() -> Option<PathBuf> {
    #[cfg(not(target_os = "macos"))]
    return None;
    #[cfg(target_os = "macos")]
    {
        let home = nix::unistd::User::from_uid(nix::unistd::Uid::current()).ok().flatten()?.dir;
        home.is_absolute().then(|| home.join("Downloads"))
    }
}

/// A folder the profile needs across launches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Root {
    pub path: PathBuf,
    /// Keep the grant for a while after the profile stops naming the folder.
    pub retain: bool,
}

impl Root {
    pub(crate) fn folder(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into(), retain: false }
    }
    /// The folder that holds an archive or another file.
    pub(crate) fn parent_of(path: &str) -> Option<Self> {
        Path::new(path).parent().filter(|parent| !parent.as_os_str().is_empty()).map(Self::folder)
    }
}

/// Native bookmark operations, separated so the bookkeeping is testable.
pub(crate) trait Bookmarks {
    /// Cheap local check that the process can open the folder now, so folders
    /// that are not granted do not cost a bookmark request on every save.
    fn accessible(&self, path: &Path) -> bool;
    /// Create a security-scoped bookmark for a path the process can access now.
    fn create(&self, path: &Path) -> Result<Vec<u8>, String>;
    /// Resolve a bookmark and start accessing it for the rest of the process.
    fn activate(&self, bookmark: &[u8]) -> Result<Activated, String>;
}

pub(crate) struct Activated {
    pub path: PathBuf,
    /// A replacement bookmark when the system reported the old one as stale.
    pub refreshed: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Saved {
    version: u32,
    /// Folder path, as the profile names it -> its grant.
    roots: BTreeMap<String, Grant>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Grant {
    /// Standard base64 of the opaque bookmark bytes.
    bookmark: String,
    #[serde(default)]
    retain: bool,
    /// Seconds since the epoch when a retained folder stopped being named.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    released_at: Option<u64>,
}

pub(crate) struct Registry {
    file: PathBuf,
    /// Paths inside the container need no bookmark.
    container: Option<PathBuf>,
    /// Persistent authorization already supplied by the Downloads entitlement.
    entitled_downloads: Option<PathBuf>,
    saved: Saved,
    /// Bookmarks that failed to activate at launch. They are replaced as soon
    /// as the user grants the folder again.
    dead: BTreeSet<String>,
    /// Folders whose bookmark creation failure was already logged.
    reported: BTreeSet<String>,
    /// `restore` changed bookmarks that are written with the next save, once
    /// the profile is known to be writable and owned by this process.
    dirty: bool,
}

static REGISTRY: Mutex<Option<Registry>> = Mutex::new(None);

/// Activate every saved bookmark for this profile. Call once, before the store
/// opens. Only call it when `is_sandboxed()`. Writes nothing.
pub fn restore(data_dir: &Path) {
    #[cfg(target_os = "macos")]
    {
        let container = std::env::var_os("HOME").map(PathBuf::from);
        let registry = Registry::restore(data_dir, container, &native::Native);
        let mut slot = REGISTRY.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        *slot = Some(registry);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = data_dir;
}

/// Record a bookmark for every root that has none yet (or whose bookmark is
/// dead) while the user's grant is still live. Call before committing the
/// profile that names these roots. Does nothing outside the sandbox.
pub(crate) fn grant(data_dir: &Path, roots: &[Root]) {
    #[cfg(target_os = "macos")]
    with_registry(data_dir, |registry| registry.grant(roots, &native::Native));
    #[cfg(not(target_os = "macos"))]
    let _ = (data_dir, roots);
}

/// Drop bookmarks the committed profile no longer needs. Call only after the
/// profile naming `roots` was committed.
pub(crate) fn release(data_dir: &Path, roots: &[Root]) {
    #[cfg(target_os = "macos")]
    with_registry(data_dir, |registry| registry.release(roots, now()));
    #[cfg(not(target_os = "macos"))]
    let _ = (data_dir, roots);
}

#[cfg(target_os = "macos")]
fn with_registry(data_dir: &Path, work: impl FnOnce(&mut Registry)) {
    // A poisoned lock only means an earlier caller panicked; the registry is
    // still consistent because each mutation is followed by its own save.
    let mut slot = REGISTRY.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(registry) = slot.as_mut() else { return };
    if !registry.serves(data_dir) {
        // A different directory is being written (for example a migration
        // target); its grants belong to whichever profile opens it.
        return;
    }
    work(registry);
}

#[cfg(target_os = "macos")]
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

/// The folders of `paths` that the sandbox does not let the app open right
/// now, so the user can be asked to grant them. Empty outside the sandbox.
pub(crate) fn folders_needing_access(paths: impl IntoIterator<Item = PathBuf>) -> Vec<String> {
    if !is_sandboxed() {
        return Vec::new();
    }
    let mut folders = BTreeSet::new();
    for path in paths {
        // A chosen or dropped folder is granted itself; only files need their parent.
        if path.is_dir() {
            continue;
        }
        if let Some(parent) = path.parent().filter(|parent| parent.is_absolute()) {
            if crate::storage::open_directory_nofollow(parent).is_err_and(|error| is_denied(&error)) {
                folders.insert(parent.to_string_lossy().into_owned());
            }
        }
    }
    folders.into_iter().collect()
}

/// The sandbox refused the operation (as opposed to the path being missing,
/// a link, or otherwise invalid).
pub(crate) fn is_denied(error: &std::io::Error) -> bool {
    matches!(error.raw_os_error(), Some(code) if code == libc::EPERM || code == libc::EACCES)
}

/// Explain a sandbox refusal to open a folder in terms the user can act on.
pub(crate) fn explain_folder_error(path: &Path, error: &std::io::Error) -> Option<String> {
    (is_sandboxed() && is_denied(error)).then(|| format!(
        "Kiwix does not have access to the folder {}. Add that folder to the library, or add the archive again and allow access to its folder.",
        path.display()
    ))
}

/// A private directory on the destination's volume where a sandboxed app may
/// prepare a file before moving it over a save-panel destination: the panel
/// grants the chosen file, not its folder, so a temporary sibling cannot be
/// created. `None` outside the sandbox or when the system offers none.
pub(crate) fn replacement_directory(destination: &Path) -> Option<PathBuf> {
    if !is_sandboxed() {
        return None;
    }
    #[cfg(target_os = "macos")]
    return native::replacement_directory(destination);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = destination;
        None
    }
}

impl Registry {
    pub(crate) fn restore(data_dir: &Path, container: Option<PathBuf>, native: &dyn Bookmarks) -> Self {
        let file = data_dir.join(FILE_NAME);
        let mut saved = match std::fs::read(&file) {
            Ok(bytes) => match serde_json::from_slice::<Saved>(&bytes) {
                Ok(saved) if saved.version == FORMAT_VERSION => saved,
                Ok(_) | Err(_) => {
                    eprintln!("Kiwix: ignoring unreadable {FILE_NAME}; folders must be chosen again");
                    Saved::default()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Saved::default(),
            Err(error) => {
                eprintln!("Kiwix: cannot read {FILE_NAME}: {error}");
                Saved::default()
            }
        };
        saved.version = FORMAT_VERSION;
        let entitled_downloads = real_downloads_dir();
        let before = saved.roots.len();
        saved.roots.retain(|root, _| Some(Path::new(root)) != entitled_downloads.as_deref());
        let mut dirty = saved.roots.len() != before;
        let mut dead = BTreeSet::new();
        for (root, grant) in saved.roots.iter_mut() {
            let activated = decode(&grant.bookmark)
                .ok_or_else(|| "The saved bookmark is corrupt".to_owned())
                .and_then(|bytes| native.activate(&bytes));
            match activated {
                Ok(activated) => {
                    if let Some(fresh) = activated.refreshed {
                        grant.bookmark = encode(&fresh);
                        dirty = true;
                    }
                    if activated.path != Path::new(root) {
                        // Access follows a moved folder, but the profile still
                        // names the old path, which then shows as missing.
                        eprintln!("Kiwix: {root} has moved to {}", activated.path.display());
                    }
                }
                // Keep the bookmark (an unmounted volume may return later), but
                // replace it if the user grants the folder again.
                Err(error) => {
                    eprintln!("Kiwix: cannot reopen {root}: {error}");
                    dead.insert(root.clone());
                }
            }
        }
        Self { file, container, entitled_downloads, saved, dead, reported: BTreeSet::new(), dirty }
    }

    pub(crate) fn grant(&mut self, roots: &[Root], native: &dyn Bookmarks) {
        let mut changed = false;
        for root in self.relevant(roots) {
            let key = root.path.to_string_lossy().into_owned();
            if !self.dead.contains(&key) {
                if let Some(grant) = self.saved.roots.get_mut(&key) {
                    if grant.retain != root.retain || grant.released_at.is_some() {
                        grant.retain = root.retain;
                        grant.released_at = None;
                        changed = true;
                    }
                    continue;
                }
            }
            // Retried on every save, because the user may grant the folder at
            // any time; the access check keeps ungranted folders cheap.
            if !native.accessible(&root.path) {
                continue;
            }
            match native.create(&root.path) {
                Ok(bytes) => {
                    self.saved.roots.insert(key.clone(), Grant { bookmark: encode(&bytes), retain: root.retain, released_at: None });
                    self.dead.remove(&key);
                    self.reported.remove(&key);
                    changed = true;
                }
                Err(error) => {
                    if self.reported.insert(key) {
                        eprintln!("Kiwix: cannot keep access to {}: {error}", root.path.display());
                    }
                }
            }
        }
        if changed || self.dirty {
            self.save();
        }
    }

    pub(crate) fn release(&mut self, roots: &[Root], now: u64) {
        let referenced: BTreeSet<String> =
            self.relevant(roots).into_iter().map(|root| root.path.to_string_lossy().into_owned()).collect();
        let mut changed = false;
        self.saved.roots.retain(|root, grant| {
            if referenced.contains(root) {
                return true;
            }
            if grant.retain {
                if grant.released_at.is_none() {
                    grant.released_at = Some(now);
                    changed = true;
                }
                return true;
            }
            changed = true;
            false
        });
        let mut released: Vec<(u64, String)> = self
            .saved
            .roots
            .iter()
            .filter_map(|(root, grant)| grant.released_at.map(|at| (at, root.clone())))
            .collect();
        if released.len() > MAX_RETAINED {
            released.sort();
            for (_, root) in &released[..released.len() - MAX_RETAINED] {
                self.saved.roots.remove(root);
            }
            changed = true;
        }
        let saved = &self.saved;
        self.dead.retain(|root| saved.roots.contains_key(root));
        if changed || self.dirty {
            self.save();
        }
    }

    /// Absolute roots outside the container, once each. A folder named both
    /// as an archive folder and as a download folder is retained.
    fn relevant(&self, roots: &[Root]) -> Vec<Root> {
        let mut merged: BTreeMap<&Path, bool> = BTreeMap::new();
        for root in roots {
            if root.path.is_absolute()
                && !self.container.as_deref().is_some_and(|home| root.path.starts_with(home))
                && Some(root.path.as_path()) != self.entitled_downloads.as_deref()
            {
                *merged.entry(root.path.as_path()).or_default() |= root.retain;
            }
        }
        merged.into_iter().map(|(path, retain)| Root { path: path.to_path_buf(), retain }).collect()
    }

    /// The store canonicalizes its directory while startup may pass the
    /// unresolved path, so compare canonical forms when they differ.
    pub(crate) fn serves(&self, data_dir: &Path) -> bool {
        let Some(own) = self.file.parent() else { return false };
        own == data_dir
            || matches!(
                (std::fs::canonicalize(own), std::fs::canonicalize(data_dir)),
                (Ok(a), Ok(b)) if a == b
            )
    }

    fn save(&mut self) {
        let result = serde_json::to_vec_pretty(&self.saved)
            .map_err(|error| error.to_string())
            .and_then(|bytes| write_atomically(&self.file, &bytes));
        match result {
            Ok(()) => self.dirty = false,
            // Keep the in-memory grant pending so a later profile save retries.
            Err(error) => {
                self.dirty = true;
                eprintln!("Kiwix: cannot save {FILE_NAME}: {error}");
            }
        }
    }
}

fn encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn decode(text: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(text).ok()
}

fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let name = path.file_name().and_then(|name| name.to_str()).unwrap_or(FILE_NAME);
    let temporary = path.with_file_name(format!(".{name}.{}.tmp", uuid::Uuid::new_v4()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options
        .open(&temporary)
        .and_then(|mut file| file.write_all(bytes).and_then(|()| file.sync_all()))
        .and_then(|()| std::fs::rename(&temporary, path));
    if let Err(error) = written {
        let _ = std::fs::remove_file(&temporary);
        return Err(error.to_string());
    }
    if let Some(parent) = path.parent() {
        if let Ok(directory) = std::fs::File::open(parent) {
            let _ = directory.sync_all();
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
mod native {
    use super::{Activated, Bookmarks};
    use objc2::rc::autoreleasepool;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSData, NSString, NSURLBookmarkCreationOptions, NSURLBookmarkResolutionOptions, NSURL};
    use std::path::{Path, PathBuf};

    pub(super) struct Native;

    pub(super) fn replacement_directory(destination: &Path) -> Option<PathBuf> {
        use objc2_foundation::{NSFileManager, NSSearchPathDirectory, NSSearchPathDomainMask};
        let destination = destination.to_str()?;
        autoreleasepool(|_| {
            let url = NSURL::fileURLWithPath(&NSString::from_str(destination));
            let directory = NSFileManager::defaultManager()
                .URLForDirectory_inDomain_appropriateForURL_create_error(
                    NSSearchPathDirectory::ItemReplacementDirectory,
                    NSSearchPathDomainMask::UserDomainMask,
                    Some(&url),
                    true,
                )
                .ok()?;
            directory.path().map(|path| PathBuf::from(path.to_string()))
        })
    }

    // These run on store and worker threads, which have no autorelease pool.
    impl Bookmarks for Native {
        fn accessible(&self, path: &Path) -> bool {
            crate::storage::open_directory_nofollow(path).is_ok()
        }

        fn create(&self, path: &Path) -> Result<Vec<u8>, String> {
            let path = path.to_str().ok_or("The path is not valid UTF-8")?;
            autoreleasepool(|_| {
                let url = NSURL::fileURLWithPath(&NSString::from_str(path));
                let data = url
                    .bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
                        NSURLBookmarkCreationOptions::WithSecurityScope,
                        None,
                        None,
                    )
                    .map_err(|error| error.localizedDescription().to_string())?;
                Ok(data.to_vec())
            })
        }

        fn activate(&self, bookmark: &[u8]) -> Result<Activated, String> {
            autoreleasepool(|_| {
                let data = NSData::with_bytes(bookmark);
                let mut stale = Bool::NO;
                // SAFETY: `stale` is a valid, writable Bool for the duration of the call.
                let url = unsafe {
                    NSURL::URLByResolvingBookmarkData_options_relativeToURL_bookmarkDataIsStale_error(
                        &data,
                        NSURLBookmarkResolutionOptions::WithSecurityScope
                            | NSURLBookmarkResolutionOptions::WithoutUI,
                        None,
                        &mut stale,
                    )
                }
                .map_err(|error| error.localizedDescription().to_string())?;
                // SAFETY: plain Objective-C message to a valid NSURL.
                if !unsafe { url.startAccessingSecurityScopedResource() } {
                    return Err("The system refused access".into());
                }
                let path = url.path().ok_or("The bookmark does not name a file")?.to_string();
                let refreshed = if stale.as_bool() {
                    url.bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
                        NSURLBookmarkCreationOptions::WithSecurityScope,
                        None,
                        None,
                    )
                    .ok()
                    .map(|data| data.to_vec())
                } else {
                    None
                };
                // Access lasts for the life of the process; never stop it.
                std::mem::forget(url);
                Ok(Activated { path: PathBuf::from(path), refreshed })
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Fake {
        accessible: RefCell<BTreeSet<PathBuf>>,
        created: RefCell<Vec<PathBuf>>,
        activated: RefCell<Vec<Vec<u8>>>,
        stale: bool,
        broken: bool,
        forbidden: Option<PathBuf>,
    }

    impl Fake {
        fn granting(paths: &[&PathBuf]) -> Self {
            Self { accessible: RefCell::new(paths.iter().map(|path| (*path).clone()).collect()), ..Self::default() }
        }
    }

    impl Bookmarks for Fake {
        fn accessible(&self, path: &Path) -> bool {
            assert_ne!(self.forbidden.as_deref(), Some(path), "unexpected entitlement-root probe");
            self.accessible.borrow().contains(path)
        }
        fn create(&self, path: &Path) -> Result<Vec<u8>, String> {
            assert_ne!(self.forbidden.as_deref(), Some(path), "unexpected entitlement-root creation");
            self.created.borrow_mut().push(path.to_path_buf());
            if self.accessible.borrow().contains(path) {
                Ok(format!("bm:{}", path.display()).into_bytes())
            } else {
                Err("no grant".into())
            }
        }
        fn activate(&self, bookmark: &[u8]) -> Result<Activated, String> {
            self.activated.borrow_mut().push(bookmark.to_vec());
            if self.broken {
                return Err("volume missing".into());
            }
            let text = String::from_utf8(bookmark.to_vec()).unwrap();
            let path = PathBuf::from(text.strip_prefix("bm:").unwrap());
            assert_ne!(self.forbidden.as_deref(), Some(path.as_path()), "unexpected entitlement-root activation");
            Ok(Activated { path, refreshed: self.stale.then(|| b"bm:fresh".to_vec()) })
        }
    }

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kiwix-sandbox-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn saved_roots(profile: &Path) -> Vec<String> {
        let saved: Saved = serde_json::from_slice(&std::fs::read(profile.join(FILE_NAME)).unwrap()).unwrap();
        saved.roots.keys().cloned().collect()
    }

    /// A fake absolute folder (never touched on disk). The registry ignores
    /// relative roots, and a POSIX path has no drive, so it is relative on Windows.
    fn abs(path: &str) -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(format!("C:{}", path.replace('/', "\\")))
        } else {
            PathBuf::from(path)
        }
    }

    fn key(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn records_new_roots_restores_them_and_prunes_only_after_commit() {
        let profile = temp();
        let volume = abs("/Volumes/Data/ZIMs");
        let folder = abs("/Users/me/ZIMs");
        let fake = Fake::granting(&[&volume, &folder]);
        let mut registry = Registry::restore(&profile, None, &fake);
        let roots = [Root::folder(&volume), Root::folder(&folder), Root::folder(&volume), Root::folder("relative")];
        registry.grant(&roots, &fake);
        assert_eq!(*fake.created.borrow(), vec![folder.clone(), volume.clone()]);
        // Already recorded roots are not re-created on later saves.
        registry.grant(&roots, &fake);
        assert_eq!(fake.created.borrow().len(), 2);

        let next = Fake::default();
        let mut restored = Registry::restore(&profile, None, &next);
        assert_eq!(next.activated.borrow().len(), 2);
        // Nothing is pruned until the profile without the volume is committed.
        restored.grant(&[Root::folder(&folder)], &next);
        assert_eq!(saved_roots(&profile).len(), 2);
        restored.release(&[Root::folder(&folder)], 3);
        assert_eq!(saved_roots(&profile), vec![key(&folder)]);
        std::fs::remove_dir_all(profile).unwrap();
    }

    #[test]
    fn failed_registry_write_is_retried_and_restored_after_repair() {
        let profile = temp();
        let folder = abs("/Volumes/Data/ZIMs");
        let roots = [Root::folder(&folder)];
        let fake = Fake::granting(&[&folder]);
        let mut registry = Registry::restore(&profile, None, &fake);
        std::fs::create_dir(profile.join(FILE_NAME)).unwrap();
        registry.grant(&roots, &fake);
        std::fs::remove_dir(profile.join(FILE_NAME)).unwrap();
        registry.grant(&roots, &fake);
        assert_eq!(saved_roots(&profile), vec![key(&folder)]);
        let restored = Fake::default();
        Registry::restore(&profile, None, &restored);
        assert_eq!(*restored.activated.borrow(), vec![format!("bm:{}", folder.display()).into_bytes()]);
        std::fs::remove_dir_all(profile).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn entitled_downloads_needs_no_bookmark_but_custom_folders_still_restore() {
        let profile = temp();
        let downloads = real_downloads_dir().unwrap();
        let custom = abs("/Volumes/Custom/Downloads");
        let roots = [Root::folder(downloads.join("././")), Root::folder(&custom)];
        let fake = Fake { forbidden: Some(downloads.clone()), ..Fake::granting(&[&custom]) };
        let mut registry = Registry::restore(&profile, None, &fake);
        registry.grant(&roots, &fake);
        assert_eq!(saved_roots(&profile), vec![key(&custom)]);
        // Simulate a registry written before entitlement roots were excluded.
        registry.saved.roots.insert(key(&downloads), Grant {
            bookmark: encode(format!("bm:{}", downloads.display()).as_bytes()),
            retain: true, released_at: None,
        });
        registry.save();
        let before = std::fs::read(profile.join(FILE_NAME)).unwrap();
        let restored = Fake { forbidden: Some(downloads), ..Fake::default() };
        let mut registry = Registry::restore(&profile, None, &restored);
        assert_eq!(std::fs::read(profile.join(FILE_NAME)).unwrap(), before);
        assert_eq!(*restored.activated.borrow(), vec![format!("bm:{}", custom.display()).into_bytes()]);
        registry.grant(&roots, &restored);
        assert_eq!(saved_roots(&profile), vec![key(&custom)]);
        std::fs::remove_dir_all(profile).unwrap();
    }

    #[test]
    fn archive_roots_are_their_folders() {
        assert_eq!(Root::parent_of("/Volumes/Data/文书.zim"), Some(Root::folder("/Volumes/Data")));
        assert_eq!(Root::parent_of("日本語.zim"), None);
        assert_eq!(Root::parent_of("/"), None);
    }

    #[test]
    fn serves_the_same_profile_through_another_spelling_only() {
        let base = temp();
        let profile = base.join("profile");
        std::fs::create_dir_all(&profile).unwrap();
        let registry = Registry::restore(&profile, None, &Fake::default());
        assert!(registry.serves(&profile));
        assert!(registry.serves(&base.join("profile/../profile")));
        assert!(!registry.serves(&base));
        assert!(!registry.serves(&base.join("missing")));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn ungranted_roots_are_retried_until_the_user_grants_them() {
        let profile = temp();
        let container = abs("/Users/me/Library/Containers/org.kiwix/Data");
        let typed = abs("/Volumes/Other/Downloads");
        let fake = Fake::default();
        let mut registry = Registry::restore(&profile, Some(container.clone()), &fake);
        let roots = [Root::folder(container.join("Downloads")), Root::folder(&typed)];
        registry.grant(&roots, &fake);
        registry.grant(&roots, &fake);
        // The container needs no bookmark, and an inaccessible folder costs no
        // bookmark request.
        assert!(fake.created.borrow().is_empty());
        assert!(!profile.join(FILE_NAME).exists());
        // The user then picks the same folder in a panel.
        fake.accessible.borrow_mut().insert(typed.clone());
        registry.grant(&roots, &fake);
        assert_eq!(saved_roots(&profile), vec![key(&typed)]);
        std::fs::remove_dir_all(profile).unwrap();
    }

    #[test]
    fn dead_bookmarks_are_replaced_when_the_folder_is_granted_again() {
        let profile = temp();
        let folder = abs("/Volumes/Gone/ZIMs");
        let fake = Fake::granting(&[&folder]);
        Registry::restore(&profile, None, &fake).grant(&[Root::folder(&folder)], &fake);
        let broken = Fake { broken: true, ..Fake::granting(&[&folder]) };
        let mut registry = Registry::restore(&profile, None, &broken);
        assert!(registry.dead.contains(&key(&folder)));
        registry.grant(&[Root::folder(&folder)], &broken);
        assert_eq!(*broken.created.borrow(), vec![folder.clone()]);
        assert!(registry.dead.is_empty());
        // Further saves keep the fresh bookmark.
        registry.grant(&[Root::folder(&folder)], &broken);
        assert_eq!(broken.created.borrow().len(), 1);
        std::fs::remove_dir_all(profile).unwrap();
    }

    #[test]
    fn earlier_download_folders_are_retained_up_to_a_limit() {
        let profile = temp();
        let folders: Vec<PathBuf> = (0..MAX_RETAINED + 2).map(|n| abs(&format!("/Volumes/D{n}"))).collect();
        let fake = Fake::granting(&folders.iter().collect::<Vec<_>>());
        let mut registry = Registry::restore(&profile, None, &fake);
        for (when, folder) in folders.iter().enumerate() {
            let roots = [Root { path: folder.clone(), retain: true }];
            registry.grant(&roots, &fake);
            registry.release(&roots, when as u64);
        }
        let kept = saved_roots(&profile);
        // The current folder plus the most recent MAX_RETAINED earlier ones.
        assert_eq!(kept.len(), MAX_RETAINED + 1);
        assert!(!kept.contains(&key(&folders[0])));
        assert!(kept.contains(&key(&folders[1])));
        // Naming a retained folder again makes it current, not released.
        let again = [Root { path: folders[5].clone(), retain: true }];
        registry.grant(&again, &fake);
        assert_eq!(registry.saved.roots[&key(&folders[5])].released_at, None);
        // Ordinary folders are dropped as soon as the committed profile stops naming them.
        let ordinary = abs("/Volumes/Ordinary");
        fake.accessible.borrow_mut().insert(ordinary.clone());
        registry.grant(&[Root::folder(&ordinary)], &fake);
        assert!(saved_roots(&profile).contains(&key(&ordinary)));
        registry.release(&again, 101);
        assert!(!saved_roots(&profile).contains(&key(&ordinary)));
        std::fs::remove_dir_all(profile).unwrap();
    }

    #[test]
    fn a_download_folder_that_also_holds_archives_is_retained() {
        let profile = temp();
        let first = abs("/Volumes/Y/D1");
        let second = abs("/Volumes/Y/D2");
        let fake = Fake::granting(&[&first, &second]);
        let mut registry = Registry::restore(&profile, None, &fake);
        for (when, folder) in [&first, &second].into_iter().enumerate() {
            // Archive folder first, download folder second, as the store lists them.
            let roots = [Root::folder(folder), Root { path: folder.clone(), retain: true }];
            registry.grant(&roots, &fake);
            registry.release(&roots, when as u64);
        }
        assert_eq!(saved_roots(&profile), vec![key(&first), key(&second)]);
        assert!(registry.saved.roots[&key(&first)].retain);
        std::fs::remove_dir_all(profile).unwrap();
    }

    #[test]
    fn restore_writes_nothing_and_stale_bookmarks_are_saved_with_the_next_grant() {
        let profile = temp();
        let folder = abs("/Volumes/Data/ZIMs");
        let fake = Fake::granting(&[&folder]);
        Registry::restore(&profile, None, &fake).grant(&[Root::folder(&folder)], &fake);
        let before = std::fs::read(profile.join(FILE_NAME)).unwrap();
        let stale = Fake { stale: true, ..Fake::default() };
        let mut restored = Registry::restore(&profile, None, &stale);
        assert_eq!(std::fs::read(profile.join(FILE_NAME)).unwrap(), before);
        restored.grant(&[Root::folder(&folder)], &stale);
        let saved: Saved = serde_json::from_slice(&std::fs::read(profile.join(FILE_NAME)).unwrap()).unwrap();
        assert_eq!(saved.roots.values().next().unwrap().bookmark, encode(b"bm:fresh"));

        std::fs::write(profile.join(FILE_NAME), b"not json").unwrap();
        let empty = Registry::restore(&profile, None, &Fake::default());
        assert!(empty.saved.roots.is_empty());
        let leftovers: Vec<_> = std::fs::read_dir(&profile).unwrap().filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp")).collect();
        assert!(leftovers.is_empty());
        std::fs::remove_dir_all(profile).unwrap();
    }
}
