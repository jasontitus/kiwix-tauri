// SPDX-License-Identifier: GPL-3.0-or-later
//! Bounded, disposable verified-chunk acceleration, not a source of trust.
//! The caller verifies SHA-256 before storing and again after every read. Keep
//! one cache instance per private directory: independent writers cannot share
//! an in-memory capacity ledger. All payload operations use the retained Dir,
//! including when the original directory path is moved or replaced.
#![forbid(unsafe_code)]
use cap_fs_ext::{FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use std::{
    collections::VecDeque,
    fs::File,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};
use uuid::Uuid;

pub(crate) const MAX_BYTES: u64 = 8 * 1024 * 1024;
pub(crate) const MAX_ENTRY_BYTES: u64 = 16 * 1024 * 1024;
pub(crate) const MAX_ENTRIES: usize = 4096;

#[derive(Clone, Copy)]
struct Limits {
    bytes: u64,
    entries: usize,
}

struct Entry {
    name: String,
    bytes: u64,
}

struct CacheState {
    directory: Dir,
    // Used only for directory identity checks and the descriptor-based mobile
    // backup hook, never to open, rename, or remove payloads.
    path: PathBuf,
    entries: VecDeque<Entry>,
    bytes: u64,
    limits: Limits,
    // If cleanup fails, retain the full temporary reservation and refuse new
    // stores. Never report a free budget while an owned temporary may remain.
    failure: Option<String>,
}

pub(crate) struct MapChunkCache {
    // A blocking worker retains the entire operation, including its accounting
    // and temp cleanup, even if the awaiting store future is dropped.
    state: Arc<Mutex<CacheState>>,
}

impl MapChunkCache {
    pub(crate) fn open(path: &Path, check: impl Fn() -> Result<(), String>) -> Result<Self, String> {
        Self::open_checked(path, Limits { bytes: MAX_BYTES, entries: MAX_ENTRIES }, check)
    }

    #[cfg(test)]
    fn open_with_limits(path: &Path, limits: Limits) -> Result<Self, String> {
        Self::open_checked(path, limits, || Ok(()))
    }

    fn open_checked(
        path: &Path,
        limits: Limits,
        check: impl Fn() -> Result<(), String>,
    ) -> Result<Self, String> {
        check()?;
        let directory = crate::storage::open_directory_nofollow(path)
            .map_err(|e| format!("Cannot open map chunk cache: {e}"))?;
        let mut state = CacheState {
            directory,
            path: path.to_path_buf(),
            entries: VecDeque::new(),
            bytes: 0,
            limits,
            failure: None,
        };
        // Streaming admission keeps metadata bounded even for a very large old
        // cache. Restart order need not be stable: FIFO, not persisted LRU.
        loop {
            check()?;
            state.entries.clear();
            state.bytes = 0;
            let mut pruned = false;
            let entries = state.directory.entries()
                .map_err(|e| format!("Cannot scan map chunk cache: {e}"))?;
            for entry in entries {
                check()?;
                let entry = entry.map_err(|e| format!("Cannot scan map chunk cache: {e}"))?;
                let name = entry.file_name();
                let Some(name) = name.to_str() else { continue };
                // Old save_cache and this implementation use the same exact,
                // canonical temporary spelling. A crash can leave one behind.
                if temporary_name(name) {
                    check()?;
                    pruned |= state.remove_regular(name)?;
                    continue;
                }
                let Some(length) = named_length(name) else { continue };
                let Some(metadata) = state.metadata(name)? else { continue };
                if !metadata.is_file() { continue; }
                if metadata.len() != length || length > limits.bytes || limits.entries == 0 {
                    check()?;
                    pruned |= state.remove_regular(name)?;
                    continue;
                }
                let before = state.entries.len();
                state.make_room(length, &check)?;
                pruned |= state.entries.len() < before;
                state.entries.push_back(Entry { name: name.to_owned(), bytes: length });
                state.bytes += length;
            }
            // Filesystems differ on enumeration during deletion. Finish with
            // an unmodified pass rather than assume directory cookies survive
            // pruning. Metadata remains bounded on every pass.
            check()?;
            if !pruned { break; }
        }
        Ok(Self { state: Arc::new(Mutex::new(state)) })
    }

    fn lock(&self) -> Result<MutexGuard<'_, CacheState>, String> {
        self.state.lock().map_err(|_| "Map chunk cache worker failed; reopen the cache".into())
    }

    pub(crate) fn bytes(&self) -> Result<u64, String> {
        Ok(self.lock()?.bytes)
    }

    pub(crate) fn unchanged(&self) -> Result<(), String> {
        self.lock()?.unchanged()
    }

    pub(crate) fn open_chunk(&mut self, name: &str) -> Result<Option<File>, String> {
        let length = named_length(name).ok_or("Invalid map chunk cache name")?;
        let mut state = self.lock()?;
        let Some(metadata) = state.metadata(name)? else {
            state.forget(name);
            return Ok(None);
        };
        if !metadata.is_file() {
            state.forget(name);
            return Ok(None);
        }
        if metadata.len() != length {
            state.discard(name)?;
            return Ok(None);
        }
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            // A concurrent replacement with a FIFO must not block this open.
            options.custom_flags(libc::O_NONBLOCK);
        }
        let file = match state.directory.open_with(name, &options) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                state.forget(name);
                return Ok(None);
            }
            Err(error) => return Err(format!("Cannot open map chunk cache entry: {error}")),
        };
        let metadata = file.metadata().map_err(|e| format!("Cannot inspect map chunk cache entry: {e}"))?;
        if !metadata.is_file() || metadata.len() != length {
            return Ok(None);
        }
        Ok(Some(file.into_std()))
    }

    /// Remove only a recognized regular cache leaf, never a directory/symlink.
    pub(crate) fn discard(&mut self, name: &str) -> Result<(), String> {
        named_length(name).ok_or("Invalid map chunk cache name")?;
        self.lock()?.discard(name)
    }

    /// `bytes` must already be SHA-verified by the caller. No hash or chunk copy
    /// is performed here. Await this operation before composition retirement.
    /// Returns false without writing when the valid chunk exceeds disk capacity.
    pub(crate) async fn store<F>(&mut self, name: &str, bytes: Arc<Vec<u8>>, admit: F) -> Result<bool, String>
    where
        F: FnOnce() -> Result<(), String> + Send + 'static,
    {
        validate_store(name, bytes.len())?;
        let name = name.to_owned();
        let state = Arc::clone(&self.state);
        tokio::task::spawn_blocking(move || {
            let mut state = state.lock()
                .map_err(|_| "Map chunk cache worker failed; reopen the cache".to_owned())?;
            state.store_with_admission(&name, &bytes, admit, |file, bytes| file.write_all(bytes))
        }).await.map_err(|e| format!("Map chunk cache worker failed: {e}"))?
    }
}

