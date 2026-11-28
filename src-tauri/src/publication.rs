// SPDX-License-Identifier: GPL-3.0-or-later
//! Publish a complete sibling temporary file without ever replacing a user file.
//! Native exclusive renames work on FAT/exFAT, unlike hard-link-only publication.
#![cfg_attr(unix, forbid(unsafe_code))]
use cap_fs_ext::{FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use cap_std::fs::{Dir, Metadata, OpenOptions};
use std::{fs::File, io, path::Path};

/// On success the source name is removed. Both paths must be on the same volume.
/// An existing destination (including a dangling symlink) is never replaced.
pub fn publish_noreplace(source: &Path, destination: &Path) -> io::Result<()> {
    platform_publish(source, destination)
}

/// An already-verified regular file, retained open until publication is checked.
///
/// Construct this from the handle whose bytes were verified, never by reopening
/// its mutable pathname. Windows handles must allow FILE_SHARE_DELETE so a
/// file's exclusive rename can run without closing the verified handle. A
/// directory cannot be renamed while any member is open on NTFS, so Windows
/// directory publication releases these handles and re-verifies each member
/// afterwards by identity, length and modification time (metadata only).
pub struct VerifiedFile {
    file: File,
    metadata: Metadata,
    change: ChangeStamp,
}

/// The clocks that move when the bytes of an inode are rewritten. Device,
/// inode and length alone cannot tell a same-inode rewrite apart, and `mtime`
/// can be restored with `utimes`; Unix `ctime` cannot be set from userland.
/// `rename` itself updates `ctime`, so this identity is only meaningful before
/// a rename. A zero/degenerate Unix change time is treated as unprovable and
/// fails closed. Windows `ChangeTime` is read from the open handle when the
/// volume exposes it; it is an additional detection signal, not proof, because
/// a same-user writer can restore it with `SetFileInformationByHandle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ChangeStamp {
    modified: Option<std::time::SystemTime>,
    changed: Option<(i64, i64)>,
}

#[cfg(unix)]
fn change_time(seconds: i64, nanos: i64) -> Option<(i64, i64)> {
    let nanos = u32::try_from(nanos).ok()?;
    if nanos >= 1_000_000_000 || (seconds == 0 && nanos == 0) {
        return None;
    }
    Some((seconds, i64::from(nanos)))
}

#[cfg(any(windows, test))]
fn filetime_change(ticks: i64) -> Option<(i64, i64)> {
    // FILETIME counts 100 ns intervals since 1601-01-01 (UTC).
    let unix_ticks = i128::from(ticks) - 116_444_736_000_000_000;
    if unix_ticks <= 0 {
        return None;
    }
    Some((
        i64::try_from(unix_ticks.div_euclid(10_000_000)).ok()?,
        i64::from((unix_ticks.rem_euclid(10_000_000) * 100) as u32),
    ))
}

#[cfg(windows)]
pub(crate) fn windows_change_time(file: &File) -> Option<(i64, i64)> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        FileBasicInfo, GetFileInformationByHandleEx, FILE_BASIC_INFO,
    };
    let mut info = FILE_BASIC_INFO::default();
    // SAFETY: `file` owns a valid handle for this synchronous query; `info`
    // points to initialized writable storage of exactly the declared size.
    unsafe {
        GetFileInformationByHandleEx(
            HANDLE(file.as_raw_handle()),
            FileBasicInfo,
            (&mut info as *mut FILE_BASIC_INFO).cast(),
            std::mem::size_of::<FILE_BASIC_INFO>() as u32,
        )
    }
    .ok()?;
    filetime_change(info.ChangeTime)
}

impl ChangeStamp {
    fn of_handle(file: &File) -> Self {
        let modified = file.metadata().ok().and_then(|metadata| metadata.modified().ok());
        #[cfg(unix)]
        let changed = file
            .metadata()
            .ok()
            .and_then(|metadata| {
                use std::os::unix::fs::MetadataExt;
                change_time(metadata.ctime(), metadata.ctime_nsec())
            });
        #[cfg(windows)]
        let changed = windows_change_time(file);
        #[cfg(not(any(unix, windows)))]
        let changed = None;
        Self { modified, changed }
    }
}

impl VerifiedFile {
    pub fn new(file: File, expected_size: u64) -> io::Result<Self> {
        let metadata = Metadata::from_file(&file)?;
        if !metadata.is_file()
            || metadata.is_symlink()
            || metadata.len() != expected_size
            || metadata.nlink() != 1
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Verified publication requires a regular file of the expected size without hard links",
            ));
        }
        let change = ChangeStamp::of_handle(&file);
        Ok(Self { file, metadata, change })
    }

    pub fn file(&self) -> &File {
        &self.file
    }

    fn matches_metadata(&self, metadata: &Metadata) -> bool {
        metadata.is_file()
            && !metadata.is_symlink()
            && metadata.len() == self.metadata.len()
            && metadata.dev() == self.metadata.dev()
            && metadata.ino() == self.metadata.ino()
    }

    /// Check a final component without following a symlink. This is a snapshot,
    /// not permission to unlink an entry later without another ownership proof.
    pub fn matches_in(&self, directory: &Dir, name: &Path) -> io::Result<bool> {
        basename(name)?;
        let before = match directory.symlink_metadata(name) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        };
        if !self.matches_metadata(&before) {
            return Ok(false);
        }
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        #[cfg(windows)]
        {
            use cap_std::fs::OpenOptionsExt;
            // Coexist with the retained writable handle. Its own sharing mode
            // governs writer exclusion; this metadata reader must not deny it.
            options.custom_flags(0x0020_0000).share_mode(1 | 2 | 4);
        }
        let opened = match directory.open_with(name, &options) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        };
        if !self.matches_metadata(&opened.metadata()?)
            || !self.matches_metadata(&Metadata::from_file(&self.file)?)
        {
            return Ok(false);
        }
        match directory.symlink_metadata(name) {
            Ok(metadata) => Ok(self.matches_metadata(&metadata)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub fn matches_path(&self, path: &Path) -> io::Result<bool> {
        let (parent, name) = parent_and_name(path)?;
        let directory = Dir::open_ambient_dir(parent, cap_std::ambient_authority())?;
        self.matches_in(&directory, name)
    }

    /// Compare another open handle to the identity this file was verified
    /// under, including the change clock. Used for a digest source that is
    /// copied by pathname: the source handle itself must still describe the
    /// bytes that were hashed. Degenerate Unix change times fail closed.
    pub fn matches_file(&self, file: &File) -> io::Result<bool> {
        let metadata = Metadata::from_file(file)?;
        if !self.matches_metadata(&metadata) || metadata.nlink() != self.metadata.nlink() {
            return Ok(false);
        }
        #[cfg(unix)]
        if self.change.changed.is_none() {
            return Ok(false);
        }
        Ok(ChangeStamp::of_handle(file) == self.change)
    }

    /// Bind a rename to the identity whose bytes were verified: the same
    /// device/inode/length as [`Self::matches_in`] plus the modification and
    /// (Unix) change clocks captured when this handle was verified. A
    /// same-inode rewrite that preserves the name, inode and length therefore
    /// fails closed where a usable change time exists. Do not use this after a
    /// rename: `rename` moves `ctime`. On Windows this is a modification-time
    /// check; the retained handle's sharing mode is the writer exclusion.
    pub fn matches_in_before_rename(&self, directory: &Dir, name: &Path) -> io::Result<bool> {
        #[cfg(unix)]
        if self.change.changed.is_none() {
            return Ok(false);
        }
        if !self.matches_in(directory, name)? {
            return Ok(false);
        }
        basename(name)?;
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        #[cfg(windows)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.custom_flags(0x0020_0000).share_mode(1 | 2 | 4);
        }
        let opened = match directory.open_with(name, &options) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        };
        let opened = opened.into_std();
        Ok(ChangeStamp::of_handle(&opened) == self.change)
    }
}

fn parent_and_name(path: &Path) -> io::Result<(&Path, &Path)> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Publication path has no filename",
        )
    })?;
    Ok((parent, Path::new(name)))
}

