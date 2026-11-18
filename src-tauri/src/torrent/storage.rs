// SPDX-License-Identifier: GPL-3.0-or-later
//! Confined librqbit storage backends that write only inside owned staging or library directories.
#[allow(unused_imports)]
use super::*;

// Capability-confined I/O prevents an intervening symlink from escaping the
// owned directory even between validation and opening a file. Hard links are
// rejected before any length change/write. No engine deletion API is enabled.
#[derive(Clone)]
pub(super) struct ConfinedFactory {
    pub(super) root: Arc<Dir>,
    pub(super) path: PathBuf,
    pub(super) expected: Arc<HashMap<PathBuf, u64>>,
    pub(super) sources: Arc<Mutex<HashMap<PathBuf, EngineSource>>>,
    pub(super) existing: bool,
    pub(super) stop: CancellationToken,
    pub(super) gate: Arc<Mutex<()>>,
    /// Parent prefixes this factory already created/validated and journaled.
    /// Without it every file re-opened and re-checked each ancestor directory.
    pub(super) dirs: Arc<Mutex<HashSet<PathBuf>>>,
    pub(super) owner: Option<(TorrentManager, String)>,
}

// These are the same open inodes the engine hashes and writes, not paths that
// can be replaced between its final piece check and publication.
#[derive(Clone)]
pub(super) struct EngineSource {
    pub(super) file: Arc<File>,
    pub(super) size: u64,
}

impl ConfinedFactory {
    /// Validate, exclude from backup and (for a newly created file) journal an
    /// opened payload file. Any error leaves the caller to undo the creation.
    fn adopt_opened(&self, path: &Path, file: cap_std::fs::File, exists: bool) -> anyhow::Result<Arc<File>> {
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.nlink() != 1 {
            bail!("Torrent storage is not a singly linked regular file");
        }
        crate::mobile_backup::exclude_payload(&file, &self.path.join(path)).map_err(anyhow::Error::msg)?;
        let file = Arc::new(file.into_std());
        if !exists {
            if let Some((owner, id)) = &self.owner {
                let entry = CleanupEntry::retain(&self.path.join(path), &file).map_err(anyhow::Error::msg)?;
                owner.update_record(id, |record| record.staging_cleanup.push((path.to_owned(), entry)))
                    .map_err(anyhow::Error::msg)?;
            }
        }
        Ok(file)
    }