impl CacheState {
    fn unchanged(&self) -> Result<(), String> {
        let current = crate::storage::open_directory_nofollow(&self.path)
            .and_then(|directory| directory.dir_metadata())
            .map_err(|e| format!("Cannot verify map chunk cache directory: {e}"))?;
        let retained = self.directory.dir_metadata()
            .map_err(|e| format!("Cannot inspect retained map chunk cache directory: {e}"))?;
        if current.dev() != retained.dev() || current.ino() != retained.ino() {
            return Err("Map chunk cache directory changed identity".into());
        }
        Ok(())
    }

    fn metadata(&self, name: &str) -> Result<Option<cap_std::fs::Metadata>, String> {
        match self.directory.symlink_metadata(name) {
            Ok(metadata) => Ok(Some(metadata)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("Cannot inspect map chunk cache entry: {error}")),
        }
    }

    fn forget(&mut self, name: &str) {
        if let Some(index) = self.entries.iter().position(|entry| entry.name == name) {
            let entry = self.entries.remove(index).expect("Known cache entry index");
            self.bytes -= entry.bytes;
        }
    }

    fn remove_regular(&self, name: &str) -> Result<bool, String> {
        if !self.metadata(name)?.is_some_and(|metadata| metadata.is_file()) {
            return Ok(false);
        }
        match self.directory.remove_file(name) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(format!("Cannot remove map chunk cache entry: {error}")),
        }
    }

    fn discard(&mut self, name: &str) -> Result<(), String> {
        // Do not reduce the ledger until removal actually succeeds.
        self.remove_regular(name)?;
        self.forget(name);
        Ok(())
    }

    fn make_room(&mut self, incoming: u64, check: &impl Fn() -> Result<(), String>) -> Result<(), String> {
        if incoming > self.limits.bytes || self.limits.entries == 0 {
            return Err("Map chunk exceeds cache capacity".into());
        }
        while self.bytes > self.limits.bytes - incoming || self.entries.len() >= self.limits.entries {
            check()?;
            let entry = self.entries.front().ok_or("Map chunk cache accounting mismatch")?;
            self.remove_regular(&entry.name)?;
            let entry = self.entries.pop_front().expect("Known oldest cache entry");
            self.bytes -= entry.bytes;
        }
        Ok(())
    }

    #[cfg(test)]
    fn store_with_writer(
        &mut self,
        name: &str,
        bytes: &[u8],
        write: impl FnOnce(&mut File, &[u8]) -> io::Result<()>,
    ) -> Result<bool, String> {
        self.store_with_admission(name, bytes, || Ok(()), write)
    }

    fn store_with_admission(
        &mut self,
        name: &str,
        bytes: &[u8],
        admit: impl FnOnce() -> Result<(), String>,
        write: impl FnOnce(&mut File, &[u8]) -> io::Result<()>,
    ) -> Result<bool, String> {
        validate_store(name, bytes.len())?;
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        if bytes.len() as u64 > self.limits.bytes || self.limits.entries == 0 {
            return Ok(false);
        }
        self.unchanged()?;
        if self.metadata(name)?.is_some_and(|metadata| !metadata.is_file()) {
            return Err("Map chunk cache name is occupied by a non-regular entry".into());
        }
        // Replacing a regular entry first reclaims it: retaining it during the
        // temp write would double its contribution to the real peak budget.
        self.discard(name)?;
        self.make_room(bytes.len() as u64, &|| Ok(()))?;
        // Unlinking a cache leaf does not prove physical space was reclaimed:
        // hard links and open handles may retain the old inode. Refresh native
        // capacity after eviction, reserving the FULL incoming allocation.
        // The callback must not re-enter this cache while its ledger is locked.
        self.unchanged()?;
        let admission = admit();
        self.unchanged()?;
        admission?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true).follow(FollowSymlinks::No);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let temp_name = format!("{name}.{}.part", Uuid::new_v4());
        let mut file = self.directory.open_with(&temp_name, &options)
            .map_err(|e| format!("Cannot create map chunk cache temporary: {e}"))?.into_std();
        let mut temporary = Temporary { directory: &self.directory, name: Some(temp_name) };
        let result = (|| {
            let temp_name = temporary.name.as_ref().expect("Owned cache temporary");
            crate::mobile_backup::exclude_payload(&file, &self.path.join(temp_name))?;
            write(&mut file, bytes).map_err(|e| format!("Cannot write map chunk cache: {e}"))?;
            file.sync_all().map_err(|e| format!("Cannot sync map chunk cache: {e}"))?;
            // Close before cleanup/rename, including on Windows. An early
            // return also drops the closure-owned file before cleanup below.
            drop(file);
            if self.metadata(name)?.is_some_and(|metadata| !metadata.is_file()) {
                return Err("Map chunk cache publication name became non-regular".into());
            }
            self.directory.rename(temp_name, &self.directory, name)
                .map_err(|e| format!("Cannot publish map chunk cache: {e}"))?;
            Ok(())
        })();
        if let Err(error) = result {
            let error = temporary.cleanup_error(error);
            if temporary.name.is_some() {
                self.bytes += bytes.len() as u64;
                self.failure = Some(error.clone());
            }
            return Err(error);
        }
        temporary.name = None;
        self.entries.push_back(Entry { name: name.to_owned(), bytes: bytes.len() as u64 });
        self.bytes += bytes.len() as u64;
        Ok(true)
    }
}