/// Commit exclusively, then bind the committed regular entry to the retained
/// verified handle before allowing callers to expose its pathname as complete.
/// A mismatch is an error; no potentially unowned destination is removed.
pub fn publish_verified_noreplace(
    source: &Path,
    destination: &Path,
    verified: &VerifiedFile,
) -> io::Result<()> {
    let (parent, source_name) = parent_and_name(source)?;
    let (destination_parent, destination_name) = parent_and_name(destination)?;
    if parent != destination_parent {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Verified publication requires sibling filenames",
        ));
    }
    let directory = Dir::open_ambient_dir(parent, cap_std::ambient_authority())?;
    publish_verified_noreplace_in(&directory, source_name, destination_name, verified)?;
    if !verified.matches_path(destination)? {
        return Err(publication_changed());
    }
    Ok(())
}

/// [`publish_verified_noreplace`] with the digest-time change clocks checked
/// immediately before the rename. Acquisition finalization persists a journal
/// between hashing and this commit, so the file must still be the exact inode
/// state whose bytes were hashed.
pub fn publish_verified_noreplace_bound(
    source: &Path,
    destination: &Path,
    verified: &VerifiedFile,
) -> io::Result<()> {
    let (parent, source_name) = parent_and_name(source)?;
    let (destination_parent, destination_name) = parent_and_name(destination)?;
    if parent != destination_parent {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Verified publication requires sibling filenames",
        ));
    }
    let directory = Dir::open_ambient_dir(parent, cap_std::ambient_authority())?;
    publish_verified_noreplace_bound_in(&directory, source_name, destination_name, verified)?;
    if !verified.matches_path(destination)? {
        return Err(publication_changed());
    }
    Ok(())
}

pub fn publish_verified_noreplace_in(
    directory: &Dir,
    source: &Path,
    destination: &Path,
    verified: &VerifiedFile,
) -> io::Result<()> {
    publish_verified_with_hook_in(directory, source, destination, verified, || {})
}

/// Commit a file whose bytes were accepted by a caller that needs the rename
/// bound to the exact identity present when the digest pass finished. The
/// pre-rename check includes the modification and (Unix) change clocks, so a
/// same-inode rewrite that restores `mtime` is refused; the ordinary
/// [`publish_verified_noreplace_in`] keeps its historical device/inode/length
/// contract for callers whose handles already exclude writers.
pub fn publish_verified_noreplace_bound_in(
    directory: &Dir,
    source: &Path,
    destination: &Path,
    verified: &VerifiedFile,
) -> io::Result<()> {
    publish_verified_bound_with_hook_in(directory, source, destination, verified, || {})
}

// The hook exercises the exact gap between the last source check and commit.
// Production monomorphizes the empty closure away.
fn publish_verified_bound_with_hook_in(
    directory: &Dir,
    source: &Path,
    destination: &Path,
    verified: &VerifiedFile,
    before_commit: impl FnOnce(),
) -> io::Result<()> {
    basename(source)?;
    basename(destination)?;
    if !verified.matches_in_before_rename(directory, source)? {
        return Err(publication_changed());
    }
    before_commit();
    platform_publish_in(directory, source, destination)?;
    if !verified.matches_in(directory, destination)? {
        return Err(publication_changed());
    }
    Ok(())
}

// The hook exercises the exact gap between the last source check and commit.
// Production monomorphizes the empty closure away.
fn publish_verified_with_hook_in(
    directory: &Dir,
    source: &Path,
    destination: &Path,
    verified: &VerifiedFile,
    before_commit: impl FnOnce(),
) -> io::Result<()> {
    basename(source)?;
    basename(destination)?;
    if !verified.matches_in(directory, source)? {
        return Err(publication_changed());
    }
    before_commit();
    // Never unlink a checked source pathname here: that can delete a
    // substituted file. Native exclusive rename works on FAT/exFAT as well;
    // where a filesystem lacks it, `exclusive_rename_fallback` captures the
    // old name by rename before it inspects and removes it.
    platform_publish_in(directory, source, destination)?;
    if !verified.matches_in(directory, destination)? {
        return Err(publication_changed());
    }
    Ok(())
}

/// Atomically expose a complete owned archive directory. Keep the source
/// capability and every verified child handle until the committed set is bound.
/// No entry is removed on a mismatch, including a raced destination.
#[cfg(not(windows))]
pub fn publish_directory_noreplace_in(
    parent: &Dir,
    source: &Path,
    destination: &Path,
    verified_directory: &Dir,
    files: &[(std::path::PathBuf, VerifiedFile)],
) -> io::Result<()> {
    directory_children(source, destination, files)?;
    let identity = verified_directory.dir_metadata()?;
    let identity = (identity.dev(), identity.ino());
    if files.is_empty() || directory_matches(parent, source, identity, files, true)?.is_none() {
        return Err(publication_changed());
    }
    platform_publish_in(parent, source, destination)?;
    if directory_matches(parent, destination, identity, files, false)?.is_none() {
        return Err(publication_changed());
    }
    Ok(())
}

/// A published directory capability and its re-bound verified children.
#[cfg(windows)]
pub type PublishedDirectory = (Dir, Vec<(std::path::PathBuf, VerifiedFile)>);

/// Atomically expose a complete owned archive directory on Windows.
///
/// Windows cannot rename a directory while handles are open to it or inside
/// it: cap-std directory capabilities never share FILE_SHARE_DELETE (they are
/// partly implemented through their pathname), and NTFS refuses to rename a
/// directory with any open child file (ERROR_ACCESS_DENIED) whatever its share
/// mode. The caller therefore surrenders the directory capability and every
/// verified child handle (and must close any duplicate of them). Before they
/// close, the whole set is checked through `parent` and each handle's identity
/// (volume serial + file index, stable across rename), size and last-write
/// time are recorded. The rename runs on a DELETE handle opened through
/// `parent` that must carry the recorded directory identity, so a directory
/// substituted after release is never published. Each child is then reopened
/// by name inside the destination (no-follow, denying writers) and must match
/// its recorded identity, size and last-write time, and the destination set is
/// re-verified as a whole. On success, returns a capability to the published
/// directory and the re-bound verified children. No entry is removed on a
/// mismatch, including after the commit.
#[cfg(windows)]
pub fn publish_directory_noreplace_in(
    parent: &Dir,
    source: &Path,
    destination: &Path,
    verified_directory: Dir,
    files: Vec<(std::path::PathBuf, VerifiedFile)>,
) -> io::Result<PublishedDirectory> {
    publish_directory_with_hook_in(parent, source, destination, verified_directory, files, || {})
}

// The hook exercises the gap between releasing the caller's handles and
// opening the identity-checked DELETE handle. Production passes an empty closure.
#[cfg(windows)]
fn publish_directory_with_hook_in(
    parent: &Dir,
    source: &Path,
    destination: &Path,
    verified_directory: Dir,
    files: Vec<(std::path::PathBuf, VerifiedFile)>,
    before_commit: impl FnOnce(),
) -> io::Result<PublishedDirectory> {
    use cap_fs_ext::DirExt;
    use cap_std::fs::OpenOptionsExt;
    directory_children(source, destination, &files)?;
    let identity = verified_directory.dir_metadata()?;
    let identity = (identity.dev(), identity.ino());
    // Verify through `parent` while the caller's handles still pin the set;
    // the probe capability is closed when this statement ends.
    if files.is_empty() || directory_matches(parent, source, identity, &files, true)?.is_none() {
        return Err(publication_changed());
    }
    let mut released = Vec::with_capacity(files.len());
    for (name, file) in files {
        // Snapshot at the moment of release; the handle closes as it drops.
        let current = Metadata::from_file(&file.file)?;
        if !file.matches_metadata(&current) || current.modified().is_err() {
            return Err(publication_changed());
        }
        released.push((name, current));
    }
    drop(verified_directory);
    before_commit();
    // Each attempt reopens the source and re-proves its identity.
    crate::storage::retry_transient_sharing(|| {
        platform_publish_entry_in(parent, source, destination, Some(identity))
    })?;
    let published = parent.open_dir_nofollow(destination)?;
    let current = published.dir_metadata()?;
    if current.dev() != identity.0 || current.ino() != identity.1 {
        return Err(publication_changed());
    }
    let mut files = Vec::with_capacity(released.len());
    for (name, recorded) in released {
        let mut options = OpenOptions::new();
        // Deny writers from here on, as the verified handles did.
        options.read(true).follow(FollowSymlinks::No).custom_flags(0x0020_0000).share_mode(1 | 4);
        let file = published.open_with(&name, &options)?;
        let current = file.metadata()?;
        if !current.is_file() || current.is_symlink() || current.len() != recorded.len()
            || current.dev() != recorded.dev() || current.ino() != recorded.ino()
            || current.modified().ok() != recorded.modified().ok()
        {
            return Err(publication_changed());
        }
        files.push((name, VerifiedFile::new(file.into_std(), recorded.len())?));
    }
    drop(published);
    let published = directory_matches(parent, destination, identity, &files, false)?.ok_or_else(publication_changed)?;
    Ok((published, files))
}