    pub(super) fn open_file(&self, path: &Path, size: u64) -> anyhow::Result<Arc<File>> {
        safe_relative(path)?;
        if self.expected.get(path) != Some(&size) {
            bail!("Torrent file path or advertised length changed");
        }
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            #[cfg_attr(not(unix), allow(unused_mut, reason = "the builder is only configured with a Unix mode"))]
            let mut builder = cap_std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use cap_std::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            let mut prefix = PathBuf::new();
            for component in parent.components() {
                prefix.push(component.as_os_str());
                // Only a *new* directory has an identity to journal. Once this
                // factory created and journaled a prefix (or validated an
                // existing one), later files need neither the second open nor
                // another journal write. The no-symlink pass below still runs
                // for every component of every open.
                let known = self
                    .dirs
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Torrent storage lock poisoned"))?
                    .contains(&prefix);
                let created = match self.root.create_dir_with(&prefix, &builder) {
                    Ok(()) => true,
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
                    Err(error) => return Err(error.into()),
                };
                if !created && known {
                    continue;
                }
                let journaled = (|| -> anyhow::Result<()> {
                    let directory = crate::storage::open_directory_nofollow(&self.path.join(&prefix))?;
                    if created {
                        if let Some((owner, id)) = &self.owner {
                            let identity = PublicationIdentity::of(&directory.dir_metadata()?);
                            owner.update_record(id, |record| record.staging_directories.push((prefix.clone(), identity)))
                                .map_err(anyhow::Error::msg)?;
                        }
                    }
                    Ok(())
                })();
                match journaled {
                    Ok(()) => {
                        self.dirs
                            .lock()
                            .map_err(|_| anyhow::anyhow!("Torrent storage lock poisoned"))?
                            .insert(prefix.clone());
                    }
                    Err(error) => {
                        // An unjournaled directory we just created would later be
                        // refused as foreign by cleanup forever; it is still empty.
                        if created { let _ = self.root.remove_dir(&prefix); }
                        return Err(error);
                    }
                }
            }
        }
        let mut prefix = PathBuf::new();
        for component in path.components() {
            prefix.push(component.as_os_str());
            if let Ok(meta) = self.root.symlink_metadata(&prefix) {
                if meta.file_type().is_symlink() {
                    bail!("Symlink in torrent storage");
                }
            }
        }
        let mut options = cap_std::fs::OpenOptions::new();
        options.read(true).write(true).follow(FollowSymlinks::No);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NONBLOCK);
        }
        let exists = match self.root.symlink_metadata(path) {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(error.into()),
        };
        if !self.existing || !exists { options.create_new(true); }
        let file = self.root.open_with(path, &options)?;
        let file = match self.adopt_opened(path, file, exists) {
            Ok(file) => file,
            Err(error) => {
                // A file this call just created (create_new) and failed to
                // journal is still empty and ours. Leaving it would make the
                // next attempt adopt it without a ledger entry, after which
                // deletion refuses it as an "unexpected temporary file".
                if !exists { let _ = self.root.remove_file(path); }
                return Err(error);
            }
        };
        if exists {
            if let Some((owner, id)) = &self.owner {
                if let Some(entry) = owner.cleanup_entry(id, path).map_err(anyhow::Error::msg)? {
                    if !entry.matches(&self.path.join(path), &file).map_err(anyhow::Error::msg)? {
                        bail!("Owned torrent staging file was replaced; refusing to resume writes");
                    }
                }
            }
        }
        if is_zim(path) {
            self.sources
                .lock()
                .map_err(|_| anyhow::anyhow!("Torrent source lock poisoned"))?
                .insert(
                    path.to_owned(),
                    EngineSource {
                        file: file.clone(),
                        size,
                    },
                );
        }
        Ok(file)
    }
}

pub(super) struct ConfinedStorage {
    pub(super) factory: ConfinedFactory,
    pub(super) files: Mutex<Vec<Option<Arc<File>>>>,
}

impl StorageFactory for ConfinedFactory {
    type Storage = ConfinedStorage;
    fn create(
        &self,
        _: &ManagedTorrentShared,
        _: &TorrentMetadata,
    ) -> anyhow::Result<Self::Storage> {
        Ok(ConfinedStorage {
            factory: self.clone(),
            files: Mutex::new(vec![]),
        })
    }
    fn clone_box(&self) -> BoxStorageFactory {
        self.clone().boxed()
    }
}