struct Temporary<'a> {
    directory: &'a Dir,
    name: Option<String>,
}

impl Temporary<'_> {
    fn cleanup_error(&mut self, original: String) -> String {
        let Some(name) = self.name.as_ref() else { return original };
        match self.directory.remove_file(name) {
            Ok(()) => { self.name = None; original }
            Err(error) if error.kind() == io::ErrorKind::NotFound => { self.name = None; original }
            Err(error) => format!("{original}; cannot remove owned map cache temporary {name}: {error}"),
        }
    }
}

impl Drop for Temporary<'_> {
    fn drop(&mut self) {
        if let Some(name) = self.name.as_ref() {
            let _ = self.directory.remove_file(name);
        }
    }
}

fn named_length(name: &str) -> Option<u64> {
    let (hash, length) = name.split_once('-')?;
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
        return None;
    }
    if length.is_empty() || length.len() > 8 || length.starts_with('0') || !length.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let length: u64 = length.parse().ok()?;
    (length <= MAX_ENTRY_BYTES).then_some(length)
}

fn temporary_name(name: &str) -> bool {
    let Some((chunk, id)) = name.strip_suffix(".part").and_then(|name| name.rsplit_once('.')) else {
        return false;
    };
    named_length(chunk).is_some()
        && Uuid::parse_str(id).is_ok_and(|uuid| uuid.to_string() == id)
}

