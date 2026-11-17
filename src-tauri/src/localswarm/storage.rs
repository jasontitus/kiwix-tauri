// SPDX-License-Identifier: GPL-3.0-or-later
//! Capability-bound private staging. Persisted names never grant ownership.
use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use cap_std::fs::{Dir, DirBuilder, Metadata, OpenOptions};
use crate::fs_ext::FileExt;
use serde::{Deserialize, Serialize};
use std::{fs::File, io::{Read, Seek, SeekFrom, Write}, path::{Component, Path, PathBuf}, time::UNIX_EPOCH};
use super::protocol::Manifest;

pub(super) fn err(e: impl std::fmt::Display) -> String { e.to_string() }
/// Benchmark-only storage latency: `KIWIX_LOCALSWARM_TEST_SYNC_DELAY_MS`
/// simulates a phone fsync so durability cadence can be measured on a fast
/// desktop. Compiled only into the lib test binary.
fn test_sync_delay() {
    #[cfg(test)]
    {
        use std::sync::LazyLock;
        static DELAY: LazyLock<Option<std::time::Duration>> = LazyLock::new(|| {
            std::env::var("KIWIX_LOCALSWARM_TEST_SYNC_DELAY_MS").ok()
                .and_then(|value| value.parse::<u64>().ok()).map(std::time::Duration::from_millis)
        });
        if let Some(delay) = *DELAY { std::thread::sleep(delay); }
    }
}
#[derive(Debug)]
pub(super) struct PersistError { pub message: String, pub committed: bool }
impl std::fmt::Display for PersistError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { self.message.fmt(formatter) }
}
impl From<PersistError> for String {
    fn from(error: PersistError) -> Self { error.message }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Identity { device: u64, inode: u64 }
impl Identity {
    pub fn of(m: &Metadata) -> Self { Self { device: m.dev(), inode: m.ino() } }
    #[cfg(target_os = "ios")]
    pub fn backup_identity(&self) -> (u64, u64) { (self.device, self.inode) }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Stamp { pub identity: Identity, pub size: u64, modified: u128 }
impl Stamp {
    pub fn of(m: &Metadata) -> Result<Self, String> {
        if !m.is_file() || m.is_symlink() || m.nlink() != 1 { return Err("LocalSwarm requires regular files without hard links".into()); }
        Self::read_only_source(m)
    }
    pub fn read_only_source(m: &Metadata) -> Result<Self, String> {
        if !m.is_file() || m.is_symlink() { return Err("LocalSwarm sources must be regular files".into()); }
        Ok(Self { identity: Identity::of(m), size: m.len(), modified: m.modified().map_err(err)?.into_std().duration_since(UNIX_EPOCH).map_err(err)?.as_nanos() })
    }
}
pub(super) struct Directory { pub path: PathBuf, pub dir: Dir, pub identity: Identity }
impl Directory {
    pub fn open(path: &Path) -> Result<Self, String> {
        let dir = crate::storage::open_directory_nofollow(path).map_err(err)?;
        let identity = Identity::of(&dir.dir_metadata().map_err(err)?);
        Ok(Self { path: path.into(), dir, identity })
    }
    pub fn unchanged(&self) -> Result<(), String> {
        // Check the held capability itself as well as the path: on Windows a
        // detached OpenSet holds a placeholder (a clone of its parent), which
        // the path check alone would not notice.
        if Identity::of(&self.dir.dir_metadata().map_err(err)?) != self.identity
            || Self::open(&self.path)?.identity != self.identity
        {
            return Err("LocalSwarm directory identity changed".into());
        }
        Ok(())
    }
    pub fn child(&self, name: &str, create: bool) -> Result<Self, String> {
        basename(name)?; self.unchanged()?;
        if create { #[cfg_attr(not(unix), allow(unused_mut, reason = "the builder is only configured with a Unix mode"))] let mut builder = DirBuilder::new(); #[cfg(unix)] { use cap_std::fs::DirBuilderExt; builder.mode(0o700); } self.dir.create_dir_with(name, &builder).map_err(err)?; self.sync()?; }
        let dir = self.dir.open_dir_nofollow(name).map_err(err)?;
        #[cfg(unix)] { use std::os::unix::fs::MetadataExt as _; let m = dir.try_clone().map_err(err)?.into_std_file().metadata().map_err(err)?; if m.uid() != crate::fs_ext::effective_uid() || m.mode() & 0o077 != 0 { return Err("LocalSwarm owned directories must be private (0700)".into()); } }
        let identity = Identity::of(&dir.dir_metadata().map_err(err)?);
        Ok(Self { path: self.path.join(name), dir, identity })
    }
    pub fn sync(&self) -> Result<(), String> { #[cfg(unix)] crate::storage::sync_cap_directory(&self.dir).map_err(err)?; Ok(()) }
    pub fn file(&self, name: &str, writable: bool, create: bool) -> Result<File, String> {
        basename(name)?;
        let mut opts = OpenOptions::new(); opts.read(true).write(writable).create_new(create).follow(FollowSymlinks::No);
        #[cfg(unix)] { use cap_std::fs::OpenOptionsExt; opts.mode(0o600); }
        #[cfg(windows)] { use cap_std::fs::OpenOptionsExt; opts.custom_flags(0x0020_0000).share_mode(1 | 4); }
        let file = self.dir.open_with(name, &opts).map_err(err)?;
        Stamp::of(&file.metadata().map_err(err)?)?;
        if create { crate::mobile_backup::exclude_payload(&file, &self.path.join(name))?; }
        let file = file.into_std();
        if writable { FileExt::try_flock_exclusive(&file).map_err(|e| format!("LocalSwarm file is busy: {e}"))?; } else { FileExt::try_flock_shared(&file).map_err(|e| format!("LocalSwarm source is busy: {e}"))?; }
        Ok(file)
    }
    /// Open an existing regular file for reading. A reader does not need link
    /// ownership: during an atomic replace it may open the old inode, whose
    /// link count has already dropped to zero. Write/create paths still use
    /// the strict `file` above.
    pub(super) fn read_file(&self, name: &str) -> Result<File, String> {
        basename(name)?;
        let mut opts = OpenOptions::new();
        opts.read(true).follow(FollowSymlinks::No);
        #[cfg(windows)]
        {
            use cap_std::fs::OpenOptionsExt;
            // Match the strict read-only opener: allow concurrent readers and
            // atomic replacement (DELETE), but deny an in-place writer.
            opts.custom_flags(0x0020_0000).share_mode(1 | 4);
        }
        let file = self.dir.open_with(name, &opts).map_err(err)?;
        Stamp::read_only_source(&file.metadata().map_err(err)?)?;
        Ok(file.into_std())
    }
    pub fn optional_bytes(&self, name: &str, limit: u64) -> Result<Option<Vec<u8>>, String> {
        basename(name)?;
        match self.dir.symlink_metadata(name) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(err(e)),
            Ok(m) if m.is_symlink() => return Err("LocalSwarm state must not be a symlink".into()),
            Ok(_) => {}
        }
        let file = self.read_file(name)?; let mut bytes = Vec::new(); file.take(limit + 1).read_to_end(&mut bytes).map_err(err)?;
        if bytes.len() as u64 > limit { return Err("LocalSwarm state exceeds its size limit".into()); } Ok(Some(bytes))
    }
    pub fn atomic(&self, name: &str, bytes: &[u8]) -> Result<(), PersistError> {
        let before = |error: String| PersistError { message: error, committed: false };
        self.unchanged().map_err(before)?;
        match self.dir.symlink_metadata(name) { Ok(m) => { Stamp::of(&m).map_err(before)?; }, Err(e) if e.kind() == std::io::ErrorKind::NotFound => (), Err(e) => return Err(before(err(e))) }
        let temp = format!(".state-{}", uuid::Uuid::new_v4());
        let mut file = self.file(&temp, true, true).map_err(before)?;
        file.write_all(bytes).map_err(|e| before(err(e)))?;
        file.sync_all().map_err(|e| before(err(e)))?;
        // Release the exclusive lock before the rename publishes this inode as
        // `name`. A concurrent reader of `name` then cannot open the new file
        // and fail with EWOULDBLOCK in the window between rename and drop.
        drop(file);
        self.unchanged().map_err(before)?;
        self.dir.rename(&temp, &self.dir, name).map_err(|e| before(err(e)))?;
        test_sync_delay();
        self.sync().map_err(|message| PersistError { message, committed: true })
    }
    pub fn absent(&self, name: &str) -> Result<bool, String> {
        basename(name)?;
        match self.dir.symlink_metadata(name) {
            Ok(_) => Ok(false),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
            Err(error) => Err(err(error)),
        }
    }
    /// Capture an owned directory entry before unlinking its private children.
    /// A raced source is retained at the journaled target, never recursively erased.
    pub fn capture_directory(&self, source: &str, target: &str, expected: &Identity) -> Result<(), String> {
        basename(source)?; basename(target)?; self.unchanged()?;
        if self.child(source, false)?.identity != *expected {
            return Err("LocalSwarm cleanup source directory was replaced".into());
        }
        rename_directory_noreplace(&self.dir, source, target, expected)?;
        self.sync()?;
        if self.child(target, false)?.identity != *expected {
            return Err("LocalSwarm cleanup captured a replaced directory; all entries were preserved".into());
        }
        self.unchanged()
    }
    /// Remove the empty owned child directory `name` only if it is still
    /// `expected`. Windows callers must first close their own capability to it
    /// (cap-std directory handles never share delete); identity is then bound
    /// by the DELETE handle that performs the removal.
    #[cfg(windows)]
    pub fn remove_empty_directory(&self, name: &str, expected: &Identity) -> Result<(), String> {
        basename(name)?; self.unchanged()?;
        remove_directory_by_identity(&self.dir, name, expected)
    }
}
pub(super) fn basename(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 240 || name.ends_with(['.', ' ']) || name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*')) || !matches!(Path::new(name).components().next(), Some(Component::Normal(_))) || Path::new(name).components().count() != 1 { return Err("LocalSwarm requires safe single-component filenames".into()); }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && matches!(stem.as_bytes()[3], b'1'..=b'9')) { return Err("LocalSwarm filename is reserved".into()); } Ok(())
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OwnedSet {
    pub parent: PathBuf, pub parent_identity: Identity, pub staging: String, pub destination: String,
    pub identity: Identity, pub files: Vec<Identity>,
}
pub(super) struct OpenSet {
    pub parent: Directory, pub directory: Directory, pub files: Vec<File>, stamps: Vec<Stamp>, pub published: bool, sealed: bool,
    /// Windows publication surrendered `directory.dir` and could not rebind it
    /// by identity; every staged-set operation fails closed until reopened.
    #[cfg(windows)] detached: bool,
}
impl OwnedSet {
    pub fn check_space(parent: &Directory, manifest: &Manifest, remaining: u64, identity: &str, acknowledgement: Option<&str>) -> Result<(), String> {
        parent.unchanged()?;
        let available = crate::fs_ext::available_space(&parent.path).map_err(err)?;
        if available < remaining.saturating_add(1024 * 1024) { return Err("Not enough free space for this LocalSwarm archive".into()); }
        for file in &manifest.files { crate::storage::check_file_size(&parent.path, file.size_bytes)?; }
        crate::disk_warning::check(&parent.path, Some(remaining), identity, acknowledgement)?;
        parent.unchanged()
    }
    pub fn create(parent: Directory, manifest: &Manifest, suffix: &str) -> Result<(Self, OpenSet), String> {
        let available = crate::fs_ext::available_space(&parent.path).map_err(err)?;
        if available < manifest.total_bytes.saturating_add(1024 * 1024) { return Err("Not enough free space for this LocalSwarm archive".into()); }
        for f in &manifest.files { crate::storage::check_file_size(&parent.path, f.size_bytes)?; }
        let staging = format!(".kiwix-localswarm-{suffix}.part"); let destination = format!("localswarm-{}-{suffix}", &manifest.swarm_id[..12]);
        let directory = parent.child(&staging, true)?; let mut files = Vec::new(); let mut identities = Vec::new();
        for entry in &manifest.files { let file = directory.file(&entry.path, true, true)?; file.set_len(entry.size_bytes).map_err(err)?; file.sync_all().map_err(err)?; identities.push(Identity::of(&Metadata::from_file(&file).map_err(err)?)); files.push(file); }
        directory.sync()?;
        let owned = Self { parent: parent.path.clone(), parent_identity: parent.identity.clone(), staging, destination, identity: directory.identity.clone(), files: identities };
        let stamps = files.iter().map(|file| Stamp::of(&Metadata::from_file(file).map_err(err)?)).collect::<Result<Vec<_>, _>>()?;
        Ok((owned, OpenSet { parent, directory, files, stamps, published: false, sealed: false, #[cfg(windows)] detached: false }))
    }
    pub fn open(&self, manifest: &Manifest, publishing: bool) -> Result<OpenSet, String> {
        basename(&self.staging)?; basename(&self.destination)?;
        let parent = Directory::open(&self.parent)?;
        if parent.identity != self.parent_identity { return Err("LocalSwarm destination parent was replaced".into()); }
        let (directory, published) = match parent.dir.symlink_metadata(&self.staging) {
            Ok(_) => (parent.child(&self.staging, false)?, false),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && publishing => (parent.child(&self.destination, false)?, true),
            Err(e) => return Err(err(e)),
        };
        if directory.identity != self.identity || self.files.len() != manifest.files.len() { return Err("LocalSwarm staging ownership changed; retained data was not touched".into()); }
        let names = directory.dir.entries().map_err(err)?.collect::<Result<Vec<_>, _>>().map_err(err)?;
        if names.len() != manifest.files.len() { return Err("Unexpected entries in LocalSwarm staging; retained data was not touched".into()); }
        let mut files = Vec::new(); let mut stamps = Vec::new();
        for (entry, identity) in manifest.files.iter().zip(&self.files) {
            let file = directory.file(&entry.path, !published, false)?; let stamp = Stamp::of(&Metadata::from_file(&file).map_err(err)?)?;
            if &stamp.identity != identity || stamp.size != entry.size_bytes { return Err("LocalSwarm file ownership or size changed".into()); } stamps.push(stamp); files.push(file);
        }
        Ok(OpenSet { parent, directory, files, stamps, published, sealed: published, #[cfg(windows)] detached: false })
    }
    pub fn final_path(&self, first: &str) -> PathBuf { self.parent.join(&self.destination).join(first) }
}
impl OpenSet {
    pub fn unchanged(&self, manifest: &Manifest, owned: &OwnedSet) -> Result<(), String> {
        #[cfg(windows)]
        if self.detached { return Err("LocalSwarm staging capability was released for publication; reopen the staged set".into()); }
        self.parent.unchanged()?; self.directory.unchanged()?;
        if self.files.len() != manifest.files.len() || self.stamps.len() != manifest.files.len() {
            return Err("LocalSwarm staged handles are incomplete".into());
        }
        let count = self.directory.dir.entries().map_err(err)?.try_fold(0usize, |count, entry| entry.map(|_| count + 1)).map_err(err)?;
        if count != manifest.files.len() { return Err("Unexpected LocalSwarm staging entries".into()); }
        for (((entry, id), file), expected) in manifest.files.iter().zip(&owned.files).zip(&self.files).zip(&self.stamps) {
            let handle = Stamp::of(&Metadata::from_file(file).map_err(err)?)?;
            let path = Stamp::of(&self.directory.dir.symlink_metadata(&entry.path).map_err(err)?)?;
            if &handle.identity != id || path != handle || &handle != expected || handle.size != entry.size_bytes { return Err("LocalSwarm staged file changed".into()); }
        } Ok(())
    }
    pub fn read_chunk(&mut self, manifest: &Manifest, index: u32) -> Result<Vec<u8>, String> {
        let (file, offset) = chunk_position(manifest, index)?; let file = self.files.get_mut(file).ok_or("LocalSwarm staged handles are incomplete")?; let mut bytes = vec![0; manifest.chunk_len(index)?]; file.seek(SeekFrom::Start(offset)).map_err(err)?; file.read_exact(&mut bytes).map_err(err)?; Ok(bytes)
    }
    /// Irreversibly release writer authority before uncancellable verification.
    /// Retain read-only identity guards across the handoff; final SHA rehash must
    /// run after this transition, including Windows' close-time mtime update.
    pub fn seal(&mut self, manifest: &Manifest, owned: &OwnedSet) -> Result<(), String> {
        // Capture the final handle state before the identity check that precedes
        // writer retirement (writes refresh stamps, but seal must not depend on
        // the last caller having done so).
        for (file, stamp) in self.files.iter().zip(self.stamps.iter_mut()) {
            *stamp = Stamp::of(&Metadata::from_file(file).map_err(err)?)?;
        }
        self.unchanged(manifest, owned)?;
        if self.sealed { return Ok(()); }
        for file in &self.files { file.sync_all().map_err(err)?; }
        let mut guards = Vec::with_capacity(self.files.len());
        for (entry, expected) in manifest.files.iter().zip(&self.stamps) {
            let mut options = OpenOptions::new();
            options.read(true).follow(FollowSymlinks::No);
            #[cfg(windows)] {
                use cap_std::fs::OpenOptionsExt;
                // Read-only access coexists with the old writer until it closes.
                options.custom_flags(0x0020_0000).share_mode(1 | 2 | 4);
            }
            let guard = self.directory.dir.open_with(&entry.path, &options).map_err(err)?;
            if Stamp::of(&guard.metadata().map_err(err)?)? != *expected {
                return Err("LocalSwarm file changed before writer retirement".into());
            }
            guards.push(guard.into_std());
        }
        self.sealed = true;
        self.files.clear(); // Releases exclusive byte-range locks and all writers.
        let mut files = Vec::with_capacity(manifest.files.len());
        let mut stamps = Vec::with_capacity(manifest.files.len());
        for ((entry, expected), guard) in manifest.files.iter().zip(&self.stamps).zip(&guards) {
            let file = self.directory.file(&entry.path, false, false)?;
            let stamp = Stamp::of(&Metadata::from_file(&file).map_err(err)?)?;
            let retained = Stamp::of(&Metadata::from_file(guard).map_err(err)?)?;
            if stamp.identity != expected.identity || stamp.size != expected.size || stamp != retained {
                return Err("LocalSwarm file identity changed during writer retirement".into());
            }
            files.push(file);
            stamps.push(stamp);
        }
        self.files = files;
        self.stamps = stamps;
        self.unchanged(manifest, owned)
    }
    /// Fresh read-only handles are identity-bound to the retained sealed set.
    /// Do not clone writable handles into an uncancellable native checksum.
    pub fn verification_files(&self, manifest: &Manifest, owned: &OwnedSet) -> Result<Vec<(String, File)>, String> {
        if !self.sealed { return Err("Seal LocalSwarm writers before native verification".into()); }
        self.unchanged(manifest, owned)?;
        let mut files = Vec::with_capacity(manifest.files.len());
        for (entry, expected) in manifest.files.iter().zip(&self.stamps) {
            let mut options = OpenOptions::new();
            options.read(true).follow(FollowSymlinks::No);
            #[cfg(windows)] {
                use cap_std::fs::OpenOptionsExt;
                // Only read-only owners remain; stock libzim's FILE_SHARE_READ
                // and the retained shared byte-range locks can now coexist.
                options.custom_flags(0x0020_0000).share_mode(1 | 4);
            }
            let file = self.directory.dir.open_with(&entry.path, &options).map_err(err)?;
            if Stamp::of(&file.metadata().map_err(err)?)? != *expected {
                return Err("LocalSwarm verification handle no longer identifies the staged bytes".into());
            }
            files.push((entry.path.clone(), file.into_std()));
        }
        self.unchanged(manifest, owned)?;
        Ok(files)
    }
    /// Write one verified chunk at its offset. Durability is batched by
    /// `sync_data`: a crash before the next sync re-downloads at most one
    /// checkpoint window, because the job state only records synced chunks.
    ///
    /// The handle stamp is refreshed here (one fstat) so an in-process reopen or
    /// identity check sees the bytes that were just written; the expensive
    /// staged-path validation is still batched into `commit_data`.
    pub fn write_chunk(&mut self, manifest: &Manifest, index: u32, bytes: &[u8]) -> Result<(), String> {
        if self.published || self.sealed || bytes.len() != manifest.chunk_len(index)? { return Err("Invalid LocalSwarm chunk write".into()); }
        let (file, offset) = chunk_position(manifest, index)?;
        self.files[file].seek(SeekFrom::Start(offset)).map_err(err)?;
        self.files[file].write_all(bytes).map_err(err)?;
        self.stamps[file] = Stamp::of(&Metadata::from_file(&self.files[file]).map_err(err)?)?;
        Ok(())
    }
    /// Flush every staged handle before the checkpoint that records them.
    pub fn sync_data(&mut self) -> Result<(), String> {
        for file in &self.files { file.sync_data().map_err(err)?; }
        test_sync_delay();
        Ok(())
    }
    /// Refresh handle stamps after a durability flush, then validate that every
    /// staged path still names the handle that received the verified bytes.
    /// This is the per-window identity check; it runs before the checkpoint and
    /// therefore before any chunk is recorded as durable.
    pub fn commit_data(&mut self, manifest: &Manifest, owned: &OwnedSet) -> Result<(), String> {
        self.sync_data()?;
        let measured = super::profile::start();
        for (file, stamp) in self.files.iter().zip(self.stamps.iter_mut()) {
            *stamp = Stamp::of(&Metadata::from_file(file).map_err(err)?)?;
        }
        self.unchanged(manifest, owned)?;
        super::profile::record(9, 0, measured);
        Ok(())
    }
    /// Exclusively rename the verified staging directory to its destination,
    /// binding the committed set to `files` (verified handles to `self.files`).
    pub fn publish(&mut self, manifest: &Manifest, owned: &OwnedSet, files: Vec<(PathBuf, crate::publication::VerifiedFile)>) -> Result<(), String> {
        if self.published { return Err("LocalSwarm set is already published".into()); }
        #[cfg(not(windows))]
        {
            let _ = manifest;
            crate::publication::publish_directory_noreplace_in(&self.parent.dir, Path::new(&owned.staging), Path::new(&owned.destination), &self.directory.dir, &files).map_err(err)?;
        }
        #[cfg(windows)]
        {
            if self.detached { return Err("LocalSwarm staging capability was released for publication; reopen the staged set".into()); }
            if !self.sealed { return Err("Seal LocalSwarm writers before publication".into()); }
            // Windows cannot rename a directory while this set holds its
            // capability (cap-std never shares delete) or any member handle
            // (NTFS refuses directory renames with open children). Surrender
            // them; until every handle is rebound by identity the set is
            // detached and fails closed. The parent clone is an inert
            // placeholder that no staged-set operation reaches while detached.
            let placeholder = self.parent.dir.try_clone().map_err(err)?;
            let released = std::mem::replace(&mut self.directory.dir, placeholder);
            self.detached = true;
            self.files.clear();
            match crate::publication::publish_directory_noreplace_in(&self.parent.dir, Path::new(&owned.staging), Path::new(&owned.destination), released, files) {
                // Returned only after the destination and every member were
                // re-verified against the identities recorded at release.
                Ok((published, verified)) => {
                    self.directory.dir = published;
                    self.directory.path = owned.parent.join(&owned.destination);
                    self.rebind_files(manifest)?;
                    drop(verified);
                    self.detached = false;
                }
                Err(error) => {
                    // A refused commit (for example an existing destination)
                    // leaves the set staged: rebind only that exact directory
                    // and the exact sealed members.
                    if let Ok(staged) = self.parent.child(&owned.staging, false) {
                        if staged.identity == self.directory.identity && staged.path == self.directory.path {
                            self.directory.dir = staged.dir;
                            if self.rebind_files(manifest).is_ok() { self.detached = false; }
                        }
                    }
                    return Err(err(error));
                }
            }
        }
        self.directory.path = owned.parent.join(&owned.destination); self.published = true;
        Ok(())
    }
}
#[cfg(windows)]
impl OpenSet {
    /// Reopen the sealed read-only members through the current directory
    /// capability; each must still carry exactly its sealed stamp.
    fn rebind_files(&mut self, manifest: &Manifest) -> Result<(), String> {
        let mut files = Vec::with_capacity(manifest.files.len());
        for (entry, expected) in manifest.files.iter().zip(&self.stamps) {
            let file = self.directory.file(&entry.path, false, false)?;
            if Stamp::of(&Metadata::from_file(&file).map_err(err)?)? != *expected {
                return Err("LocalSwarm file identity changed during publication".into());
            }
            files.push(file);
        }
        self.files = files;
        Ok(())
    }
}
pub(super) fn chunk_position(manifest: &Manifest, index: u32) -> Result<(usize, u64), String> {
    for (i, f) in manifest.files.iter().enumerate() { if index >= f.start_chunk_index && index <= f.end_chunk_index { return Ok((i, u64::from(index - f.start_chunk_index) * u64::from(manifest.chunk_size_bytes))); } } Err("LocalSwarm chunk index is outside the manifest".into())
}

#[cfg(any(target_os = "macos", target_os = "ios", target_os = "linux", target_os = "android"))]
fn rename_directory_noreplace(directory: &Dir, source: &str, target: &str, _expected: &Identity) -> Result<(), String> {
    use std::os::fd::AsRawFd;
    let source = std::ffi::CString::new(source).map_err(err)?;
    let target = std::ffi::CString::new(target).map_err(err)?;
    let fd = directory.as_raw_fd();
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    let result = unsafe { libc::renameatx_np(fd, source.as_ptr(), fd, target.as_ptr(), libc::RENAME_EXCL) };
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let result = unsafe { libc::syscall(libc::SYS_renameat2, fd, source.as_ptr(), fd, target.as_ptr(), libc::RENAME_NOREPLACE) };
    if result == 0 { Ok(()) } else { Err(err(std::io::Error::last_os_error())) }
}

#[cfg(windows)]
fn rename_directory_noreplace(directory: &Dir, source: &str, target: &str, expected: &Identity) -> Result<(), String> {
    use cap_std::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    // Each attempt reopens the source and re-proves its identity; transient
    // third-party opens of a member are retried briefly.
    crate::storage::retry_transient_sharing(|| {
        let mut options = OpenOptions::new();
        options.access_mode(0x0001_0000 | 0x0000_0080).follow(FollowSymlinks::No).custom_flags(0x0020_0000 | 0x0200_0000);
        let opened = directory.open_with(source, &options)?;
        let metadata = opened.metadata()?;
        if !metadata.is_dir() || metadata.is_symlink() || Identity::of(&metadata) != *expected {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "LocalSwarm cleanup source directory was replaced"));
        }
        crate::storage::rename_into_directory_noreplace(
            opened.as_raw_handle(), directory.as_raw_handle(), std::ffi::OsStr::new(target))
    })
    .map_err(err)
}

#[cfg(windows)]
fn remove_directory_by_identity(directory: &Dir, name: &str, expected: &Identity) -> Result<(), String> {
    crate::storage::remove_empty_directory_verified(
        directory,
        std::path::Path::new(name),
        |metadata| Identity::of(metadata) == *expected,
        "LocalSwarm cleanup directory was replaced; it was preserved",
    )
    .map_err(err)
}