fn directory_children(
    source: &Path,
    destination: &Path,
    files: &[(std::path::PathBuf, VerifiedFile)],
) -> io::Result<()> {
    basename(source)?;
    basename(destination)?;
    let mut names = std::collections::HashSet::with_capacity(files.len());
    for (name, _) in files {
        basename(name)?;
        if !names.insert(name) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Duplicate directory publication child"));
        }
    }
    Ok(())
}

/// The no-follow directory `name` in `parent` has `identity` and contains
/// exactly the verified children; returns the capability that was checked.
fn directory_matches(
    parent: &Dir,
    name: &Path,
    identity: (u64, u64),
    files: &[(std::path::PathBuf, VerifiedFile)],
    before_rename: bool,
) -> io::Result<Option<Dir>> {
    use cap_fs_ext::DirExt;
    let metadata = parent.symlink_metadata(name)?;
    if !metadata.is_dir() || metadata.is_symlink()
        || metadata.dev() != identity.0 || metadata.ino() != identity.1
    {
        return Ok(None);
    }
    let opened = parent.open_dir_nofollow(name)?;
    let current = opened.dir_metadata()?;
    if current.dev() != identity.0 || current.ino() != identity.1
        || opened.entries()?.try_fold(0usize, |count, entry| entry.map(|_| count + 1))? != files.len()
    {
        return Ok(None);
    }
    for (name, file) in files {
        let matches = if before_rename {
            file.matches_in_before_rename(&opened, name)?
        } else {
            file.matches_in(&opened, name)?
        };
        if !matches {
            return Ok(None);
        }
    }
    Ok(Some(opened))
}

fn publication_changed() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Publication entry no longer identifies the verified regular file",
    )
}

fn basename(path: &Path) -> io::Result<()> {
    let mut components = path.components();
    if !matches!(components.next(), Some(std::path::Component::Normal(_)))
        || components.next().is_some()
        || path.file_name() != Some(path.as_os_str())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Confined publication requires a single filename",
        ));
    }
    Ok(())
}

#[cfg(any(unix, test))]
pub(crate) fn rename_entry_noreplace(directory: &Dir, source: &Path, destination: &Path) -> io::Result<()> {
    basename(source)?;
    basename(destination)?;
    platform_publish_in(directory, source, destination)
}

/// The kernel enforces exclusivity in one operation relative to the supplied
/// directories. Return the original OS error if the filesystem lacks support.
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub(crate) fn rename_noreplace_at(
    source_dir: impl std::os::fd::AsFd,
    source: &std::ffi::CStr,
    destination_dir: impl std::os::fd::AsFd,
    destination: &std::ffi::CStr,
) -> io::Result<()> {
    rustix::fs::renameat_with(
        source_dir,
        source,
        destination_dir,
        destination,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(io::Error::from)
}

/// The kernel enforces exclusivity in one operation relative to the supplied
/// directories. Filesystems without `RENAME_NOREPLACE` (NFS, FUSE servers
/// without rename2) take [`exclusive_rename_fallback`], which is exclusive too.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub(crate) fn rename_noreplace_at(
    source_dir: impl std::os::fd::AsFd,
    source: &std::ffi::CStr,
    destination_dir: impl std::os::fd::AsFd,
    destination: &std::ffi::CStr,
) -> io::Result<()> {
    rename_noreplace_with(
        source_dir.as_fd(),
        source,
        destination_dir.as_fd(),
        destination,
        native_rename_noreplace,
        native_link,
        || {},
    )
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn native_rename_noreplace(
    source_dir: std::os::fd::BorrowedFd<'_>,
    source: &std::ffi::CStr,
    destination_dir: std::os::fd::BorrowedFd<'_>,
    destination: &std::ffi::CStr,
) -> io::Result<()> {
    rustix::fs::renameat_with(
        source_dir,
        source,
        destination_dir,
        destination,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(io::Error::from)
}

/// `linkat` without `AT_SYMLINK_FOLLOW`: a symlink source is linked itself.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn native_link(
    source_dir: std::os::fd::BorrowedFd<'_>,
    source: &std::ffi::CStr,
    destination_dir: std::os::fd::BorrowedFd<'_>,
    destination: &std::ffi::CStr,
) -> io::Result<()> {
    rustix::fs::linkat(source_dir, source, destination_dir, destination, rustix::fs::AtFlags::empty())
        .map_err(io::Error::from)
}

/// Only these mean "this filesystem cannot do an exclusive rename". Every
/// other failure, `EEXIST` above all, is the answer and is returned as it is.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn exclusive_rename_unsupported(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::ENOSYS | libc::EINVAL | libc::EOPNOTSUPP)
    )
}