impl TorrentStorage for ConfinedStorage {
    fn init(&mut self, _: &ManagedTorrentShared, metadata: &TorrentMetadata) -> anyhow::Result<()> {
        let _write = self
            .factory
            .gate
            .lock()
            .map_err(|_| anyhow::anyhow!("Sharing was interrupted. Turn sharing off and on again"))?;
        if self.factory.stop.is_cancelled() {
            bail!("Torrent manager has shut down");
        }
        let mut files = Vec::with_capacity(metadata.file_infos.len());
        let mut seen = BTreeSet::new();
        for info in metadata.file_infos.iter() {
            let path = &info.relative_filename;
            safe_relative(path)?;
            if info.attrs.symlink {
                bail!("This file contains a shortcut and cannot be shared. Download it again");
            }
            if self.factory.expected.get(path) != Some(&info.len)
                || !seen.insert(path.to_string_lossy().to_lowercase())
            {
                bail!("Torrent paths changed or collide on a case-insensitive filesystem");
            }
            if info.attrs.padding {
                files.push(None);
                continue;
            }
            files.push(Some(self.factory.open_file(path, info.len)?));
        }
        *self
            .files
            .lock()
            .map_err(|_| anyhow::anyhow!("Sharing was interrupted. Turn sharing off and on again"))? = files;
        Ok(())
    }
    fn pread_exact(&self, id: usize, offset: u64, buf: &mut [u8]) -> anyhow::Result<()> {
        let _read = self.factory.gate.lock()
            .map_err(|_| anyhow::anyhow!("Sharing was interrupted. Turn sharing off and on again"))?;
        if self.factory.stop.is_cancelled() { bail!("Sharing stopped because the app is closing"); }
        let files = self
            .files
            .lock()
            .map_err(|_| anyhow::anyhow!("Sharing was interrupted. Turn sharing off and on again"))?;
        match files.get(id).context("Unknown torrent file")? {
            Some(f) => f.pread_exact(offset, buf),
            None => {
                buf.fill(0);
                Ok(())
            }
        }
    }
    fn pwrite_all(&self, id: usize, offset: u64, buf: &[u8]) -> anyhow::Result<()> {
        let _write = self
            .factory
            .gate
            .lock()
            .map_err(|_| anyhow::anyhow!("Sharing was interrupted. Turn sharing off and on again"))?;
        if self.factory.stop.is_cancelled() {
            bail!("Torrent manager has shut down");
        }
        let files = self
            .files
            .lock()
            .map_err(|_| anyhow::anyhow!("Sharing was interrupted. Turn sharing off and on again"))?;
        match files.get(id).context("Unknown torrent file")? {
            Some(f) => f.pwrite_all(offset, buf),
            None if buf.iter().all(|b| *b == 0) => Ok(()),
            None => bail!("Nonzero torrent padding"),
        }
    }
    fn ensure_file_length(&self, id: usize, length: u64) -> anyhow::Result<()> {
        let _write = self
            .factory
            .gate
            .lock()
            .map_err(|_| anyhow::anyhow!("Sharing was interrupted. Turn sharing off and on again"))?;
        if self.factory.stop.is_cancelled() {
            bail!("Torrent manager has shut down");
        }
        let files = self
            .files
            .lock()
            .map_err(|_| anyhow::anyhow!("Sharing was interrupted. Turn sharing off and on again"))?;
        if let Some(f) = files.get(id).context("Unknown torrent file")? {
            f.set_len(length)?;
        }
        Ok(())
    }
    fn take(&self) -> anyhow::Result<Box<dyn TorrentStorage>> {
        let files = std::mem::take(
            &mut *self
                .files
                .lock()
                .map_err(|_| anyhow::anyhow!("Sharing was interrupted. Turn sharing off and on again"))?,
        );
        Ok(Box::new(Self {
            factory: self.factory.clone(),
            files: Mutex::new(files),
        }))
    }
    fn remove_file(&self, _: usize, _: &Path) -> anyhow::Result<()> {
        bail!("Engine file deletion is disabled")
    }
    fn remove_directory_if_empty(&self, _: &Path) -> anyhow::Result<()> {
        bail!("Engine directory deletion is disabled")
    }
}

/// Storage for a shared library archive. It maps the torrent's single file to
/// the exact inode selected by the user and opens it read-only. Any attempt by
/// the engine to resize or write proves the library file no longer matches the
/// created metadata and fails closed instead of mutating the library.
#[derive(Clone)]
pub(super) struct LibraryFactory {
    pub(super) path: PathBuf,
    pub(super) expected: ShareIdentity,
    pub(super) stop: CancellationToken,
    pub(super) gate: Arc<Mutex<()>>,
}

pub(super) struct LibraryStorage {
    pub(super) factory: LibraryFactory,
    pub(super) file: Mutex<Option<Arc<File>>>,
}

impl StorageFactory for LibraryFactory {
    type Storage = LibraryStorage;

    fn create(&self, _: &ManagedTorrentShared, _: &TorrentMetadata) -> anyhow::Result<Self::Storage> {
        Ok(LibraryStorage {
            factory: self.clone(),
            file: Mutex::new(None),
        })
    }

    fn clone_box(&self) -> BoxStorageFactory {
        self.clone().boxed()
    }
}

