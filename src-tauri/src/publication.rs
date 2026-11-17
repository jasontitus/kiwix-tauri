// SPDX-License-Identifier: GPL-3.0-or-later
//! Publish a complete sibling temporary file without ever replacing a user file.
//! Native exclusive renames work on FAT/exFAT, unlike hard-link-only publication.
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
        Ok(Self { file, metadata })
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

pub fn publish_verified_noreplace_in(
    directory: &Dir,
    source: &Path,
    destination: &Path,
    verified: &VerifiedFile,
) -> io::Result<()> {
    publish_verified_with_hook_in(directory, source, destination, verified, || {})
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
    // Do not use link/unlink here: unlinking a mutable source name can delete a
    // substituted file. Native exclusive rename works on FAT/exFAT as well.
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
    if files.is_empty() || directory_matches(parent, source, identity, files)?.is_none() {
        return Err(publication_changed());
    }
    platform_publish_in(parent, source, destination)?;
    if directory_matches(parent, destination, identity, files)?.is_none() {
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
    if files.is_empty() || directory_matches(parent, source, identity, &files)?.is_none() {
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
    let published = directory_matches(parent, destination, identity, &files)?.ok_or_else(publication_changed)?;
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
        if !file.matches_in(&opened, name)? {
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

#[cfg(any(target_os = "macos", target_os = "ios", target_os = "linux", target_os = "android"))]
fn platform_publish_in(
    directory: &cap_std::fs::Dir,
    source: &Path,
    destination: &Path,
) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    let from = c_path(source)?;
    let to = c_path(destination)?;
    let fd = directory.as_raw_fd();
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    let result =
        unsafe { libc::renameatx_np(fd, from.as_ptr(), fd, to.as_ptr(), libc::RENAME_EXCL) };
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            fd,
            from.as_ptr(),
            fd,
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    Err(error)
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
    // renameatx_np(2): RENAME_EXCL is enforced by the filesystem, not a racy
    // existence check. Do not substitute plain rename if a volume rejects it.
    let result = unsafe {
        libc::renameatx_np(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn platform_publish(source: &Path, destination: &Path) -> io::Result<()> {
    let from = c_path(source)?;
    let to = c_path(destination)?;
    // Calling the syscall directly also supports libc versions predating the
    // renameat2 wrapper. Modern Linux exFAT implements RENAME_NOREPLACE.
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if !matches!(
        error.raw_os_error(),
        Some(libc::ENOSYS | libc::EINVAL | libc::EOPNOTSUPP)
    ) {
        return Err(error);
    }
    // Old kernels/filesystems may lack exclusive rename. link(2) is itself
    // atomic and exclusive; it is safe for regular files only. Unsupported
    // filesystems return their real error rather than falling back to clobbering.
    if !std::fs::symlink_metadata(source)?.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Publication source must be a regular file",
        ));
    }
    std::fs::hard_link(source, destination)?;
    // Publication has committed even if unlink fails; report that error and
    // leave the destination intact so durable callers can recover by digest.
    std::fs::remove_file(source)
}

#[cfg(windows)]
fn platform_publish(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    fn wide(path: &Path) -> io::Result<Vec<u16>> {
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
        let path = std::fs::canonicalize(parent)?.join(name);
        let mut value: Vec<u16> = path.as_os_str().encode_wide().collect();
        if value.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Publication path contains NUL",
            ));
        }
        value.push(0);
        Ok(value)
    }
    let from = wide(source)?;
    let to = wide(destination)?;
    // Flags zero forbids replacement and cross-volume copy/delete emulation.
    let result = unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0) };
    if result != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
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
}