/// The rename and link primitives are parameters so tests can model a
/// filesystem without `RENAME_NOREPLACE` or without hard links; `after_link`
/// exercises the gap between publishing the new name and retiring the old one.
/// Production monomorphizes the native calls and the empty closure in.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn rename_noreplace_with(
    source_dir: std::os::fd::BorrowedFd<'_>,
    source: &std::ffi::CStr,
    destination_dir: std::os::fd::BorrowedFd<'_>,
    destination: &std::ffi::CStr,
    rename: impl FnOnce(std::os::fd::BorrowedFd<'_>, &std::ffi::CStr, std::os::fd::BorrowedFd<'_>, &std::ffi::CStr) -> io::Result<()>,
    link: impl FnOnce(std::os::fd::BorrowedFd<'_>, &std::ffi::CStr, std::os::fd::BorrowedFd<'_>, &std::ffi::CStr) -> io::Result<()>,
    after_link: impl FnOnce(),
) -> io::Result<()> {
    let Err(error) = rename(source_dir, source, destination_dir, destination) else {
        return Ok(());
    };
    if !exclusive_rename_unsupported(&error) {
        return Err(error);
    }
    exclusive_rename_fallback(source_dir, source, destination_dir, destination, link, after_link)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn entry_identity(
    directory: std::os::fd::BorrowedFd<'_>,
    name: &std::ffi::CStr,
) -> io::Result<(rustix::fs::FileType, u64, u64)> {
    let stat = rustix::fs::statat(directory, name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW)?;
    // The field widths differ between Linux architectures.
    #[allow(clippy::unnecessary_cast, clippy::useless_conversion)]
    Ok((rustix::fs::FileType::from_raw_mode(stat.st_mode), stat.st_dev as u64, stat.st_ino as u64))
}

/// An exclusive move for filesystems whose rename cannot refuse to replace.
/// Every step goes through the two retained directory descriptors and never
/// follows a symlink in the final component.
///
/// A directory is moved onto an exclusively created (`mkdir`) placeholder:
/// `rename(2)` replaces a directory only while it is empty, so nothing but an
/// empty directory can ever be displaced.
///
/// Anything else is published with `link(2)`, which is atomic and fails with
/// `EEXIST` rather than replace. The old name is then retired without ever
/// unlinking a checked public pathname: it is first captured by a rename to a
/// private random name in the same directory, and only the captured entry is
/// inspected. It is unlinked only while it is the same inode the destination
/// name holds, so the file stays reachable there; a different entry (the
/// source name was reused after the link) is put back under its old name.
///
/// A filesystem with neither exclusive rename nor hard links is refused: an
/// `O_EXCL` placeholder followed by a plain rename could replace a file that
/// took the placeholder's name in between.
///
/// A crash between the link and the unlink leaves two names for one file
/// (the source name or the private `.kiwix-renamed-*` name); no step can leave
/// fewer names than it found.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn exclusive_rename_fallback(
    source_dir: std::os::fd::BorrowedFd<'_>,
    source: &std::ffi::CStr,
    destination_dir: std::os::fd::BorrowedFd<'_>,
    destination: &std::ffi::CStr,
    link: impl FnOnce(std::os::fd::BorrowedFd<'_>, &std::ffi::CStr, std::os::fd::BorrowedFd<'_>, &std::ffi::CStr) -> io::Result<()>,
    after_link: impl FnOnce(),
) -> io::Result<()> {
    use rustix::fs::{AtFlags, FileType};
    let (kind, device, inode) = entry_identity(source_dir, source)?;
    if kind == FileType::Directory {
        rustix::fs::mkdirat(destination_dir, destination, rustix::fs::Mode::from_raw_mode(0o700))?;
        return rustix::fs::renameat(source_dir, source, destination_dir, destination).map_err(|error| {
            // Only an empty directory can be removed here; one that gained
            // entries in the meantime is no longer the placeholder and stays.
            let _ = rustix::fs::unlinkat(destination_dir, destination, AtFlags::REMOVEDIR);
            if error == rustix::io::Errno::NOTEMPTY {
                return io::Error::from_raw_os_error(libc::EEXIST);
            }
            io::Error::from(error)
        });
    }
    if let Err(error) = link(source_dir, source, destination_dir, destination) {
        let unsupported = match error.raw_os_error() {
            Some(libc::EPERM | libc::ENOSYS | libc::EOPNOTSUPP) => true,
            // Android denies hard links to applications with EACCES.
            Some(libc::EACCES) => cfg!(target_os = "android"),
            _ => false,
        };
        if !unsupported {
            return Err(error);
        }
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "This storage location cannot move a file into place without the risk of overwriting another file: it supports neither an exclusive rename nor hard links ({error}). Choose a folder on a different drive"
            ),
        ));
    }
    after_link();
    // What the link published. A destination that already changed again is
    // not evidence about the source name, so nothing is removed then.
    let (_, linked_device, linked_inode) = entry_identity(destination_dir, destination).map_err(|error| {
        io::Error::other(format!(
            "The moved file's new name changed before its old name could be removed; the old name was kept: {error}"
        ))
    })?;
    let private = format!(".kiwix-renamed-{}", uuid::Uuid::new_v4());
    let captured = std::ffi::CString::new(private.clone())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Publication path contains NUL"))?;
    match rustix::fs::statat(source_dir, captured.as_c_str(), AtFlags::SYMLINK_NOFOLLOW) {
        Err(rustix::io::Errno::NOENT) => {}
        Ok(_) => return Err(io::Error::new(io::ErrorKind::AlreadyExists, "Private rename name is already in use; the old name was kept")),
        Err(error) => return Err(error.into()),
    }
    // Capture whatever the source name holds now, atomically.
    match rustix::fs::renameat(source_dir, source, source_dir, captured.as_c_str()) {
        Ok(()) => {}
        // The old name is already gone; the destination holds the file.
        Err(rustix::io::Errno::NOENT) => return Ok(()),
        Err(error) => {
            return Err(io::Error::other(format!(
                "The file was published under its new name but its old name could not be removed: {error}"
            )))
        }
    }
    let (captured_kind, captured_device, captured_inode) = entry_identity(source_dir, captured.as_c_str())?;
    if (captured_device, captured_inode) == (linked_device, linked_inode) {
        // The same file is reachable through the destination name.
        return rustix::fs::unlinkat(source_dir, captured.as_c_str(), AtFlags::empty()).map_err(|error| {
            io::Error::other(format!(
                "The file was published under its new name; removing its old name, kept as {private}, failed: {error}"
            ))
        });
    }
    // The source name was reused after the link (or the destination changed).
    // The captured entry is not the published file: put it back, exclusively.
    let restored = if captured_kind == FileType::Directory {
        // A directory can only displace an empty directory.
        rustix::fs::renameat(source_dir, captured.as_c_str(), source_dir, source)
    } else {
        rustix::fs::linkat(source_dir, captured.as_c_str(), source_dir, source, AtFlags::empty())
            .and_then(|()| rustix::fs::unlinkat(source_dir, captured.as_c_str(), AtFlags::empty()))
    };
    if let Err(error) = restored {
        return Err(io::Error::other(format!(
            "An entry that replaced the moved file's old name was preserved as {private}: {error}"
        )));
    }
    if (linked_device, linked_inode) == (device, inode) {
        // The original file moved; its old name now belongs to a newer entry.
        return Ok(());
    }
    Err(io::Error::other(
        "The file changed while it was being moved; both names were kept",
    ))
}

#[cfg(any(target_os = "macos", target_os = "ios", target_os = "linux", target_os = "android"))]
fn platform_publish_in(
    directory: &cap_std::fs::Dir,
    source: &Path,
    destination: &Path,
) -> io::Result<()> {
    let from = c_path(source)?;
    let to = c_path(destination)?;
    rename_noreplace_at(directory, &from, directory, &to)
}

#[cfg(windows)]
fn platform_publish_in(
    directory: &cap_std::fs::Dir,
    source: &Path,
    destination: &Path,
) -> io::Result<()> {
    platform_publish_entry_in(directory, source, destination, None)
}

/// `directory_identity` selects a directory source, which must be exactly the
/// (volume serial, file index) object; `None` publishes a regular file.
#[cfg(windows)]
fn platform_publish_entry_in(
    directory: &cap_std::fs::Dir,
    source: &Path,
    destination: &Path,
    directory_identity: Option<(u64, u64)>,
) -> io::Result<()> {
    let is_directory = directory_identity.is_some();
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
    use cap_std::fs::{OpenOptions, OpenOptionsExt};
    use std::os::windows::{ffi::OsStrExt, io::AsRawHandle};
    // Prevent alternate data streams and Win32 path reinterpretation. These
    // names are not paths, and neither source nor target may traverse a link.
    for name in [source, destination] {
        if name
            .as_os_str()
            .encode_wide()
            .any(|c| matches!(c, 0 | 47 | 58 | 92))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid confined publication filename",
            ));
        }
    }
    let mut options = OpenOptions::new();
    options.access_mode(0x0001_0000 | 0x0000_0080); // DELETE | FILE_READ_ATTRIBUTES
    options.follow(FollowSymlinks::No);
    options.custom_flags(0x0020_0000 | if is_directory { 0x0200_0000 } else { 0 });
    let source = directory.open_with(source, &options)?;
    let metadata = source.metadata()?;
    if metadata.is_symlink()
        || if is_directory { !metadata.is_dir() } else { !metadata.is_file() }
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Publication source must be a regular file",
        ));
    }
    // The DELETE handle itself is what gets renamed, so checking its identity
    // binds the commit to the verified directory even though the caller's
    // capability had to be closed first.
    if directory_identity.is_some_and(|(dev, ino)| metadata.dev() != dev || metadata.ino() != ino) {
        return Err(publication_changed());
    }
    // RootDirectory binds the target to the capability, even if its old
    // filesystem pathname has been renamed or replaced.
    crate::storage::rename_into_directory_noreplace(
        source.as_raw_handle(),
        directory.as_raw_handle(),
        destination.as_os_str(),
    )
}