impl TorrentStorage for LibraryStorage {
    fn init(&mut self, _: &ManagedTorrentShared, metadata: &TorrentMetadata) -> anyhow::Result<()> {
        let _gate = self
            .factory
            .gate
            .lock()
            .map_err(|_| anyhow::anyhow!("Sharing was interrupted. Turn sharing off and on again"))?;
        if self.factory.stop.is_cancelled() {
            bail!("Torrent manager has shut down");
        }
        let expected_name = self
            .factory
            .path
            .file_name()
            .context("This file cannot be shared because it has no name. Rename it and try again")?;
        let mut selected = None;
        for info in metadata.file_infos.iter() {
            if info.attrs.symlink {
                bail!("This file contains a shortcut and cannot be shared. Download it again");
            }
            if info.attrs.padding {
                if info.len != 0 {
                    bail!("This file cannot be shared because its contents are not valid. Download it again");
                }
                continue;
            }
            if selected.is_some()
                || info.relative_filename.file_name() != Some(expected_name)
                || info.len != self.factory.expected.size
            {
                bail!("Torrent metadata does not describe the selected library archive");
            }
            selected = Some(info);
        }
        if selected.is_none() {
            bail!("This file cannot be shared because it has no content. Download it again");
        }
        let parent = self
            .factory
            .path
            .parent()
            .context("This shared file is no longer where Kiwix last saw it. Add it to the library again")?;
        let directory = crate::storage::open_directory_nofollow(parent)?;
        let mut options = cap_std::fs::OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NONBLOCK);
        }
        let file = directory
            .open_with(expected_name, &options)
            .with_context(|| "Kiwix could not open the shared file. Add it to the library again".to_string())?;
        let metadata = file.metadata()?;
        if ShareIdentity::of(&metadata).map_err(anyhow::Error::msg)? != self.factory.expected {
            bail!("The shared file changed while sharing started. Start sharing again");
        }
        let file = file.into_std();
        FileExt::try_flock_shared(&file)
            .map_err(|_| anyhow::anyhow!("The shared file is in use by another app. Close the other app and try again"))?;
        *self
            .file
            .lock()
            .map_err(|_| anyhow::anyhow!("Sharing was interrupted. Turn sharing off and on again"))? =
            Some(Arc::new(file));
        Ok(())
    }

    fn pread_exact(&self, id: usize, offset: u64, buf: &mut [u8]) -> anyhow::Result<()> {
        let _read = self
            .factory
            .gate
            .lock()
            .map_err(|_| anyhow::anyhow!("Sharing was interrupted. Turn sharing off and on again"))?;
        if self.factory.stop.is_cancelled() {
            bail!("Sharing stopped because the app is closing");
        }
        let file = self
            .file
            .lock()
            .map_err(|_| anyhow::anyhow!("Sharing was interrupted. Turn sharing off and on again"))?;
        let file = file.as_ref().context("Unknown torrent file")?;
        if id != 0 {
            bail!("Unknown torrent file");
        }
        file.pread_exact(offset, buf)
    }

    fn pwrite_all(&self, _: usize, _: u64, _: &[u8]) -> anyhow::Result<()> {
        bail!("Shared library archives are read-only")
    }

    fn ensure_file_length(&self, id: usize, length: u64) -> anyhow::Result<()> {
        let _write = self
            .factory
            .gate
            .lock()
            .map_err(|_| anyhow::anyhow!("Sharing was interrupted. Turn sharing off and on again"))?;
        if self.factory.stop.is_cancelled() {
            bail!("Torrent manager has shut down");
        }
        let file = self
            .file
            .lock()
            .map_err(|_| anyhow::anyhow!("Sharing was interrupted. Turn sharing off and on again"))?;
        let file = file.as_ref().context("Unknown torrent file")?;
        if id != 0 {
            bail!("Unknown torrent file");
        }
        let metadata = cap_std::fs::Metadata::from_file(file)?;
        if metadata.len() != length
            || ShareIdentity::of(&metadata).map_err(anyhow::Error::msg)? != self.factory.expected
        {
            bail!("The shared library archive no longer matches its torrent");
        }
        Ok(())
    }

    fn take(&self) -> anyhow::Result<Box<dyn TorrentStorage>> {
        let file = self
            .file
            .lock()
            .map_err(|_| anyhow::anyhow!("Sharing was interrupted. Turn sharing off and on again"))?
            .take();
        Ok(Box::new(Self {
            factory: self.factory.clone(),
            file: Mutex::new(file),
        }))
    }

    fn remove_file(&self, _: usize, _: &Path) -> anyhow::Result<()> {
        bail!("Engine file deletion is disabled")
    }

    fn remove_directory_if_empty(&self, _: &Path) -> anyhow::Result<()> {
        bail!("Engine directory deletion is disabled")
    }
}