fn validate_store(name: &str, size: usize) -> Result<(), String> {
    if named_length(name) != Some(size as u64) {
        return Err("Map chunk cache name and payload size differ or exceed the entry limit".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io::Read};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("map-chunk-cache-{}", Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(path.canonicalize().unwrap())
        }
        fn cache(&self, bytes: u64, entries: usize) -> MapChunkCache {
            MapChunkCache::open_with_limits(&self.0, Limits { bytes, entries }).unwrap()
        }
        fn count_and_bytes(&self) -> (usize, u64) {
            fs::read_dir(&self.0).unwrap().map(|entry| {
                // Query the entry itself: Windows directory-enumeration sizes
                // are cached and can lag a still-open writer (reported 0).
                let metadata = fs::symlink_metadata(entry.unwrap().path()).unwrap();
                (1, metadata.len())
            }).fold((0, 0), |(n, bytes), (one, len)| (n + one, bytes + len))
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
    }
    fn name(index: u64, size: u64) -> String { format!("{index:064x}-{size}") }
    fn store(cache: &MapChunkCache, index: u64, bytes: &[u8]) {
        cache.lock().unwrap().store_with_writer(&name(index, bytes.len() as u64), bytes,
            |file, bytes| file.write_all(bytes)).unwrap();
    }

    #[test]
    fn evicts_before_allocating_and_bounds_entries() {
        let fixture = Fixture::new();
        let mut cache = fixture.cache(10, 2);
        store(&cache, 1, b"123456");
        store(&cache, 2, b"7890");
        cache.lock().unwrap().store_with_writer(&name(3, 7), b"abcdefg", |file, bytes| {
            // The temporary exists but is still empty; both old entries had
            // to go before allocating any new payload bytes.
            assert_eq!(fixture.count_and_bytes(), (1, 0));
            file.write_all(bytes)?;
            assert_eq!(fixture.count_and_bytes(), (1, 7));
            Ok(())
        }).unwrap();
        assert_eq!(cache.bytes().unwrap(), 7);
        assert!(cache.open_chunk(&name(1, 6)).unwrap().is_none());
        store(&cache, 4, b"x");
        store(&cache, 5, b"y");
        assert_eq!(cache.bytes().unwrap(), 2);
        assert_eq!(fixture.count_and_bytes(), (2, 2));
        assert!(cache.open_chunk(&name(3, 7)).unwrap().is_none());
    }

    #[test]
    fn startup_prunes_old_cache_with_bounded_accounting() {
        let fixture = Fixture::new();
        for index in 0..20 { fs::write(fixture.0.join(name(index, 3)), b"abc").unwrap(); }
        // A recognized name with oversized actual payload must be reclaimed.
        let invalid = fixture.0.join(name(50, 1));
        File::create(&invalid).unwrap().set_len(MAX_ENTRY_BYTES + 1).unwrap();
        let cache = fixture.cache(10, 2);
        assert!(!invalid.exists());
        assert_eq!(cache.bytes().unwrap(), 6);
        assert_eq!(cache.lock().unwrap().entries.len(), 2);
        assert_eq!(fixture.count_and_bytes(), (2, 6));
        drop(cache);
        assert_eq!(fixture.cache(4, 1).bytes().unwrap(), 3);
        assert_eq!(fixture.count_and_bytes(), (1, 3));
        let oversized = fixture.0.join(name(80, MAX_ENTRY_BYTES));
        File::create(&oversized).unwrap().set_len(MAX_ENTRY_BYTES).unwrap();
        let production_cache = MapChunkCache::open(&fixture.0, || Ok(())).unwrap();
        assert!(!oversized.exists());
        assert!(production_cache.bytes().unwrap() <= MAX_BYTES);
    }

    #[test]
    fn invalid_names_and_unrelated_entries_are_preserved() {
        let fixture = Fixture::new();
        let unknown = ["readme".to_owned(), format!("{}-1", "A".repeat(64)),
            format!("{}-01", "a".repeat(64)), name(1, 0), name(2, MAX_ENTRY_BYTES + 1)];
        for name in &unknown { fs::write(fixture.0.join(name), b"sentinel").unwrap(); }
        fs::create_dir(fixture.0.join(name(3, 1))).unwrap();
        fs::write(fixture.0.join(name(4, 5)), b"wrong length").unwrap();
        let mut cache = fixture.cache(8, 2);
        assert_eq!(cache.bytes().unwrap(), 0);
        for name in &unknown {
            assert_eq!(fs::read(fixture.0.join(name)).unwrap(), b"sentinel");
            assert!(cache.discard(name).is_err());
        }
        cache.discard(&name(3, 1)).unwrap();
        assert!(fixture.0.join(name(3, 1)).is_dir());
        assert!(!fixture.0.join(name(4, 5)).exists());
        assert!(cache.lock().unwrap().store_with_writer(&name(3, 1), b"x", |f, b| f.write_all(b)).is_err());
        assert!(cache.lock().unwrap().store_with_writer(&name(4, 2), b"x", |f, b| f.write_all(b)).is_err());
        assert_eq!(named_length(&name(5, MAX_ENTRY_BYTES)), Some(MAX_ENTRY_BYTES));
    }

    #[test]
    fn restart_reclaims_only_canonical_regular_cache_temporaries() {
        let fixture = Fixture::new();
        let id = "12345678-1234-4234-8234-123456789abc";
        let old_temp = format!("{}.{id}.part", name(1, 3));
        fs::write(fixture.0.join(&old_temp), b"unfinished old payload").unwrap();
        let unknown = format!("{}.NOT-A-UUID.part", name(2, 3));
        fs::write(fixture.0.join(&unknown), b"unknown").unwrap();
        let directory = format!("{}.{id}.part", name(3, 3));
        fs::create_dir(fixture.0.join(&directory)).unwrap();
        let cache = fixture.cache(8, 2);
        assert_eq!(cache.bytes().unwrap(), 0);
        assert!(!fixture.0.join(old_temp).exists());
        assert_eq!(fs::read(fixture.0.join(unknown)).unwrap(), b"unknown");
        assert!(fixture.0.join(directory).is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_and_replaced_directory_do_not_escape_capability() {
        let fixture = Fixture::new();
        let outside = Fixture::new();
        let sentinel = outside.0.join("sentinel");
        fs::write(&sentinel, b"outside").unwrap();
        let link_name = name(1, 7);
        std::os::unix::fs::symlink(&sentinel, fixture.0.join(&link_name)).unwrap();
        let temp_link = format!("{}.12345678-1234-4234-8234-123456789abc.part", name(3, 7));
        std::os::unix::fs::symlink(&sentinel, fixture.0.join(&temp_link)).unwrap();
        let mut cache = fixture.cache(10, 2);
        assert!(cache.open_chunk(&link_name).unwrap().is_none());
        cache.discard(&link_name).unwrap();
        assert!(cache.lock().unwrap().store_with_writer(&link_name, b"outside", |f, b| f.write_all(b)).is_err());
        assert!(fs::symlink_metadata(fixture.0.join(&link_name)).unwrap().is_symlink());
        assert!(fs::symlink_metadata(fixture.0.join(&temp_link)).unwrap().is_symlink());
        store(&cache, 2, b"old");
        let retained = outside.0.join("retained-cache");
        fs::rename(&fixture.0, &retained).unwrap();
        fs::create_dir(&fixture.0).unwrap();
        fs::write(fixture.0.join(name(2, 3)), b"new").unwrap();
        assert!(cache.unchanged().is_err());
        assert!(cache.lock().unwrap().store_with_writer(&name(2, 3), b"old", |f, b| f.write_all(b)).is_err());
        let mut bytes = Vec::new();
        cache.open_chunk(&name(2, 3)).unwrap().unwrap().read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"old");
        cache.discard(&name(2, 3)).unwrap();
        assert!(!retained.join(name(2, 3)).exists());
        assert_eq!(fs::read(fixture.0.join(name(2, 3))).unwrap(), b"new");
        assert_eq!(fs::read(sentinel).unwrap(), b"outside");
    }

    #[test]
    fn failed_write_cleans_temporary_and_retains_accurate_bytes() {
        let fixture = Fixture::new();
        let cache = fixture.cache(8, 3);
        store(&cache, 1, b"aaaa");
        store(&cache, 2, b"bb");
        let error = cache.lock().unwrap().store_with_writer(&name(3, 4), b"cccc", |file, bytes| {
            file.write_all(&bytes[..2])?;
            Err(io::Error::other("injected write failure"))
        }).unwrap_err();
        assert!(error.contains("injected write failure"));
        assert_eq!(cache.bytes().unwrap(), 2);
        assert_eq!(fixture.count_and_bytes(), (1, 2));
        assert_eq!(fs::read(fixture.0.join(name(2, 2))).unwrap(), b"bb");
        assert!(!fixture.0.join(name(3, 4)).exists());
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_failure_reserves_bytes_and_refuses_further_growth() {
        let fixture = Fixture::new();
        let cache = fixture.cache(8, 3);
        store(&cache, 1, b"aa");
        let error = cache.lock().unwrap().store_with_writer(&name(2, 4), b"bbbb", |file, bytes| {
            file.write_all(bytes)?;
            let temp = fs::read_dir(&fixture.0).unwrap().map(|entry| entry.unwrap().path())
                .find(|path| path.file_name().unwrap().to_str().is_some_and(temporary_name)).unwrap();
            // An unremovable replacement makes cleanup fail without changing
            // filesystem permissions or relying on the test process's uid.
            fs::remove_file(&temp)?;
            fs::create_dir(&temp)?;
            Err(io::Error::other("injected cleanup obstruction"))
        }).unwrap_err();
        assert!(error.contains("cannot remove owned map cache temporary"));
        assert_eq!(cache.bytes().unwrap(), 6);
        let mut called = false;
        assert!(cache.lock().unwrap().store_with_writer(&name(3, 1), b"c", |_, _| {
            called = true;
            Ok(())
        }).is_err());
        assert!(!called);
        assert_eq!(cache.bytes().unwrap(), 6);
        assert_eq!(fs::read(fixture.0.join(name(1, 2))).unwrap(), b"aa");
    }

    #[tokio::test]
    async fn verified_arc_store_reads_exact_length_and_discards_corruption() {
        let fixture = Fixture::new();
        let mut cache = fixture.cache(8, 3);
        let bytes = Arc::new(b"verified".to_vec());
        assert!(cache.store(&name(1, 8), Arc::clone(&bytes), || Ok(())).await.unwrap());
        let mut read = Vec::new();
        cache.open_chunk(&name(1, 8)).unwrap().unwrap().read_to_end(&mut read).unwrap();
        assert_eq!(read, *bytes);
        fs::write(fixture.0.join(name(1, 8)), b"short").unwrap();
        assert!(cache.open_chunk(&name(1, 8)).unwrap().is_none());
        assert_eq!(cache.bytes().unwrap(), 0);
        assert!(!fixture.0.join(name(1, 8)).exists());
        assert!(cache.store(&name(2, 1), Arc::new(Vec::new()), || Ok(())).await.is_err());
        assert_eq!(fixture.count_and_bytes(), (0, 0));
        let oversized = Arc::new(b"oversized".to_vec());
        assert!(!cache.store(&name(3, 9), oversized, || Err("Oversized chunks must skip admission".into())).await.unwrap());
        assert_eq!(cache.bytes().unwrap(), 0);
        assert_eq!(fixture.count_and_bytes(), (0, 0));
    }

    #[test]
    fn cancellation_interrupts_scan_before_any_eviction() {
        use std::cell::Cell;
        let fixture = Fixture::new();
        for index in 0..20 { fs::write(fixture.0.join(name(index, 1)), b"x").unwrap(); }
        let checks = Cell::new(0);
        let result = MapChunkCache::open_checked(&fixture.0, Limits { bytes: 100, entries: 100 }, || {
            checks.set(checks.get() + 1);
            if checks.get() == 6 { Err("cancelled during scan".into()) } else { Ok(()) }
        });
        assert!(matches!(result, Err(error) if error == "cancelled during scan"));
        assert_eq!(fixture.count_and_bytes(), (20, 20));
    }

    #[test]
    fn cancellation_interrupts_pruning_between_evictions() {
        let fixture = Fixture::new();
        for index in 0..20 { fs::write(fixture.0.join(name(index, 1)), b"x").unwrap(); }
        let result = MapChunkCache::open_checked(&fixture.0, Limits { bytes: 2, entries: 2 }, || {
            if fixture.count_and_bytes().0 < 20 { Err("cancelled after eviction".into()) } else { Ok(()) }
        });
        assert!(matches!(result, Err(error) if error == "cancelled after eviction"));
        assert_eq!(fixture.count_and_bytes(), (19, 19));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn admission_runs_after_unlink_and_rejects_without_allocating_a_temporary() {
        let fixture = Fixture::new();
        let outside = Fixture::new();
        let mut cache = fixture.cache(8, 3);
        let old_name = name(1, 6);
        assert!(cache.store(&old_name, Arc::new(b"retain".to_vec()), || Ok(())).await.unwrap());
        let mut open_old = cache.open_chunk(&old_name).unwrap().unwrap();
        let hard_link = outside.0.join("retained-inode");
        fs::hard_link(fixture.0.join(&old_name), &hard_link).unwrap();
        let directory = fixture.0.clone();
        let callback_link = hard_link.clone();
        let error = cache.store(&name(2, 4), Arc::new(b"next".to_vec()), move || {
            assert!(!directory.join(old_name).exists());
            assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);
            // Eviction changed the namespace, not physical allocation: both
            // an open descriptor and another hard link retain the old inode.
            let mut retained = Vec::new();
            open_old.read_to_end(&mut retained).unwrap();
            assert_eq!(retained, b"retain");
            assert_eq!(fs::read(callback_link).unwrap(), b"retain");
            Err("fresh capacity cannot admit the full incoming chunk".into())
        }).await.unwrap_err();
        assert!(error.contains("fresh capacity"));
        assert_eq!(cache.bytes().unwrap(), 0);
        assert_eq!(fixture.count_and_bytes(), (0, 0));
        assert_eq!(fs::read(hard_link).unwrap(), b"retain");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn moved_directory_is_rejected_before_and_after_admission() {
        use std::sync::atomic::{AtomicBool, Ordering};
        for move_before in [true, false] {
            let fixture = Fixture::new();
            let outside = Fixture::new();
            let mut cache = fixture.cache(8, 3);
            cache.unchanged().unwrap();
            let retained = outside.0.join("retained-cache");
            if move_before {
                fs::rename(&fixture.0, &retained).unwrap();
                fs::create_dir(&fixture.0).unwrap();
            }
            let directory = fixture.0.clone();
            let callback_retained = retained.clone();
            let called = Arc::new(AtomicBool::new(false));
            let callback_called = Arc::clone(&called);
            let error = cache.store(&name(1, 4), Arc::new(b"next".to_vec()), move || {
                callback_called.store(true, Ordering::SeqCst);
                fs::rename(&directory, callback_retained).map_err(|e| e.to_string())?;
                fs::create_dir(directory).map_err(|e| e.to_string())?;
                Ok(())
            }).await.unwrap_err();
            assert!(error.contains("changed identity"));
            assert_eq!(called.load(Ordering::SeqCst), !move_before);
            assert!(cache.unchanged().is_err());
            assert_eq!(cache.bytes().unwrap(), 0);
            assert_eq!(fixture.count_and_bytes(), (0, 0));
            assert_eq!(fs::read_dir(retained).unwrap().count(), 0);
        }
    }
}