#[cfg(unix)]
fn c_path(path: &Path) -> io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Publication path contains NUL"))
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn platform_publish(source: &Path, destination: &Path) -> io::Result<()> {
    let source = c_path(source)?;
    let destination = c_path(destination)?;
    rename_noreplace_at(rustix::fs::CWD, &source, rustix::fs::CWD, &destination)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn platform_publish(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::fd::AsFd;
    let from = c_path(source)?;
    let to = c_path(destination)?;
    let Err(error) = native_rename_noreplace(rustix::fs::CWD, &from, rustix::fs::CWD, &to) else {
        return Ok(());
    };
    if !exclusive_rename_unsupported(&error) {
        return Err(error);
    }
    // Old kernels/filesystems may lack exclusive rename. The one fallback works
    // through directory descriptors, so anchor both names to their parents.
    let (source_parent, source_name) = parent_and_name(source)?;
    let (destination_parent, destination_name) = parent_and_name(destination)?;
    let source_dir = Dir::open_ambient_dir(source_parent, cap_std::ambient_authority())?;
    let destination_dir = Dir::open_ambient_dir(destination_parent, cap_std::ambient_authority())?;
    exclusive_rename_fallback(
        source_dir.as_fd(),
        &c_path(source_name)?,
        destination_dir.as_fd(),
        &c_path(destination_name)?,
        native_link,
        || {},
    )
}

#[cfg(windows)]
fn platform_publish(source: &Path, destination: &Path) -> io::Result<()> {
    fn resolved(path: &Path) -> io::Result<std::path::PathBuf> {
        // Canonicalize only the parent: destination need not exist, and the
        // final component must not be followed if it is a symlink. Canonical
        // parents also give the Win32 extended-length path form for long names.
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let name = path.file_name().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Publication path has no filename",
            )
        })?;
        Ok(std::fs::canonicalize(parent)?.join(name))
    }
    // Flags zero forbid replacement and cross-volume copy/delete emulation.
    let from = resolved(source)?;
    let to = resolved(destination)?;
    crate::storage::move_file(&from, &to, crate::storage::MoveKind::NoReplace)
}