/// Seeds a first-run acquisition's staging files with byte ranges that an
/// unfinished LocalSwarm job has already verified for a same-sized file.
///
/// Each file is created through the factory, so its ownership is journaled
/// exactly as if the engine had created it. Nothing copied here is trusted:
/// the engine hashes every piece in its initial check and counts only pieces
/// that match the torrent metadata; any other byte is downloaded again.
/// Donor problems are logged and skipped, never fatal to the acquisition.
/// Returns whether any staging file was created.
pub(super) fn prefill_from_partials(
    factory: &ConfinedFactory,
    store: &crate::store::Store,
    files: &[SpaceFile],
) -> Result<bool, String> {
    let donors = match crate::localswarm::LocalSwarmManager::saved_partial_files(store) {
        Ok(donors) => donors,
        Err(error) => {
            eprintln!("Skipping LocalSwarm partial reuse: {error}");
            return Ok(false);
        }
    };
    prefill_from_donors(factory, &donors, files)
}

pub(super) fn prefill_from_donors(
    factory: &ConfinedFactory,
    donors: &[crate::partial_donor::Donor],
    files: &[SpaceFile],
) -> Result<bool, String> {
    // Index usable donors by size once: re-filtering the whole donor list for
    // every target file was quadratic in (files x donors).
    let mut by_size: HashMap<u64, Vec<&crate::partial_donor::Donor>> = HashMap::new();
    for donor in donors {
        if donor.size > 0 && donor.ranges.as_ref().is_some_and(|ranges| !ranges.is_empty()) {
            by_size.entry(donor.size).or_default().push(donor);
        }
    }
    // One 1 MiB copy buffer for the whole prefill, not one per (file, donor).
    let mut buffer: Option<Vec<u8>> = None;
    let mut created = false;
    for file in files {
        // `by_size` holds only non-empty range maps for nonzero sizes, so a
        // miss means no donor can seed this file.
        let Some(candidates) = by_size.get(&file.size) else {
            continue;
        };
        if factory.stop.is_cancelled() {
            break;
        }
        let target = factory.open_file(&file.path, file.size).map_err(display)?;
        created = true;
        target.set_len(file.size).map_err(display)?;
        for &donor in candidates {
            let buffer = buffer.get_or_insert_with(|| vec![0u8; COPY_BLOCK as usize]);
            match copy_verified_ranges(&target, donor, &factory.stop, buffer) {
                Ok(0) => {}
                Ok(copied) => eprintln!(
                    "Reused {copied} bytes for {} from an unfinished LocalSwarm download",
                    file.path.display()
                ),
                Err(error) => eprintln!("Skipping LocalSwarm partial {}: {error}", donor.path.display()),
            }
        }
        target.sync_data().map_err(display)?;
    }
    Ok(created)
}

const COPY_BLOCK: u64 = 1024 * 1024;

fn copy_verified_ranges(
    target: &File,
    donor: &crate::partial_donor::Donor,
    stop: &CancellationToken,
    buffer: &mut [u8],
) -> Result<u64, String> {
    let Some(ranges) = donor.ranges.as_deref() else { return Ok(0) };
    let source = crate::partial_donor::open(&donor.path, donor.size)?;
    let mut copied = 0u64;
    for &(offset, length) in ranges {
        let end = offset.checked_add(length).filter(|end| *end <= donor.size)
            .ok_or("Partial range is outside the file")?;
        let mut position = offset;
        while position < end {
            if stop.is_cancelled() {
                return Ok(copied);
            }
            let count = (end - position).min(COPY_BLOCK) as usize;
            let chunk = buffer.get_mut(..count).ok_or("Partial copy buffer is too small")?;
            crate::partial_donor::read_at(&source, position, chunk)?;
            if chunk.iter().any(|byte| *byte != 0) {
                #[cfg(unix)]
                std::os::unix::fs::FileExt::write_all_at(target, chunk, position).map_err(display)?;
                #[cfg(windows)]
                {
                    let mut written = 0usize;
                    while written < count {
                        written += std::os::windows::fs::FileExt::seek_write(
                            target, &chunk[written..count], position + written as u64,
                        ).map_err(display)?;
                    }
                }
                copied += count as u64;
            }
            position += count as u64;
        }
    }
    Ok(copied)
}