#[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "linux", target_os = "android", windows)))]
compile_error!(
    "Atomic publication requires a native exclusive-rename implementation on this target"
);

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::io::Read;

    #[test]
    fn filetime_change_rejects_degenerate_values_and_converts_epoch_ticks() {
        const EPOCH: i64 = 116_444_736_000_000_000;
        assert_eq!(filetime_change(0), None);
        assert_eq!(filetime_change(-1), None);
        assert_eq!(filetime_change(EPOCH), None);
        assert_eq!(filetime_change(EPOCH + 1), Some((0, 100)));
        assert_eq!(filetime_change(EPOCH + 10_000_000), Some((1, 0)));
    }

    fn verified_fixture(directory: &Path) -> (VerifiedFile, Vec<u8>) {
        let mut bytes = vec![b'v'; 128];
        bytes[..4].copy_from_slice(b"ZIM\x04");
        let source = directory.join("partial");
        std::fs::write(&source, &bytes).unwrap();
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x0020_0000).share_mode(1 | 4);
        }
        let mut file = options.open(&source).unwrap();
        let mut verified_bytes = Vec::new();
        file.read_to_end(&mut verified_bytes).unwrap();
        assert_eq!(Sha256::digest(&verified_bytes), Sha256::digest(&bytes));
        (VerifiedFile::new(file, bytes.len() as u64).unwrap(), bytes)
    }

    #[cfg(not(windows))]
    #[test]
    fn complete_directory_publication_preserves_collisions_and_rejects_replacements() {
        for replacement in [false, true] {
            let root = std::env::temp_dir().join(format!("kiwix-directory-publication-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&root).unwrap();
            std::fs::create_dir(root.join("staging")).unwrap();
            let (verified, bytes) = verified_fixture(&root.join("staging"));
            let parent = Dir::open_ambient_dir(&root, cap_std::ambient_authority()).unwrap();
            let source = parent.open_dir("staging").unwrap();
            let files = vec![(std::path::PathBuf::from("partial"), verified)];
            if replacement {
                std::fs::rename(root.join("staging"), root.join("retained")).unwrap();
                std::fs::create_dir(root.join("staging")).unwrap();
                std::fs::write(root.join("staging/partial"), &bytes).unwrap();
                assert!(publish_directory_noreplace_in(&parent, Path::new("staging"), Path::new("complete"), &source, &files).is_err());
                assert!(!root.join("complete").exists());
                assert_eq!(std::fs::read(root.join("staging/partial")).unwrap(), bytes);
                assert_eq!(std::fs::read(root.join("retained/partial")).unwrap(), bytes);
            } else {
                std::fs::create_dir(root.join("complete")).unwrap();
                std::fs::write(root.join("complete/sentinel"), b"user data").unwrap();
                assert!(publish_directory_noreplace_in(&parent, Path::new("staging"), Path::new("complete"), &source, &files).is_err());
                assert_eq!(std::fs::read(root.join("complete/sentinel")).unwrap(), b"user data");
                std::fs::remove_file(root.join("complete/sentinel")).unwrap();
                std::fs::remove_dir(root.join("complete")).unwrap();
                publish_directory_noreplace_in(&parent, Path::new("staging"), Path::new("complete"), &source, &files).unwrap();
                assert!(files[0].1.matches_in(&parent.open_dir("complete").unwrap(), Path::new("partial")).unwrap());
            }
            drop(files); drop(source); drop(parent);
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    /// Re-verify an existing member, as a caller would after a refused commit
    /// consumed its Windows handles.
    #[cfg(windows)]
    fn reverified(path: &Path, bytes: &[u8]) -> VerifiedFile {
        use std::os::windows::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new().read(true).custom_flags(0x0020_0000).share_mode(1 | 4).open(path).unwrap();
        let mut read = Vec::new();
        file.read_to_end(&mut read).unwrap();
        assert_eq!(read, bytes);
        VerifiedFile::new(file, bytes.len() as u64).unwrap()
    }

    #[cfg(windows)]
    #[test]
    fn complete_directory_publication_preserves_collisions_and_rejects_replacements() {
        for replacement in [false, true] {
            let root = std::env::temp_dir().join(format!("kiwix-directory-publication-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&root).unwrap();
            // NTFS cannot rename a directory while a member is open, so the
            // verified set is created under its displaced name instead.
            let verified_name = if replacement { "retained" } else { "staging" };
            std::fs::create_dir(root.join(verified_name)).unwrap();
            let (verified, bytes) = verified_fixture(&root.join(verified_name));
            let parent = Dir::open_ambient_dir(&root, cap_std::ambient_authority()).unwrap();
            let source = parent.open_dir(verified_name).unwrap();
            let files = vec![(std::path::PathBuf::from("partial"), verified)];
            if replacement {
                std::fs::create_dir(root.join("staging")).unwrap();
                std::fs::write(root.join("staging/partial"), &bytes).unwrap();
                let error = publish_directory_noreplace_in(&parent, Path::new("staging"), Path::new("complete"), source, files).err().unwrap();
                assert_eq!(error.kind(), io::ErrorKind::InvalidData);
                assert!(!root.join("complete").exists());
                assert_eq!(std::fs::read(root.join("staging/partial")).unwrap(), bytes);
                assert_eq!(std::fs::read(root.join("retained/partial")).unwrap(), bytes);
            } else {
                std::fs::create_dir(root.join("complete")).unwrap();
                std::fs::write(root.join("complete/sentinel"), b"user data").unwrap();
                let error = publish_directory_noreplace_in(&parent, Path::new("staging"), Path::new("complete"), source, files).err().unwrap();
                assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
                assert_eq!(std::fs::read(root.join("complete/sentinel")).unwrap(), b"user data");
                assert_eq!(std::fs::read(root.join("staging/partial")).unwrap(), bytes);
                std::fs::remove_file(root.join("complete/sentinel")).unwrap();
                std::fs::remove_dir(root.join("complete")).unwrap();
                let files = vec![(std::path::PathBuf::from("partial"), reverified(&root.join("staging/partial"), &bytes))];
                let (published, files) = publish_directory_noreplace_in(&parent, Path::new("staging"), Path::new("complete"), parent.open_dir("staging").unwrap(), files).unwrap();
                assert!(files[0].1.matches_in(&published, Path::new("partial")).unwrap());
                assert!(files[0].1.matches_in(&parent.open_dir("complete").unwrap(), Path::new("partial")).unwrap());
                assert!(!root.join("staging").exists());
                assert_eq!(std::fs::read(root.join("complete/partial")).unwrap(), bytes);
            }
            drop(parent);
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    /// Every Windows handle to the set is released for the rename, so the
    /// commit binds by identity: a directory substituted in that gap, even with
    /// byte-identical members, is never published, and the genuine set still is.
    #[cfg(windows)]
    #[test]
    fn released_directory_set_cannot_publish_a_substituted_directory() {
        let root = std::env::temp_dir().join(format!("kiwix-directory-publication-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join("staging")).unwrap();
        let (verified, bytes) = verified_fixture(&root.join("staging"));
        let parent = Dir::open_ambient_dir(&root, cap_std::ambient_authority()).unwrap();
        let files = vec![(std::path::PathBuf::from("partial"), verified)];
        // While the caller holds the set, Windows refuses any directory rename.
        assert!(std::fs::rename(root.join("staging"), root.join("blocked")).is_err());
        let source = parent.open_dir("staging").unwrap();
        let error = publish_directory_with_hook_in(&parent, Path::new("staging"), Path::new("complete"), source, files, || {
            // Every handle is closed here: displace the verified directory,
            // then plant an identical-looking set under its name.
            std::fs::rename(root.join("staging"), root.join("retained")).unwrap();
            std::fs::create_dir(root.join("staging")).unwrap();
            std::fs::write(root.join("staging/partial"), &bytes).unwrap();
        })
        .err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(!root.join("complete").exists());
        assert_eq!(std::fs::read(root.join("staging/partial")).unwrap(), bytes);
        let files = vec![(std::path::PathBuf::from("partial"), reverified(&root.join("retained/partial"), &bytes))];
        let (published, files) = publish_directory_noreplace_in(&parent, Path::new("retained"), Path::new("complete"), parent.open_dir("retained").unwrap(), files).unwrap();
        assert!(files[0].1.matches_in(&published, Path::new("partial")).unwrap());
        assert!(!root.join("retained").exists());
        drop((published, files, parent));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A member rewritten in place while the Windows handles are released is
    /// refused after the commit: its last-write time no longer matches the
    /// verified handle's. Nothing is deleted on that refusal.
    #[cfg(windows)]
    #[test]
    fn released_directory_set_rejects_a_member_rewritten_in_the_gap() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("kiwix-directory-publication-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join("staging")).unwrap();
        let (verified, bytes) = verified_fixture(&root.join("staging"));
        let parent = Dir::open_ambient_dir(&root, cap_std::ambient_authority()).unwrap();
        let files = vec![(std::path::PathBuf::from("partial"), verified)];
        let mut edited = bytes.clone();
        *edited.last_mut().unwrap() ^= 0xff;
        let error = publish_directory_with_hook_in(&parent, Path::new("staging"), Path::new("complete"), parent.open_dir("staging").unwrap(), files, || {
            // Outlast the last-write timestamp granularity, then rewrite in place.
            std::thread::sleep(std::time::Duration::from_millis(50));
            let mut writer = std::fs::OpenOptions::new().write(true).open(root.join("staging/partial")).unwrap();
            writer.write_all(&edited).unwrap();
            writer.sync_all().unwrap();
        })
        .err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(std::fs::read(root.join("complete/partial")).unwrap(), edited);
        drop(parent);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn degenerate_change_times_are_not_publication_proof() {
        assert_eq!(change_time(0, 0), None);
        assert_eq!(change_time(1, 1_000_000_000), None);
        assert!(change_time(1, 0).is_some());
    }

    #[cfg(unix)]
    #[test]
    fn verified_file_handle_identity_rejects_a_same_inode_rewrite() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("kiwix-publication-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let (verified, bytes) = verified_fixture(&root);
        let source = root.join("partial");
        assert!(verified.matches_file(&std::fs::File::open(&source).unwrap()).unwrap());
        let modified = std::fs::metadata(&source).unwrap().modified().unwrap();
        let mut edited = bytes.clone();
        *edited.get_mut(4).unwrap() ^= 0xff;
        let mut writer = std::fs::OpenOptions::new().write(true).open(&source).unwrap();
        writer.write_all(&edited).unwrap();
        writer.sync_all().unwrap();
        drop(writer);
        std::fs::OpenOptions::new().write(true).open(&source).unwrap()
            .set_modified(modified).unwrap();
        assert!(!verified.matches_file(&std::fs::File::open(&source).unwrap()).unwrap());
        drop(verified);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn pre_rename_identity_rejects_a_same_inode_rewrite_with_restored_mtime() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("kiwix-publication-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let (verified, bytes) = verified_fixture(&root);
        let source = root.join("partial");
        let destination = root.join("complete");
        let modified = std::fs::metadata(&source).unwrap().modified().unwrap();
        let mut edited = bytes.clone();
        *edited.get_mut(4).unwrap() ^= 0xff;
        let mut writer = std::fs::OpenOptions::new().write(true).open(&source).unwrap();
        writer.write_all(&edited).unwrap();
        writer.sync_all().unwrap();
        drop(writer);
        std::fs::OpenOptions::new().write(true).open(&source).unwrap()
            .set_modified(modified).unwrap();
        let parent = Dir::open_ambient_dir(&root, cap_std::ambient_authority()).unwrap();
        assert!(!verified.matches_in_before_rename(&parent, Path::new("partial")).unwrap());
        let error = publish_verified_noreplace_bound(&source, &destination, &verified).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(!destination.exists());
        assert_eq!(std::fs::read(&source).unwrap(), edited);
        drop((parent, verified));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn verified_publication_retains_the_hashed_inode_without_clobbering() {
        let root = std::env::temp_dir().join(format!("kiwix-publication-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let (verified, bytes) = verified_fixture(&root);
        let source = root.join("partial");
        let destination = root.join("complete");
        std::fs::write(&destination, b"user archive").unwrap();
        assert!(publish_verified_noreplace(&source, &destination, &verified).is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"user archive");
        assert!(verified.matches_path(&source).unwrap());
        std::fs::remove_file(&destination).unwrap();
        publish_verified_noreplace(&source, &destination, &verified).unwrap();
        assert!(verified.matches_path(&destination).unwrap());
        assert_eq!(std::fs::read(&destination).unwrap(), bytes);
        assert!(!source.exists());
        drop(verified);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn source_replacement_after_precheck_cannot_publish_as_the_verified_inode() {
        for identical_bytes in [false, true] {
            let root =
                std::env::temp_dir().join(format!("kiwix-publication-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&root).unwrap();
            let (verified, bytes) = verified_fixture(&root);
            let directory = Dir::open_ambient_dir(&root, cap_std::ambient_authority()).unwrap();
            let mut foreign = if identical_bytes {
                bytes.clone()
            } else {
                vec![b'f'; bytes.len()]
            };
            foreign[..4].copy_from_slice(b"ZIM\x04");
            let verified_digest = Sha256::digest(&bytes);
            assert_eq!(Sha256::digest(&foreign) == verified_digest, identical_bytes);
            let error = publish_verified_with_hook_in(
                &directory,
                Path::new("partial"),
                Path::new("complete"),
                &verified,
                || {
                    // Substitute a different, same-sized inode AFTER the exact
                    // production check, including content-identical substitutes.
                    directory
                        .rename("partial", &directory, "retained-verified")
                        .unwrap();
                    directory.write("partial", &foreign).unwrap();
                },
            )
            .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert_eq!(directory.read("complete").unwrap(), foreign);
            assert_eq!(directory.read("retained-verified").unwrap(), bytes);
            assert!(!verified
                .matches_in(&directory, Path::new("complete"))
                .unwrap());
            assert!(verified
                .matches_in(&directory, Path::new("retained-verified"))
                .unwrap());
            assert_eq!(
                Sha256::digest(directory.read("retained-verified").unwrap()),
                verified_digest,
            );
            drop(verified);
            drop(directory);
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn source_symlink_substitution_is_rejected_without_deleting_foreign_entries() {
        let root = std::env::temp_dir().join(format!("kiwix-publication-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let (verified, bytes) = verified_fixture(&root);
        let directory = Dir::open_ambient_dir(&root, cap_std::ambient_authority()).unwrap();
        // Even identical content at the symlink target cannot replace the
        // verified object: publication must compare identity without following.
        directory.write("foreign", &bytes).unwrap();
        let error = publish_verified_with_hook_in(
            &directory,
            Path::new("partial"),
            Path::new("complete"),
            &verified,
            || {
                directory
                    .rename("partial", &directory, "retained-verified")
                    .unwrap();
                std::os::unix::fs::symlink("foreign", root.join("partial")).unwrap();
            },
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            directory.read_link("complete").unwrap(),
            Path::new("foreign")
        );
        assert_eq!(directory.read("foreign").unwrap(), bytes);
        assert!(verified
            .matches_in(&directory, Path::new("retained-verified"))
            .unwrap());
        assert!(!verified
            .matches_in(&directory, Path::new("complete"))
            .unwrap());
        drop(verified);
        drop(directory);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn exclusive_publication_preserves_existing_and_moves_new_file() {
        let directory =
            std::env::temp_dir().join(format!("kiwix-publication-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let source = directory.join("partial");
        let destination = directory.join("complete");
        std::fs::write(&source, b"new").unwrap();
        std::fs::write(&destination, b"existing").unwrap();
        assert!(publish_noreplace(&source, &destination).is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"existing");
        assert_eq!(std::fs::read(&source).unwrap(), b"new");
        std::fs::remove_file(&destination).unwrap();
        publish_noreplace(&source, &destination).unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"new");
        assert!(!source.exists());
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn confined_publication_moves_without_clobber_and_rejects_paths() {
        let root = std::env::temp_dir().join(format!("kiwix-publication-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let directory =
            cap_std::fs::Dir::open_ambient_dir(&root, cap_std::ambient_authority()).unwrap();
        directory.write("partial", b"new").unwrap();
        let verified = VerifiedFile::new(directory.open("partial").unwrap().into_std(), 3).unwrap();
        directory.write("complete", b"existing").unwrap();
        for name in ["../escape", "child/file", ".", "/absolute"] {
            assert!(publish_verified_noreplace_in(
                &directory,
                Path::new("partial"),
                Path::new(name),
                &verified
            )
            .is_err());
        }
        assert!(publish_verified_noreplace_in(
            &directory,
            Path::new("partial"),
            Path::new("complete"),
            &verified
        )
        .is_err());
        assert_eq!(directory.read("complete").unwrap(), b"existing");
        directory.remove_file("complete").unwrap();
        publish_verified_noreplace_in(
            &directory,
            Path::new("partial"),
            Path::new("complete"),
            &verified,
        )
        .unwrap();
        assert_eq!(directory.read("complete").unwrap(), b"new");
        assert!(!directory.exists("partial"));
        drop(verified);
        drop(directory);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn confined_publication_ignores_a_swapped_parent_path() {
        let root = std::env::temp_dir().join(format!("kiwix-publication-{}", uuid::Uuid::new_v4()));
        let original = root.join("destination");
        let moved = root.join("moved");
        let outside = root.join("outside");
        std::fs::create_dir_all(&original).unwrap();
        std::fs::create_dir(&outside).unwrap();
        let directory =
            cap_std::fs::Dir::open_ambient_dir(&original, cap_std::ambient_authority()).unwrap();
        directory.write("partial", b"new").unwrap();
        let verified = VerifiedFile::new(directory.open("partial").unwrap().into_std(), 3).unwrap();
        std::fs::write(outside.join("complete"), b"user").unwrap();
        std::fs::rename(&original, &moved).unwrap();
        std::os::unix::fs::symlink(&outside, &original).unwrap();
        publish_verified_noreplace_in(
            &directory,
            Path::new("partial"),
            Path::new("complete"),
            &verified,
        )
        .unwrap();
        assert_eq!(std::fs::read(moved.join("complete")).unwrap(), b"new");
        assert_eq!(std::fs::read(outside.join("complete")).unwrap(), b"user");
        drop(verified);
        drop(directory);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "linux", target_os = "android"))]
    #[test]
    fn cross_directory_rename_stays_with_both_handles_after_parent_replacement() {
        let root = std::env::temp_dir().join(format!("kiwix-publication-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let parent = Dir::open_ambient_dir(&root, cap_std::ambient_authority()).unwrap();
        parent.create_dir("from").unwrap();
        parent.create_dir("to").unwrap();
        let from = parent.open_dir("from").unwrap();
        let to = parent.open_dir("to").unwrap();
        from.write("partial", b"verified").unwrap();
        to.write("complete", b"existing").unwrap();

        parent.rename("from", &parent, "held-from").unwrap();
        parent.rename("to", &parent, "held-to").unwrap();
        parent.create_dir("from").unwrap();
        parent.create_dir("to").unwrap();
        parent.write("from/partial", b"unrelated source").unwrap();
        parent.write("to/complete", b"unrelated destination").unwrap();

        let error = rename_noreplace_at(&from, c"partial", &to, c"complete").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(from.read("partial").unwrap(), b"verified");
        assert_eq!(to.read("complete").unwrap(), b"existing");
        to.remove_file("complete").unwrap();
        rename_noreplace_at(&from, c"partial", &to, c"complete").unwrap();
        assert_eq!(from.open("partial").unwrap_err().kind(), io::ErrorKind::NotFound);
        assert_eq!(to.read("complete").unwrap(), b"verified");
        assert_eq!(parent.read("from/partial").unwrap(), b"unrelated source");
        assert_eq!(parent.read("to/complete").unwrap(), b"unrelated destination");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A filesystem whose rename rejects `RENAME_NOREPLACE` with `errno`, as
    /// NFS and FUSE servers without rename2 do. Real NFS/FUSE mounts are not
    /// available to the test suite.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn rename_without_flag(
        errno: i32,
        from: &Dir,
        source: &std::ffi::CStr,
        to: &Dir,
        destination: &std::ffi::CStr,
        after_link: impl FnOnce(),
    ) -> io::Result<()> {
        use std::os::fd::AsFd;
        rename_noreplace_with(
            from.as_fd(),
            source,
            to.as_fd(),
            destination,
            |_, _, _, _| Err(io::Error::from_raw_os_error(errno)),
            native_link,
            after_link,
        )
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn fallback_root() -> (std::path::PathBuf, Dir) {
        let root = std::env::temp_dir().join(format!("kiwix-publication-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let directory = Dir::open_ambient_dir(&root, cap_std::ambient_authority()).unwrap();
        (root, directory)
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn names(root: &Path) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(root).unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap()).collect();
        names.sort();
        names
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn unsupported_exclusive_rename_falls_back_to_an_exclusive_move() {
        for errno in [libc::EINVAL, libc::ENOSYS, libc::EOPNOTSUPP] {
            let (root, directory) = fallback_root();
            let (verified, bytes) = verified_fixture(&root);
            directory.create_dir("library").unwrap();
            let library = directory.open_dir("library").unwrap();
            // Within one directory, then across two retained directories.
            rename_without_flag(errno, &directory, c"partial", &directory, c"complete", || {}).unwrap();
            assert_eq!(names(&root), ["complete", "library"]);
            assert!(verified.matches_in(&directory, Path::new("complete")).unwrap());
            rename_without_flag(errno, &directory, c"complete", &library, c"book.zim", || {}).unwrap();
            assert_eq!(names(&root), ["library"]);
            assert_eq!(names(&root.join("library")), ["book.zim"]);
            assert!(verified.matches_in(&library, Path::new("book.zim")).unwrap());
            assert_eq!(library.read("book.zim").unwrap(), bytes);
            // One name again, so later ownership checks still accept the file.
            assert_eq!(Metadata::from_file(verified.file()).unwrap().nlink(), 1);
            drop((verified, library, directory));
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn fallback_preserves_an_existing_destination_as_already_exists() {
        let (root, directory) = fallback_root();
        directory.write("partial", b"new").unwrap();
        directory.write("complete", b"existing").unwrap();
        std::os::unix::fs::symlink("missing-user-target", root.join("dangling")).unwrap();
        directory.create_dir("occupied").unwrap();
        for destination in [c"complete", c"dangling", c"occupied"] {
            let error = rename_without_flag(libc::EINVAL, &directory, c"partial", &directory, destination, || {
                panic!("nothing was linked, so nothing may be retired")
            })
            .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "{error}");
        }
        assert_eq!(directory.read("partial").unwrap(), b"new");
        assert_eq!(directory.read("complete").unwrap(), b"existing");
        assert_eq!(directory.read_link("dangling").unwrap(), Path::new("missing-user-target"));
        assert_eq!(names(&root), ["complete", "dangling", "occupied", "partial"]);
        drop(directory);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn fallback_is_taken_only_when_the_filesystem_lacks_the_flag() {
        use std::os::fd::AsFd;
        let (root, directory) = fallback_root();
        directory.write("partial", b"new").unwrap();
        for errno in [libc::EEXIST, libc::EACCES, libc::EXDEV, libc::ENOENT, libc::EIO, libc::EROFS, libc::ENOSPC] {
            let error = rename_noreplace_with(
                directory.as_fd(), c"partial", directory.as_fd(), c"complete",
                |_, _, _, _| Err(io::Error::from_raw_os_error(errno)),
                |_, _, _, _| panic!("errno {errno} must not reach the link fallback"),
                || panic!("errno {errno} must not reach the link fallback"),
            )
            .unwrap_err();
            assert_eq!(error.raw_os_error(), Some(errno));
        }
        assert_eq!(directory.read("partial").unwrap(), b"new");
        assert_eq!(names(&root), ["partial"]);
        // The real kernel answer for an occupied name is likewise final.
        directory.write("complete", b"existing").unwrap();
        let error = rename_noreplace_at(&directory, c"partial", &directory, c"complete").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(directory.read("complete").unwrap(), b"existing");
        drop(directory);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn fallback_never_unlinks_a_source_name_that_was_reused_after_the_link() {
        // (how the old name was reused, whether the original keeps another name)
        for case in ["renamed-aside", "renamed-over", "directory", "removed"] {
            let (root, directory) = fallback_root();
            directory.write("partial", b"verified archive").unwrap();
            directory.write("incoming", b"somebody else's file").unwrap();
            let original = directory.symlink_metadata("partial").unwrap();
            rename_without_flag(libc::EINVAL, &directory, c"partial", &directory, c"complete", || match case {
                "renamed-aside" => {
                    directory.rename("partial", &directory, "kept-aside").unwrap();
                    directory.rename("incoming", &directory, "partial").unwrap();
                }
                "renamed-over" => directory.rename("incoming", &directory, "partial").unwrap(),
                "directory" => {
                    directory.remove_file("partial").unwrap();
                    directory.create_dir("partial").unwrap();
                    directory.write("partial/child", b"somebody else's folder").unwrap();
                }
                _ => directory.remove_file("partial").unwrap(),
            })
            .unwrap();
            // The original was published exactly once, under the new name.
            let published = directory.symlink_metadata("complete").unwrap();
            assert_eq!((published.dev(), published.ino()), (original.dev(), original.ino()), "{case}");
            assert_eq!(directory.read("complete").unwrap(), b"verified archive", "{case}");
            let expected: &[&str] = match case {
                "renamed-aside" => {
                    assert_eq!(directory.read("partial").unwrap(), b"somebody else's file");
                    assert_eq!(directory.read("kept-aside").unwrap(), b"verified archive");
                    &["complete", "kept-aside", "partial"]
                }
                "renamed-over" => {
                    assert_eq!(directory.read("partial").unwrap(), b"somebody else's file");
                    &["complete", "partial"]
                }
                "directory" => {
                    assert_eq!(directory.read("partial/child").unwrap(), b"somebody else's folder");
                    &["complete", "incoming", "partial"]
                }
                _ => &["complete", "incoming"],
            };
            // Nothing was deleted, and no private capture name is left behind.
            assert_eq!(names(&root), expected, "{case}");
            drop(directory);
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn fallback_keeps_the_source_when_the_destination_changes_after_the_link() {
        for replaced in [false, true] {
            let (root, directory) = fallback_root();
            directory.write("partial", b"verified archive").unwrap();
            let error = rename_without_flag(libc::EINVAL, &directory, c"partial", &directory, c"complete", || {
                directory.rename("complete", &directory, "moved-away").unwrap();
                if replaced {
                    directory.write("complete", b"another program's file").unwrap();
                }
            })
            .unwrap_err();
            assert_ne!(error.kind(), io::ErrorKind::AlreadyExists, "{error}");
            // The old name still holds the file; nothing was unlinked.
            assert_eq!(directory.read("partial").unwrap(), b"verified archive");
            assert_eq!(directory.read("moved-away").unwrap(), b"verified archive");
            if replaced {
                assert_eq!(directory.read("complete").unwrap(), b"another program's file");
                assert_eq!(names(&root), ["complete", "moved-away", "partial"]);
            } else {
                assert_eq!(names(&root), ["moved-away", "partial"]);
            }
            drop(directory);
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn fallback_moves_a_directory_only_onto_its_own_exclusive_placeholder() {
        let (root, directory) = fallback_root();
        directory.create_dir("staging").unwrap();
        directory.write("staging/book.zim", b"archive").unwrap();
        let original = directory.symlink_metadata("staging").unwrap();
        directory.create_dir("empty").unwrap();
        directory.create_dir("full").unwrap();
        directory.write("full/user.txt", b"user data").unwrap();
        directory.write("file", b"user file").unwrap();
        for destination in [c"empty", c"full", c"file"] {
            let error = rename_without_flag(libc::EINVAL, &directory, c"staging", &directory, destination, || {}).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "{error}");
        }
        assert_eq!(directory.read("full/user.txt").unwrap(), b"user data");
        assert_eq!(directory.read("file").unwrap(), b"user file");
        assert_eq!(names(&root), ["empty", "file", "full", "staging"]);
        rename_without_flag(libc::EINVAL, &directory, c"staging", &directory, c"complete", || {}).unwrap();
        let published = directory.symlink_metadata("complete").unwrap();
        assert_eq!((published.dev(), published.ino()), (original.dev(), original.ino()));
        assert_eq!(directory.read("complete/book.zim").unwrap(), b"archive");
        assert_eq!(names(&root), ["complete", "empty", "file", "full"]);
        drop(directory);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn fallback_refuses_a_filesystem_without_hard_links_instead_of_risking_an_overwrite() {
        use std::os::fd::AsFd;
        for errno in [libc::EPERM, libc::ENOSYS, libc::EOPNOTSUPP] {
            let (root, directory) = fallback_root();
            directory.write("partial", b"verified archive").unwrap();
            let error = rename_noreplace_with(
                directory.as_fd(), c"partial", directory.as_fd(), c"complete",
                |_, _, _, _| Err(io::Error::from_raw_os_error(libc::EINVAL)),
                |_, _, _, _| Err(io::Error::from_raw_os_error(errno)),
                || panic!("nothing was linked, so nothing may be retired"),
            )
            .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::Unsupported);
            assert!(error.to_string().contains("neither an exclusive rename nor hard links"), "{error}");
            assert_eq!(directory.read("partial").unwrap(), b"verified archive");
            assert_eq!(names(&root), ["partial"]);
            drop(directory);
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn dangling_symlink_is_an_existing_destination() {
        let directory =
            std::env::temp_dir().join(format!("kiwix-publication-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let source = directory.join("partial");
        let destination = directory.join("complete");
        std::fs::write(&source, b"new").unwrap();
        std::os::unix::fs::symlink("missing-user-target", &destination).unwrap();
        assert!(publish_noreplace(&source, &destination).is_err());
        assert_eq!(
            std::fs::read_link(&destination).unwrap(),
            Path::new("missing-user-target")
        );
        assert_eq!(std::fs::read(&source).unwrap(), b"new");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn nul_paths_are_rejected_before_the_rename_fallback() {
        use std::os::unix::ffi::OsStrExt;
        let nul = Path::new(std::ffi::OsStr::from_bytes(b"bad\0name"));
        for error in [
            publish_noreplace(Path::new("missing"), nul).unwrap_err(),
            publish_noreplace(nul, Path::new("missing")).unwrap_err(),
        ] {
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
            assert_eq!(error.to_string(), "Publication path contains NUL");
        }
        let directory =
            std::env::temp_dir().join(format!("kiwix-publication-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let dir = Dir::open_ambient_dir(&directory, cap_std::ambient_authority()).unwrap();
        let error = rename_entry_noreplace(&dir, Path::new("partial"), nul).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(error.to_string(), "Publication path contains NUL");
        std::fs::remove_dir_all(directory).unwrap();
    }
}
