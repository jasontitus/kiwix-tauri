// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit trusted-shell filesystem actions. Never dispatch these from archive messages.
//! Migration journals are confined to an owned profile directory. Failed moves retain
//! their journal and every remaining source/destination part for startup recovery.
//! Cross-volume cleanup captures sources in journal-owned private staging before
//! checking their identities and unlinking; failed first renames keep the library
//! pointed at the intact source.
#![cfg_attr(unix, forbid(unsafe_code))]
use crate::{
    archive::Archive,
    store::{Book, Store},
};
use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use cap_std::fs::{Dir, DirBuilder, Metadata, OpenOptions};
use crate::fs_ext::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::File,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

const MAX_PARTS: usize = 26 * 26;
const MAX_JOURNAL: u64 = 1024 * 1024;
const MAX_ATTACHMENT: u64 = 512 * 1024 * 1024;
const CHUNK: u64 = 1024 * 1024;
const EXPORT_RETENTION: Duration = Duration::from_secs(30 * 24 * 60 * 60);
static OPERATIONS: Mutex<()> = Mutex::new(());
static EXPORT_SESSION_STARTED: LazyLock<SystemTime> = LazyLock::new(SystemTime::now);
/// Test-only interleaving point after the staged archive's identity check.
/// Bound to the installing thread, so a parallel test's removal cannot run it.
#[cfg(test)]
#[allow(clippy::type_complexity)]
static DUPLICATE_INTAKE_HOOK: Mutex<Option<(std::thread::ThreadId, Box<dyn FnOnce() + Send>)>> =
    Mutex::new(None);
/// Test-only: fail the Nth exclusive rename on one thread, so the staged-delete
/// restore path can be exercised without a second volume.
#[cfg(test)]
static MOVE_FAILURE_AT: Mutex<Option<(std::thread::ThreadId, u32)>> = Mutex::new(None);
/// Test-only: the free space one thread sees on every volume, so the space
/// check can be exercised without filling a disk.
#[cfg(test)]
static FREE_SPACE_OVERRIDE: Mutex<Option<(std::thread::ThreadId, u64)>> = Mutex::new(None);
/// Test-only: the real Trash / Recycle Bin is refused unless a test opts in
/// (none does), so a test cannot leave entries in the developer's Trash.
#[cfg(test)]
static TEST_ALLOWS_REAL_TRASH: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Identity {
    device: u64,
    inode: u64,
}
impl Identity {
    fn of(metadata: &Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct FileStamp {
    identity: Identity,
    size: u64,
    modified_ns: u128,
}
impl FileStamp {
    fn of(metadata: &Metadata) -> Result<Self, String> {
        if !metadata.is_file() || metadata.nlink() != 1 {
            return Err("Archive parts must be regular files without hard links".into());
        }
        Self::read(metadata)
    }
    /// Readers need a regular retained inode, not exclusive link ownership.
    fn read(metadata: &Metadata) -> Result<Self, String> {
        if !metadata.is_file() {
            return Err("Archive parts must be regular files".into());
        }
        let modified_ns = metadata
            .modified()
            .map_err(err)?
            .into_std()
            .duration_since(UNIX_EPOCH)
            .map_err(err)?
            .as_nanos();
        Ok(Self {
            identity: Identity::of(metadata),
            size: metadata.len(),
            modified_ns,
        })
    }
}
struct Directory {
    path: PathBuf,
    dir: Dir,
    identity: Identity,
}
impl Directory {
    /// Walk each component through a directory capability without following links.
    fn open(path: &Path) -> Result<Self, String> {
        let dir = crate::storage::open_directory_nofollow(path).map_err(|error|
            crate::sandbox::explain_folder_error(path, &error).unwrap_or_else(|| err(error)))?;
        let identity = Identity::of(&dir.dir_metadata().map_err(err)?);
        Ok(Self {
            path: path.to_path_buf(),
            dir,
            identity,
        })
    }
    fn unchanged(&self) -> Result<(), String> {
        let current = Self::open(&self.path)?;
        if current.identity != self.identity {
            return Err(format!(
                "Directory identity changed: {}",
                self.path.display()
            ));
        }
        Ok(())
    }
    fn sync(&self) -> Result<(), String> {
        #[cfg(unix)]
        crate::storage::sync_cap_directory(&self.dir).map_err(err)?;
        Ok(())
    }
    fn open_file(&self, name: &str) -> Result<File, String> {
        filename(name)?;
        self.open_read_file(name)
    }
    fn open_read_file(&self, name: &str) -> Result<File, String> {
        read_filename(name)?;
        let before = self.dir.symlink_metadata(name).map_err(err)?;
        if !before.is_file() || before.file_type().is_symlink() {
            return Err(format!("Not a regular archive file: {name}"));
        }
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        #[cfg(windows)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.custom_flags(0x0020_0000).share_mode(1 | 4);
        }
        let file = self.dir.open_with(name, &options).map_err(err)?;
        if Identity::of(&before) != Identity::of(&file.metadata().map_err(err)?) {
            return Err("Archive file changed while opening".into());
        }
        Ok(file.into_std())
    }
    fn stamp(&self, name: &str) -> Result<FileStamp, String> {
        let file = self.open_file(name)?;
        FileStamp::of(&cap_std::fs::File::from_std(file).metadata().map_err(err)?)
    }
    fn read_stamp(&self, name: &str) -> Result<FileStamp, String> {
        FileStamp::read(&Metadata::from_file(&self.open_read_file(name)?).map_err(err)?)
    }
    fn absent(&self, name: &str) -> Result<bool, String> {
        filename(name)?;
        match self.dir.symlink_metadata(name) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(true),
            Ok(_) => Ok(false),
            Err(e) => Err(err(e)),
        }
    }
}
fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}
fn filename(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 240
        || name
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'))
        || name.ends_with(['.', ' '])
        || Path::new(name).components().count() != 1
        || !matches!(
            Path::new(name).components().next(),
            Some(Component::Normal(_))
        )
    {
        return Err("Invalid single-component archive filename".into());
    }
    Ok(())
}
/// Reading preserves platform-valid names already accepted by import; it does
/// not grant mutation/export authority or traverse another path component.
fn read_filename(name: &str) -> Result<(), String> {
    if name.contains('\0') || Path::new(name).components().count() != 1
        || !matches!(Path::new(name).components().next(), Some(Component::Normal(_))) {
        return Err("Invalid single-component archive filename".into());
    }
    #[cfg(windows)]
    if name.contains(':') {
        return Err("Archive streams are not regular archive filenames".into());
    }
    Ok(())
}
#[cfg(mobile)]
fn basename(path: &Path) -> Result<String, String> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Archive filename must be valid Unicode")?;
    filename(name)?;
    Ok(name.to_string())
}
/// The file name of a library archive for an operation that moves, deletes or
/// shares the file itself. Reading accepts every platform-valid name; these
/// operations keep the stricter rule, and say which part of it the name fails
/// and what the person can do instead.
fn archive_basename(path: &Path) -> Result<String, String> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Archive filename must be valid Unicode")?;
    if filename(name).is_ok() {
        return Ok(name.to_string());
    }
    let reason = if name.len() > 240 {
        "is longer than 240 bytes"
    } else if name.chars().any(|c| c.is_control() || matches!(c, '\\' | ':')) {
        "contains a colon, a backslash or a control character"
    } else if name.ends_with(['.', ' ']) {
        "ends with a dot or a space"
    } else {
        return Err("Invalid single-component archive filename".into());
    };
    Err(format!(
        "This archive's file name {reason}, so Kiwix will not move or delete the file itself. Rename the file and use Locate to find it again, or remove the archive from the library and delete the file yourself"
    ))
}
fn private_child(parent: &Directory, name: &str, create: bool) -> Result<Directory, String> {
    filename(name)?;
    if create {
        #[cfg_attr(not(unix), allow(unused_mut, reason = "the builder is only configured with a Unix mode"))]
        let mut builder = DirBuilder::new();
        #[cfg(unix)]
        {
            use cap_std::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match parent.dir.create_dir_with(name, &builder) {
            Ok(()) => parent.sync()?,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(err(e)),
        }
    }
    let dir = parent.dir.open_dir_nofollow(name).map_err(err)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let metadata = dir
            .try_clone()
            .map_err(err)?
            .into_std_file()
            .metadata()
            .map_err(err)?;
        if metadata.uid() != crate::fs_ext::effective_uid() || metadata.mode() & 0o077 != 0 {
            return Err(
                "Owned library operation directory must be private to this user (0700)".into(),
            );
        }
    }
    let identity = Identity::of(&dir.dir_metadata().map_err(err)?);
    Ok(Directory {
        path: parent.path.join(name),
        dir,
        identity,
    })
}
/// Re-open an owned private child that was released, adopting it only if it is
/// still the directory whose identity was verified before the release.
fn reopen_child(parent: &Directory, name: &str, identity: &Identity) -> Result<Directory, String> {
    let child = private_child(parent, name, false)?;
    if &child.identity != identity {
        return Err(format!(
            "Owned operation directory was replaced; nothing was taken from {}",
            child.path.display()
        ));
    }
    Ok(child)
}
/// Windows refuses to remove, rename or trash a directory while any handle to
/// it lacks `FILE_SHARE_DELETE`, and cap-std directory capabilities never share
/// delete (they resolve operations through the directory's path). Release the
/// capability, then prove through the still-held parent that the name resolves
/// to the identity the capability verified. The caller then acts on the name
/// through the parent, as the Unix path already does.
#[cfg(windows)]
fn release_directory(
    parent: &Directory,
    slot: &mut Option<Directory>,
    name: &str,
) -> Result<(), String> {
    let identity = slot
        .take()
        .ok_or("Owned operation directory capability unavailable")?
        .identity;
    let current = parent.dir.symlink_metadata(name).map_err(err)?;
    if !current.is_dir() || current.file_type().is_symlink() || Identity::of(&current) != identity {
        return Err("Owned operation directory was replaced".into());
    }
    Ok(())
}
/// Windows: release the capability and delete the now-empty directory through
/// a handle that itself proves the released identity, so a directory swapped
/// in under the same name is never removed (the Unix path keeps `remove_dir`).
#[cfg(windows)]
fn remove_owned_directory(
    parent: &Directory,
    slot: &mut Option<Directory>,
    name: &str,
) -> Result<(), String> {
    let identity = slot
        .take()
        .ok_or("Owned operation directory capability unavailable")?
        .identity;
    crate::storage::remove_empty_directory_verified(
        &parent.dir,
        Path::new(name),
        |metadata| Identity::of(metadata) == identity,
        "Owned operation directory was replaced; it was preserved",
    )
    .map_err(err)
}
fn create_file(directory: &Directory, name: &str) -> Result<cap_std::fs::File, String> {
    filename(name)?;
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = directory.dir.open_with(name, &options).map_err(err)?;
    crate::mobile_backup::exclude_payload(&file, &directory.path.join(name))?;
    Ok(file)
}
fn copy_hashed(mut input: impl Read, output: &mut cap_std::fs::File) -> Result<(u64, String), String> {
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 128 * 1024];
    let mut total = 0u64;
    loop {
        let length = input.read(&mut buffer).map_err(err)?;
        if length == 0 {
            break;
        }
        hash.update(&buffer[..length]);
        output.write_all(&buffer[..length]).map_err(err)?;
        total += length as u64;
    }
    Ok((total, hex::encode(hash.finalize())))
}

fn digest(
    directory: &Directory,
    name: &str,
    expected: Option<&FileStamp>,
) -> Result<(FileStamp, String), String> {
    let file = directory.open_file(name)?;
    FileExt::try_flock_shared(&file).map_err(|e| format!("Archive is busy: {e}"))?;
    let mut file = cap_std::fs::File::from_std(file);
    let before = FileStamp::of(&file.metadata().map_err(err)?)?;
    if expected.is_some_and(|expected| expected != &before) {
        return Err(format!("File identity changed: {name}"));
    }
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 128 * 1024];
    let mut total = 0u64;
    let mut limited = (&mut file).take(before.size.checked_add(1).ok_or("Archive part too large")?);
    loop {
        let length = limited.read(&mut buffer).map_err(err)?;
        if length == 0 {
            break;
        }
        hash.update(&buffer[..length]);
        total += length as u64;
    }
    if total != before.size {
        return Err("Archive changed size while hashing".into());
    }
    if FileStamp::of(&file.metadata().map_err(err)?)? != before || directory.stamp(name)? != before
    {
        return Err(format!("File changed while checking: {name}"));
    }
    Ok((before, hex::encode(hash.finalize())))
}

fn archive_suffix(name: &str, suffix: &str) -> bool {
    name.get(name.len().saturating_sub(suffix.len())..)
        .is_some_and(|extension| extension.eq_ignore_ascii_case(suffix))
}

fn split_part_matches(primary: &str, index: usize, name: &str) -> bool {
    let base = &primary.as_bytes()[..primary.len() - 2];
    let suffix = [b'a' + (index / 26) as u8, b'a' + (index % 26) as u8];
    name.len() == base.len() + 2
        && name.as_bytes()[..base.len()].eq_ignore_ascii_case(base)
        && name.as_bytes()[base.len()..].eq_ignore_ascii_case(&suffix)
}

/// Exact ZIM part names, bounded by the on-disk checksum position, not a prefix
/// glob. A similarly prefixed .bak or a surplus .zimac can never be deleted.
fn part_names(directory: &Directory, primary: &str) -> Result<Vec<String>, String> {
    filename(primary)?;
    let names = read_part_names(directory, primary, false)?;
    for name in &names {
        directory.stamp(name)?;
    }
    Ok(names)
}

fn read_part_names(directory: &Directory, primary: &str, allow_legacy_single: bool) -> Result<Vec<String>, String> {
    read_filename(primary)?;
    let split = archive_suffix(primary, ".zimaa");
    if !split && !archive_suffix(primary, ".zim") {
        return Err("Select a .zim or first .zimaa archive part".into());
    }
    // Native providers preserve names, including mixed extension case. Resolve
    // each exact declared part to its actual directory entry, rejecting case
    // collisions rather than choosing or merging two possible cohorts.
    let mut split_names = std::collections::HashMap::new();
    if split {
        let base = &primary.as_bytes()[..primary.len() - 2];
        for entry in directory.dir.entries().map_err(err)? {
            let entry = entry.map_err(err)?;
            let filename = entry.file_name();
            let Some(name) = filename.to_str() else { continue; };
            let bytes = name.as_bytes();
            if bytes.len() != base.len() + 2 || !bytes[..base.len()].eq_ignore_ascii_case(base)
                || !bytes[base.len()..].iter().all(u8::is_ascii_alphabetic) {
                continue;
            }
            let suffix = [bytes[base.len()].to_ascii_lowercase(), bytes[base.len() + 1].to_ascii_lowercase()];
            split_names.entry(suffix).and_modify(|saved| *saved = None).or_insert_with(|| Some(name.to_owned()));
        }
    }
    let mut header = [0u8; 80];
    let mut header_bytes = 0;
    let mut expected = None;
    let mut total = 0u64;
    let mut names = Vec::new();
    for index in 0..if split { MAX_PARTS } else { 1 } {
        let name = if split {
            let suffix = [b'a' + (index / 26) as u8, b'a' + (index % 26) as u8];
            split_names.remove(&suffix).ok_or("Archive is missing a declared split part")?
                .ok_or("Split archive has ambiguous case-colliding part names")?
        } else {
            primary.to_string()
        };
        if index == 0 && name != primary {
            return Err("The exact selected archive primary is missing".into());
        }
        let stamp = directory.read_stamp(&name)?;
        if stamp.size == 0 {
            return Err("Empty split archive part".into());
        }
        if header_bytes < header.len() {
            let mut file = directory.open_read_file(&name)?;
            if FileStamp::read(&Metadata::from_file(&file).map_err(err)?)? != stamp {
                return Err("Archive header part changed before reading".into());
            }
            while header_bytes < header.len() {
                let count = file.read(&mut header[header_bytes..]).map_err(|e| format!("Reading ZIM header: {e}"))?;
                if count == 0 { break; }
                header_bytes += count;
            }
            if FileStamp::read(&Metadata::from_file(&file).map_err(err)?)? != stamp || directory.read_stamp(&name)? != stamp {
                return Err("Archive header part changed while reading".into());
            }
            // A legacy single file has a MIME table at byte 72 and no checksum
            // offset. Read admission can use the exact file size. Destructive
            // operations and split archives still require a declared boundary.
            if header_bytes >= 72 && &header[..4] == b"ZIM\x04"
                && u64::from_le_bytes(header[56..64].try_into().map_err(err)?) == 72 {
                return if allow_legacy_single && !split { Ok(vec![name]) }
                else { Err("A legacy archive has no declared checksum boundary".into()) };
            }
            if header_bytes == header.len() {
                if &header[..4] != b"ZIM\x04" { return Err("File is not a ZIM archive".into()); }
                expected = Some(u64::from_le_bytes(header[72..80].try_into().map_err(err)?)
                    .checked_add(16).filter(|size| *size >= 96).ok_or("Invalid ZIM checksum position")?);
            }
        }
        total = total
            .checked_add(stamp.size)
            .ok_or("Archive part lengths overflow")?;
        if expected.is_some_and(|expected| total > expected) {
            return Err("Archive parts exceed their declared checksum boundary".into());
        }
        names.push(name);
        if expected == Some(total) {
            return Ok(names);
        }
    }
    Err("Archive is missing parts, exceeds the 676-part bound, or has trailing data".into())
}
const IDENTITY_MISMATCH: &str =
    "Kiwix could not verify this archive, so no files were deleted. Remove it from your library instead";

fn stamp_parts(
    directory: &Directory,
    names: &[String],
) -> Result<Vec<FileStamp>, String> {
    let stamps: Vec<_> = names
        .iter()
        .map(|name| directory.stamp(name))
        .collect::<Result<_, _>>()?;
    directory.unchanged()?;
    for (name, stamp) in names.iter().zip(&stamps) {
        if directory.stamp(name)? != *stamp {
            return Err("Archive changed during verification".into());
        }
    }
    Ok(stamps)
}

/// Ownership proof for destructive actions: the same directory, the exact part
/// cohort declared by the archive's own checksum boundary, and every part still
/// carrying the identity captured before the operation. Deliberately reads no
/// archive bytes, so a 3 GB archive is proven in milliseconds instead of two
/// full MD5 passes; a corrupt archive the user asked to delete stays deletable.
fn verified_parts(
    directory: &Directory,
    primary: &str,
    uuid: &str,
) -> Result<Vec<(String, FileStamp)>, String> {
    directory.unchanged()?;
    let names = part_names(directory, primary)?;
    let stamps = stamp_parts(directory, &names)?;
    let parts: Vec<_> = names.into_iter().zip(stamps).collect();
    if open_stamped_parts(directory, &parts)?.metadata()?.id != uuid {
        return Err(IDENTITY_MISMATCH.into());
    }
    directory.unchanged()?;
    Ok(parts)
}

fn open_stamped_parts(directory: &Directory, parts: &[(String, FileStamp)]) -> Result<Archive, String> {
    let mut files = Vec::with_capacity(parts.len());
    for (name, expected) in parts {
        let file = directory.open_file(name)?;
        if FileStamp::of(&Metadata::from_file(&file).map_err(err)?)? != *expected {
            return Err("Archive changed while retaining its exact part set".into());
        }
        files.push((name.clone(), file));
    }
    Archive::open_files(&directory.path, &files)
}

/// `verified_parts` plus libzim's full content checksum. Reserved for operations
/// whose own contract is byte-level validation (library relocation, journal
/// recovery); never for an interactive delete.
fn verified(
    directory: &Directory,
    primary: &str,
    uuid: &str,
) -> Result<Vec<(String, FileStamp)>, String> {
    let parts = verified_parts(directory, primary, uuid)?;
    let archive = open_stamped_parts(directory, &parts)?;
    if archive.metadata()?.id != uuid || !archive.check_integrity()? {
        return Err(IDENTITY_MISMATCH.into());
    }
    directory.unchanged()?;
    for (name, stamp) in &parts {
        if directory.stamp(name)? != *stamp {
            return Err("Archive changed during verification".into());
        }
    }
    Ok(parts)
}

/// Exact files a library action would touch. Identity only: the delete
/// confirmation must not stall on a multi-gigabyte checksum.
pub fn archive_files(book: &Book) -> Result<Vec<PathBuf>, String> {
    let path = Path::new(&book.path);
    let directory = Directory::open(path.parent().ok_or("Archive has no parent directory")?)?;
    Ok(verified_parts(&directory, &archive_basename(path)?, &book.metadata.id)?
        .into_iter()
        .map(|(name, _)| directory.path.join(name))
        .collect())
}

/// `archive_files` with a full content checksum, for the library relocation
/// gate that must refuse to move a damaged archive.
#[cfg(any(mobile, test))]
pub fn verified_archive_files(book: &Book) -> Result<Vec<PathBuf>, String> {
    let path = Path::new(&book.path);
    let directory = Directory::open(path.parent().ok_or("Archive has no parent directory")?)?;
    Ok(verified(&directory, &archive_basename(path)?, &book.metadata.id)?
        .into_iter()
        .map(|(name, _)| directory.path.join(name))
        .collect())
}

/// Remove the exact staged copy of an archive that resolved to an installed
/// book. The cohort comes from the archive's own declared part boundary and
/// every candidate must still carry the installed archive's identity, so a
/// different archive with the same stem (for example a split set admitted in
/// the same batch) is never touched.
#[cfg(any(mobile, test))]
pub(crate) fn remove_duplicate_intake_copy(path: &str, uuid: &str) -> Result<(), String> {
    let path = Path::new(path);
    let directory =
        Directory::open(path.parent().ok_or("Duplicate archive has no parent directory")?)?;
    let primary = archive_basename(path)?;
    if directory.absent(&primary)? {
        return Ok(());
    }
    let names = if archive_suffix(&primary, ".zim") || archive_suffix(&primary, ".zimaa") {
        // A canonical primary proves its own cohort by checksum boundary. A
        // failed proof removes nothing: an orphaned part is safer than a
        // deleted neighbour.
        part_names(&directory, &primary)?
    } else {
        vec![primary.clone()]
    };
    // Capture the stamps before anything is opened, then prove that the
    // handles actually opened still carry those identities. The old order
    // (open and check the id, then stamp by pathname) deleted a replacement
    // that appeared in between.
    let stamps = names
        .iter()
        .map(|name| directory.stamp(name))
        .collect::<Result<Vec<_>, _>>()?;
    let mut files = Vec::with_capacity(names.len());
    for (name, stamp) in names.iter().zip(&stamps) {
        let file = directory.open_file(name)?;
        if FileStamp::of(&Metadata::from_file(&file).map_err(err)?)? != *stamp {
            return Err("This staged copy changed while it was being removed".into());
        }
        files.push((name.clone(), file));
    }
    {
        let archive = Archive::open_files(&directory.path, &files)?;
        if archive.metadata()?.id != uuid {
            return Err(
                "This staged copy is not the archive that is already in the library; nothing was removed"
                    .into(),
            );
        }
    }
    #[cfg(test)]
    {
        let hook = {
            let mut slot = DUPLICATE_INTAKE_HOOK.lock().unwrap_or_else(|e| e.into_inner());
            match slot.take() {
                Some((thread, hook)) if thread == std::thread::current().id() => Some(hook),
                other => {
                    *slot = other;
                    None
                }
            }
        };
        if let Some(hook) = hook {
            hook();
        }
    }
    directory.unchanged()?;
    for (name, stamp) in names.iter().zip(&stamps) {
        // Removal is by pathname, so re-check the identity immediately before
        // unlinking: the opened handles above must still be what the name
        // resolves to.
        if directory.stamp(name)? != *stamp {
            return Err("This staged copy changed while it was being removed".into());
        }
        match directory.dir.remove_file(name) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Removing this staged copy failed: {error}")),
        }
    }
    Ok(())
}

/// A verified source admission retains the capabilities whose identities were
/// checked. Callers must not replace these with files reopened by pathname.
pub(crate) struct PinnedArchive {
    pub path: PathBuf,
    pub directory: Dir,
    pub files: Vec<(String, File)>,
    pub stamps: Vec<Metadata>,
}

pub(crate) fn retain_archive_files(book: &Book) -> Result<PinnedArchive, String> {
    archive_basename(Path::new(&book.path))?;
    let retained = retain_files(book, false)?;
    for stamp in &retained.stamps {
        FileStamp::of(stamp)?;
    }
    Ok(retained)
}

pub(crate) fn retain_read_archive_files(book: &Book) -> Result<PinnedArchive, String> {
    retain_files(book, true)
}

fn retain_files(book: &Book, allow_legacy_single: bool) -> Result<PinnedArchive, String> {
    let path = Path::new(&book.path);
    let directory = Directory::open(path.parent().ok_or("Archive has no parent directory")?)?;
    let primary = path.file_name().and_then(|name| name.to_str()).ok_or("Archive filename must be valid Unicode")?;
    let names = read_part_names(&directory, primary, allow_legacy_single)?;
    let mut files = Vec::with_capacity(names.len());
    let mut stamps = Vec::with_capacity(names.len());
    for name in names {
        let file = directory.open_read_file(&name)?;
        FileExt::try_flock_shared(&file).map_err(|e| format!("Archive is busy: {e}"))?;
        let stamp = Metadata::from_file(&file).map_err(err)?;
        if directory.read_stamp(&name)? != FileStamp::read(&stamp)? {
            return Err("Archive changed during source admission".into());
        }
        stamps.push(stamp);
        files.push((name, file));
    }
    directory.unchanged()?;
    Ok(PinnedArchive { path: directory.path, directory: directory.dir, files, stamps })
}

pub(crate) fn retained_source_unchanged(source: &PinnedArchive) -> Result<(), String> {
    let directory = Directory::open(&source.path)?;
    if Identity::of(&source.directory.dir_metadata().map_err(err)?) != directory.identity {
        return Err("Retained archive directory was replaced".into());
    }
    for ((name, file), expected) in source.files.iter().zip(&source.stamps) {
        let expected = FileStamp::read(expected)?;
        if FileStamp::read(&Metadata::from_file(file).map_err(err)?)? != expected
            || directory.read_stamp(name)? != expected {
            return Err("Retained archive source changed".into());
        }
    }
    directory.unchanged()
}

pub(crate) fn archive_content_identity(book: &Book) -> Result<crate::integrity::ContentIdentity, String> {
    crate::archive_source::retain(book)?.identity()
}

pub(crate) fn pin_archive_files(book: &Book) -> Result<PinnedArchive, String> {
    let pinned = retain_archive_files(book)?;
    let directory = Directory::open(&pinned.path)?;
    let files = &pinned.files;
    let stamps = &pinned.stamps;
    let archive = Archive::open_files(&directory.path, files)?;
    if archive.metadata()?.id != book.metadata.id || !archive.check_integrity()? {
        return Err("Kiwix could not verify this archive. Remove it from your library".into());
    }
    directory.unchanged()?;
    for ((name, file), stamp) in files.iter().zip(stamps) {
        if FileStamp::of(&Metadata::from_file(file).map_err(err)?)? != FileStamp::of(stamp)?
            || directory.stamp(name)? != FileStamp::of(stamp)?
        {
            return Err("Archive changed during source verification".into());
        }
    }
    Ok(pinned)
}
pub fn reveal_book(book: &Book) -> Result<(), String> {
    #[cfg(mobile)]
    {
        let _ = book;
        Err("This mobile platform cannot reveal archive folders in an external file manager. Use Export to save a copy.".into())
    }
    #[cfg(desktop)]
    crate::os_open::open(reveal_target(book)?).map_err(err)
}

/// The folder a reveal opens. Reveal is nondestructive: do not checksum a
/// multi-gigabyte book just to show it, and accept every name and link count
/// that reading accepts. It grants no authority to move or delete the file.
#[cfg(desktop)]
fn reveal_target(book: &Book) -> Result<PathBuf, String> {
    let retained = retain_read_archive_files(book)?;
    if Archive::open_files(&retained.path, &retained.files)?.metadata()?.id != book.metadata.id {
        return Err("The file at this archive's location is a different archive. Use Locate to find the right file".into());
    }
    Ok(retained.path)
}

/// The staging folder is what the person finds in the Trash, so it must be
/// visible and recognizable there on every desktop: NSFileManager Trash may
/// omit Put Back, the Windows Recycle Bin lists the folder by name, and Linux
/// file managers hide dot-names in the Trash view. A permanent delete never
/// leaves the folder behind and keeps it hidden while it exists.
fn deletion_staging_prefix(trash: bool) -> &'static str {
    if trash { "kiwix-deleted" } else { ".kiwix-delete" }
}

/// Stage only the verified exact set in a private sibling directory. A failure
/// before irreversible deletion restores files with exclusive renames; failures
/// after staging report the recovery directory, never erase unknown files.
pub fn delete_book(book: &Book, trash: bool) -> Result<(), String> {
    delete_book_impl(book, trash, None)
}

/// Acquisition cleanup must prove ownership of the exact captured files, not
/// merely match an old import receipt's pathname to a current library item.
pub(crate) fn delete_book_with_ownership(
    book: &Book,
    trash: bool,
    ownership: &[(PathBuf, crate::torrent::CleanupEntry)],
) -> Result<(), String> {
    delete_book_impl(book, trash, Some(ownership))
}

fn delete_book_impl(
    book: &Book,
    trash: bool,
    ownership: Option<&[(PathBuf, crate::torrent::CleanupEntry)]>,
) -> Result<(), String> {
    #[cfg(mobile)]
    if trash {
        return Err("This mobile platform has no supported OS Trash. Disable Move deleted archives to Trash and explicitly confirm permanent deletion.".into());
    }
    let _operation = OPERATIONS
        .lock()
        .map_err(|_| "Library operation lock unavailable")?;
    let path = Path::new(&book.path);
    let source = Directory::open(path.parent().ok_or("Archive has no parent directory")?)?;
    let working_directory = source.path.to_str().ok_or("Archive directory must be valid Unicode")?;
    let primary = archive_basename(path)?;
    let parts = verified_parts(&source, &primary, &book.metadata.id)?;
    if let Some(ownership) = ownership {
        for (part, _) in &parts {
            let original = source.path.join(part);
            let proof = ownership.iter().find(|(path, _)| path == &original || path.to_str().zip(original.to_str())
                .is_some_and(|(left, right)| crate::store::acquisition_paths_equal(left, right, working_directory)))
                .ok_or("This archive is not owned by the selected download")?;
            if !proof.1.matches(&original, &source.open_file(part)?)? {
                return Err("Downloaded archive changed before deletion".into());
            }
        }
    }
    let name = format!("{}-{}", deletion_staging_prefix(trash), Uuid::new_v4());
    // Windows directory capabilities never share delete, so the staging
    // capability is released (taken) just before its directory is trashed or
    // removed. Unix keeps it for the whole operation, exactly as before.
    let staging = private_child(&source, &name, true)?;
    let staging_identity = staging.identity.clone();
    #[cfg_attr(not(windows), allow(unused_mut, reason = "only Windows releases the staging capability early"))]
    let mut staging_slot = Some(staging);
    let result = (|| {
        let staging = staging_slot.as_ref().ok_or("Deletion staging capability unavailable")?;
        for (name, stamp) in &parts {
            source.unchanged()?;
            if source.stamp(name)? != *stamp {
                return Err("Archive part changed before deletion".into());
            }
            move_exclusive(&source, name, staging, name)?;
            if staging.stamp(name)? != *stamp {
                return Err("Staged archive changed identity".into());
            }
        }
        source.sync()?;
        staging.sync()?;
        verified_parts(staging, &primary, &book.metadata.id)?;
        if let Some(ownership) = ownership {
            for (part, _) in &parts {
                let original = source.path.join(part);
                let proof = ownership.iter().find(|(path, _)| path == &original || path.to_str().zip(original.to_str())
                    .is_some_and(|(left, right)| crate::store::acquisition_paths_equal(left, right, working_directory)))
                    .ok_or("This archive is not owned by the selected download")?;
                if !proof.1.matches_retained(&original, &staging.open_file(part)?)? {
                    return Err("Captured archive no longer belongs to the selected download".into());
                }
            }
        }
        let actual: HashSet<_> = staging
            .dir
            .entries()
            .map_err(err)?
            .map(|e| e.map(|e| e.file_name()).map_err(err))
            .collect::<Result<_, _>>()?;
        let expected: HashSet<_> = parts
            .iter()
            .map(|(name, _)| std::ffi::OsString::from(name))
            .collect();
        if actual != expected {
            return Err("Deletion staging directory contains unexpected files".into());
        }
        staging.unchanged()?;
        source.unchanged()?;
        if trash {
            // Tests must never fill the developer's real Trash / Recycle Bin;
            // a test that reaches here should turn `trash_deleted_files` off.
            // The staged parts are rolled back like any trash failure.
            #[cfg(test)]
            if !TEST_ALLOWS_REAL_TRASH.load(std::sync::atomic::Ordering::SeqCst) {
                return Err("Moving archive to Trash: refused under test".into());
            }
            #[cfg(target_os = "macos")]
            {
                use trash::macos::{DeleteMethod, TrashContextExtMacos};
                let mut context = trash::TrashContext::new();
                // Native Trash avoids Finder automation prompts and script waits.
                // NSFileManager moves the staged directory into the archive
                // volume's own Trash, so this is a rename, never a 3 GB copy
                // (measured: 73 ms for a 2.83 GB archive). Recovery remains
                // possible by dragging the folder out of Trash.
                context.set_delete_method(DeleteMethod::NsFileManager);
                context.delete(&staging.path).map_err(|e| format!("Moving archive to Trash: {e}"))?;
            }
            #[cfg(all(desktop, not(target_os = "macos")))]
            {
                let path = staging.path.clone();
                // The shell cannot move a directory we hold open; trash is
                // path-based on every platform, after the same identity checks.
                #[cfg(windows)]
                release_directory(&source, &mut staging_slot, &name)?;
                trash::delete(&path).map_err(|e| format!("Moving archive to Trash: {e}"))?;
            }
        } else {
            for (name, stamp) in &parts {
                if staging.stamp(name)? != *stamp {
                    return Err("Staged archive part changed; deletion stopped".into());
                }
                staging.dir.remove_file(name).map_err(err)?;
            }
            staging.sync()?;
            #[cfg(windows)]
            remove_owned_directory(&source, &mut staging_slot, &name)?;
            #[cfg(not(windows))]
            source.dir.remove_dir(&name).map_err(err)?;
        }
        source.sync()
    })();
    if let Err(error) = result {
        let staging_path = source.path.join(&name);
        // Only Windows can reach the error path after releasing the staging
        // capability; restore solely from the same verified directory.
        let staging = match staging_slot {
            Some(staging) => Ok(staging),
            None => reopen_child(&source, &name, &staging_identity),
        };
        let mut restoration = Vec::new();
        match &staging {
            Ok(staging) => {
                for (part, stamp) in &parts {
                    if staging.stamp(part).as_ref() == Ok(stamp) {
                        if let Err(error) = move_exclusive(staging, part, &source, part) {
                            restoration.push(error);
                        }
                    }
                }
            }
            Err(error) if source.absent(&name) != Ok(true) => restoration.push(error.clone()),
            Err(_) => {}
        }
        let _ = source.sync();
        // Windows cannot remove a directory while its capability is open.
        #[cfg(windows)]
        drop(staging);
        let _ = source.dir.remove_dir(&name);
        // Point at the staging folder only while it can still hold something.
        if restoration.is_empty() {
            if parts.iter().all(|(part, stamp)| source.stamp(part).as_ref() == Ok(stamp)) {
                return Err(format!("{error}. The archive was left where it is"));
            }
            if source.absent(&name) == Ok(true) {
                return Err(format!(
                    "{error}. The archive files had already been {}",
                    if trash { "moved to the Trash" } else { "deleted" }
                ));
            }
        }
        return Err(format!(
            "{error}. Any retained archive parts are recoverable at {}. {}",
            crate::storage::display_path(&staging_path),
            restoration.join("; ")
        ));
    }
    if !trash {
        // The book is gone, so its private native-import staging directory is no
        // longer referenced. This is best-effort and only removes one empty,
        // canonical-UUID child of `archives/mobile-imports-v1`; non-empty or
        // foreign directories are never touched.
        let path = source.path.clone();
        // Windows directory handles deny deletion; release ours first.
        drop(source);
        let _ = remove_empty_import_directory(&path);
    }
    Ok(())
}

/// Native imports copy documents into `archives/mobile-imports-v1/<uuid>/`.
/// Permanent deletion of the last archive in one of those directories used to
/// leave the empty directory behind. Remove it only when every ownership
/// condition holds; a racing write makes `remove_dir` fail and is ignored.
fn remove_empty_import_directory(source: &Path) -> Result<(), String> {
    let Some(parent) = source.parent() else {
        return Ok(());
    };
    let collection = parent.file_name().and_then(|name| name.to_str());
    let archives = parent
        .parent()
        .and_then(|root| root.file_name())
        .and_then(|name| name.to_str());
    if collection != Some("mobile-imports-v1") || archives != Some("archives") {
        return Ok(());
    }
    let Some(name) = source.file_name().and_then(|name| name.to_str()) else {
        return Ok(());
    };
    // Only the canonical lowercase UUID form this app creates is ours.
    if Uuid::parse_str(name).map(|id| id.to_string()).ok().as_deref() != Some(name) {
        return Ok(());
    }
    // Close the child capability before removing its directory entry.
    {
        let directory = Directory::open(source)?;
        if directory.dir.entries().map_err(err)?.next().is_some() {
            return Ok(());
        }
    }
    let parent = Directory::open(parent)?;
    parent.dir.remove_dir(name).map_err(err)?;
    parent.sync()
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Part {
    name: String,
    source: FileStamp,
    sha256: String,
    target: Option<FileStamp>,
}
/// A part whose SHA-256 was computed from exactly these bytes in this process.
/// Later stages of the same move prove it by file identity instead of another
/// whole-file pass; journal replay (a new process) never has one.
///
/// The stamp uses device/inode/size/mtime because those survive rename. ctime
/// is deliberately excluded (rename updates it), so an in-place rewrite that
/// restores mtime is not distinguished here; the staging directory is
/// owner-only and this proof only covers bytes hashed in this process.
#[derive(Clone)]
struct VerifiedPart {
    name: String,
    stamp: FileStamp,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    id: String,
    owner: String,
    book_id: String,
    title: String,
    source_directory: PathBuf,
    destination_directory: PathBuf,
    source_directory_identity: Identity,
    destination_directory_identity: Identity,
    primary: String,
    same_filesystem: bool,
    staging_identity: Option<Identity>,
    parts: Vec<Part>,
    #[serde(default)]
    cleanup_staging_identity: Option<Identity>,
}
struct Journals {
    directory: Directory,
    owner: String,
}
impl Journals {
    fn open(store: &Store, create: bool) -> Result<Option<Self>, String> {
        // Store has already selected/locked this trusted profile. Resolve OS
        // aliases such as macOS /var once; owned children remain no-follow.
        let profile = Directory::open(&store.data_dir().canonicalize().map_err(err)?)?;
        if !create && profile.absent("library-migrations-v1")? {
            return Ok(None);
        }
        let directory = private_child(&profile, "library-migrations-v1", create)?;
        let owner = if directory.absent("owner")? {
            if !create {
                return Err(
                    "Migration directory has no ownership marker; no recovery performed".into(),
                );
            }
            let owner = Uuid::new_v4().to_string();
            let mut file = create_file(&directory, "owner")?;
            file.write_all(owner.as_bytes()).map_err(err)?;
            file.sync_all().map_err(err)?;
            directory.sync()?;
            owner
        } else {
            let mut owner = String::new();
            directory
                .open_file("owner")?
                .take(65)
                .read_to_string(&mut owner)
                .map_err(err)?;
            canonical_uuid(&owner)?;
            owner
        };
        Ok(Some(Self { directory, owner }))
    }
    fn write(&self, journal: &Journal) -> Result<(), String> {
        canonical_uuid(&journal.id)?;
        let bytes = serde_json::to_vec(journal).map_err(err)?;
        if bytes.len() as u64 > MAX_JOURNAL {
            return Err("Migration journal exceeds size bound".into());
        }
        let temporary = format!("{}.{}.tmp", journal.id, Uuid::new_v4());
        let mut file = create_file(&self.directory, &temporary)?;
        file.write_all(&bytes).map_err(err)?;
        file.sync_all().map_err(err)?;
        drop(file);
        // Replacing our own regular journal is permitted, never an arbitrary path.
        let name = format!("{}.json", journal.id);
        if !self.directory.absent(&name)? {
            self.directory.stamp(&name)?;
        }
        self.directory
            .dir
            .rename(&temporary, &self.directory.dir, &name)
            .map_err(err)?;
        self.directory.sync()
    }
    fn remove(&self, id: &str) -> Result<(), String> {
        canonical_uuid(id)?;
        self.directory
            .dir
            .remove_file(format!("{id}.json"))
            .map_err(err)?;
        self.directory.sync()
    }
}
fn canonical_uuid(value: &str) -> Result<(), String> {
    if Uuid::parse_str(value).map_err(err)?.to_string() != value {
        return Err("This action could not be verified. Try again".into());
    }
    Ok(())
}
fn validate_journal(journal: &Journal, journals: &Journals, store: &Store) -> Result<(), String> {
    canonical_uuid(&journal.id)?;
    if journal.version != 1
        || journal.owner != journals.owner
        || journal.parts.is_empty()
        || journal.parts.len() > MAX_PARTS
    {
        return Err("Unowned or invalid migration journal; sources were not touched".into());
    }
    filename(&journal.primary)?;
    let split = archive_suffix(&journal.primary, ".zimaa");
    if !split && (!archive_suffix(&journal.primary, ".zim") || journal.parts.len() != 1) {
        return Err("Invalid migration archive set".into());
    }
    for (index, part) in journal.parts.iter().enumerate() {
        let matches = if split {
            split_part_matches(&journal.primary, index, &part.name)
                && (index != 0 || part.name == journal.primary)
        } else {
            part.name == journal.primary
        };
        filename(&part.name)?;
        if !matches
            || part.source.size == 0
            || part.sha256.len() != 64
            || !part.sha256.bytes().all(|c| c.is_ascii_hexdigit())
        {
            return Err("Invalid exact migration part manifest".into());
        }
    }
    let state = store.read()?;
    let book = state
        .books
        .iter()
        .find(|book| book.metadata.id == journal.book_id && book.managed)
        .ok_or("Journal is not bound to a managed library archive")?;
    let source = journal.source_directory.join(&journal.primary);
    let destination = journal.destination_directory.join(&journal.primary);
    if Path::new(&book.path) != source && Path::new(&book.path) != destination {
        return Err("Library path no longer belongs to this migration; nothing was deleted".into());
    }
    if journal.source_directory == journal.destination_directory {
        return Err("Migration source and destination are identical".into());
    }
    Ok(())
}
fn commit_path(store: &Store, journal: &Journal, destination: &Path) -> Result<(), String> {
    let source = journal.source_directory.join(&journal.primary);
    let target = journal.destination_directory.join(&journal.primary);
    store.update(|state| {
        let index = state
            .books
            .iter()
            .position(|book| book.metadata.id == journal.book_id && book.managed)
            .ok_or("Managed archive removed during migration")?;
        let book = &state.books[index];
        if Path::new(&book.path) != source && Path::new(&book.path) != target {
            return Err("Archive path changed during migration".into());
        }
        let before = book.clone();
        let mut after = before.clone();
        after.path = destination
            .to_str()
            .ok_or("Destination path must be Unicode")?
            .into();
        after.available = true;
        state.reconcile_edition_relocation(&before, &after)?;
        state.books[index] = after;
        Ok(())
    })
}
fn check_part(directory: &Directory, part: &Part, expected: &FileStamp) -> Result<(), String> {
    if digest(directory, &part.name, Some(expected))?.1 != part.sha256 {
        return Err(format!("Archive part checksum changed: {}", part.name));
    }
    Ok(())
}
/// `check_part` without a second whole-file read when this process hashed these
/// exact bytes at `expected`. The stamp is re-read after the hash, so a rewrite
/// that changed device, inode, size or mtime still fails through `check_part`.
fn check_part_proven(
    directory: &Directory,
    part: &Part,
    expected: &FileStamp,
    proven: &[VerifiedPart],
) -> Result<(), String> {
    let is_proven = proven
        .iter()
        .any(|verified| verified.name == part.name && verified.stamp == *expected);
    if is_proven && directory.stamp(&part.name)? == *expected {
        return Ok(());
    }
    check_part(directory, part, expected)
}
/// Free bytes on the volume holding `path`; `None` when the platform does not
/// report capacity, which is never treated as zero.
fn free_space(path: &Path) -> Option<u64> {
    #[cfg(test)]
    if let Some((thread, free)) = *FREE_SPACE_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) {
        if thread == std::thread::current().id() {
            return Some(free);
        }
    }
    crate::storage::volume_space(path).map(|(free, _)| free)
}
/// Refuse a copy that cannot fit before any byte is hashed or written.
fn ensure_copy_space(free: Option<u64>, needed: u64, destination: &Path) -> Result<(), String> {
    match free {
        Some(free) if free < needed => Err(format!(
            "Not enough free space in {}: {needed} bytes are needed but {free} bytes are free",
            crate::storage::display_path(destination)
        )),
        _ => Ok(()),
    }
}
/// Remove what an unfinished cross-volume copy left at its destination, and
/// only that. `Ok(true)` means nothing of this move remains there: no part was
/// published and the private staging directory is gone. `Ok(false)` means a
/// destination part exists or the destination is no longer the journaled
/// directory; nothing was touched. The source and the journal are never
/// touched here.
///
/// Ownership: the staging directory must be this user's private (0700) child
/// carrying the identity the journal recorded when it was created. A finished
/// staged copy is removed only while it is still the recorded file (device,
/// inode and size). An unfinished copy has no recorded identity of its own; as
/// on a retry, it is owned only by sitting in that journal-bound directory.
/// The directory itself is removed only when empty, so an unexpected entry
/// keeps it, and the caller keeps the journal.
fn clear_unpublished_copy(journal: &Journal) -> Result<bool, String> {
    if journal.same_filesystem {
        return Ok(false);
    }
    let destination = Directory::open(&journal.destination_directory)?;
    if destination.identity != journal.destination_directory_identity {
        return Ok(false);
    }
    for part in &journal.parts {
        if !destination.absent(&part.name)? {
            return Ok(false);
        }
    }
    let staging_name = format!(".kiwix-move-{}", journal.id);
    if destination.absent(&staging_name)? {
        return Ok(true);
    }
    let staging = private_child(&destination, &staging_name, false)?;
    match &journal.staging_identity {
        Some(identity) if identity == &staging.identity => {
            for part in &journal.parts {
                if staging.absent(&part.name)? {
                    continue;
                }
                let stamp = staging.stamp(&part.name)?;
                if part.target.as_ref().is_some_and(|target| {
                    target.identity != stamp.identity || target.size != stamp.size
                }) {
                    return Err(format!(
                        "A staged copy changed and was preserved at {}",
                        crate::storage::display_path(&staging.path)
                    ));
                }
                staging.dir.remove_file(&part.name).map_err(err)?;
            }
            staging.sync()?;
        }
        Some(_) => {
            return Err(format!(
                "The staging folder was replaced and was preserved at {}",
                crate::storage::display_path(&staging.path)
            ))
        }
        // The identity was never recorded, so nothing inside is adopted. The
        // empty-only removal below still clears a directory created just
        // before the journal write that would have recorded it failed.
        None => {}
    }
    // Empty-only removal: an entry the move did not write keeps the folder.
    #[cfg(windows)]
    let removed = remove_owned_directory(&destination, &mut Some(staging), &staging_name);
    #[cfg(not(windows))]
    let removed = {
        drop(staging);
        destination.dir.remove_dir(&staging_name).map_err(err)
    };
    removed.map_err(|error| {
        format!(
            "The staging folder could not be removed and was preserved at {}: {error}",
            crate::storage::display_path(&destination.path.join(&staging_name))
        )
    })?;
    destination.sync()?;
    Ok(true)
}
/// Give up a cross-volume move that failed before it published anything, so it
/// is not re-hashed and re-copied at every launch. The journal is removed only
/// when the library still points at the intact source side, no destination
/// part exists and the owned staging entries are gone; in every other case it
/// stays for startup recovery. Returns whether the move was abandoned.
fn abandon_unpublished_copy(store: &Store, journals: &Journals, journal: &Journal) -> Result<bool, String> {
    if journal.same_filesystem {
        return Ok(false);
    }
    if validate_journal(journal, journals, store).is_err() {
        return Ok(false);
    }
    let source = journal.source_directory.join(&journal.primary);
    let committed = !store.read()?.books.iter().any(|book| {
        book.metadata.id == journal.book_id && book.managed && Path::new(&book.path) == source
    });
    if committed || !clear_unpublished_copy(journal)? {
        return Ok(false);
    }
    journals.remove(&journal.id)?;
    Ok(true)
}
/// The message for a failed move, after abandoning it where that is safe.
fn failed_move(store: &Store, journals: &Journals, journal: &Journal, error: String) -> String {
    match abandon_unpublished_copy(store, journals, journal) {
        Ok(true) => format!(
            "{error}. This archive was not moved and is still in {}",
            crate::storage::display_path(&journal.source_directory)
        ),
        Ok(false) => error,
        Err(cleanup) if cleanup == error => error,
        Err(cleanup) => format!("{error}. The unfinished move is kept and will be tried again at the next start: {cleanup}"),
    }
}
/// A journal this profile wrote for a book that is no longer in the library
/// can never be validated or finished.
fn journal_orphaned(journal: &Journal, journals: &Journals, store: &Store) -> Result<bool, String> {
    canonical_uuid(&journal.id)?;
    if journal.version != 1 || journal.owner != journals.owner {
        return Ok(false);
    }
    Ok(!store.read()?.books.iter().any(|book| book.metadata.id == journal.book_id))
}
fn finish_move(
    store: &Store,
    journals: &Journals,
    journal: &mut Journal,
    proven: &mut Vec<VerifiedPart>,
) -> Result<(), String> {
    finish_move_with_rename(store, journals, journal, proven, move_exclusive)
}
fn finish_move_with_rename(
    store: &Store,
    journals: &Journals,
    journal: &mut Journal,
    proven: &mut Vec<VerifiedPart>,
    mut rename: impl FnMut(&Directory, &str, &Directory, &str) -> Result<(), String>,
) -> Result<(), String> {
    validate_journal(journal, journals, store)?;
    let source = Directory::open(&journal.source_directory)?;
    let destination = Directory::open(&journal.destination_directory)?;
    if source.identity != journal.source_directory_identity
        || destination.identity != journal.destination_directory_identity
    {
        return Err(
            "Migration directory identity changed; all remaining sources were preserved".into(),
        );
    }
    if journal.same_filesystem != (source.identity.device == destination.identity.device) {
        return Err("Migration filesystem identity changed".into());
    }
    let target_path = destination.path.join(&journal.primary);
    if journal.same_filesystem {
        // The profile is durably pointed at the recoverable journal destination
        // before any source name disappears. Store refuses uncertain durability.
        commit_path(store, journal, &target_path)?;
        let result = (|| {
            for part in &journal.parts {
                let has_source = !source.absent(&part.name)?;
                let has_target = !destination.absent(&part.name)?;
                if has_target {
                    check_part(&destination, part, &part.source)?;
                    // An exclusive native rename must not leave two names. In
                    // particular do not unlink a foreign collision on recovery.
                    if has_source {
                        return Err(
                            "Both migration names exist; source retained for manual recovery"
                                .into(),
                        );
                    }
                } else {
                    check_part_proven(&source, part, &part.source, proven)?;
                    source.unchanged()?;
                    destination.unchanged()?;
                    rename(&source, &part.name, &destination, &part.name)?;
                    // The rename preserved device, inode, size and mtime, so the
                    // destination is still the exact file this process hashed.
                    check_part_proven(&destination, part, &part.source, proven)?;
                    destination.sync()?;
                    source.sync()?;
                }
            }
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            // Keep the pre-rename crash ordering, but do not strand an intact
            // archive at a nonexistent target after a failed first rename.
            // Partial moves still belong to the journal destination.
            let intact_source = journal
                .parts
                .iter()
                .all(|part| check_part(&source, part, &part.source).is_ok());
            if intact_source {
                commit_path(store, journal, &source.path.join(&journal.primary)).map_err(
                    |restore| format!("{error}; restoring intact source path failed: {restore}"),
                )?;
            }
            return Err(error);
        }
    } else {
        // Cross-volume source is retained until every destination part is
        // published, the complete archive verifies, and the profile is durable.
        let staging_name = format!(".kiwix-move-{}", journal.id);
        // A crash after removing the empty staging directory but before deleting
        // the journal must remain recoverable and idempotent.
        if destination.absent(&staging_name)? && journal.staging_identity.is_some() {
            for part in &journal.parts {
                check_part_proven(
                    &destination,
                    part,
                    part.target
                        .as_ref()
                        .ok_or("Missing published copy identity")?,
                    proven,
                )?;
            }
            cleanup_migration_sources(store, journals, journal, &source, &destination, proven)?;
            return journals.remove(&journal.id);
        }
        let staging = private_child(
            &destination,
            &staging_name,
            journal.staging_identity.is_none(),
        )?;
        if let Some(identity) = &journal.staging_identity {
            if identity != &staging.identity {
                return Err("Migration staging directory was replaced".into());
            }
        } else {
            journal.staging_identity = Some(staging.identity.clone());
            journals.write(journal)?;
        }
        // Every part without a recorded copy still has to be written. Check
        // before any source is hashed again, so a copy that cannot fit fails
        // at once instead of filling the volume first. An uncommitted partial
        // copy is removed before its part is copied again, so it counts as free.
        let mut remaining = 0u64;
        let mut reclaimed = 0u64;
        for part in journal.parts.iter().filter(|part| part.target.is_none()) {
            remaining = remaining
                .checked_add(part.source.size)
                .ok_or("Archive part lengths overflow")?;
            if !staging.absent(&part.name)? {
                reclaimed = reclaimed.saturating_add(staging.stamp(&part.name)?.size);
            }
        }
        ensure_copy_space(
            free_space(&destination.path).map(|free| free.saturating_add(reclaimed)),
            remaining,
            &destination.path,
        )?;
        for index in 0..journal.parts.len() {
            let part = &journal.parts[index];
            if !destination.absent(&part.name)? {
                let expected = part
                    .target
                    .as_ref()
                    .ok_or("Unowned migration destination collision; source retained")?;
                check_part(&destination, part, expected)?;
                continue;
            }
            check_part_proven(&source, part, &part.source, proven)?;
            if part.target.is_none() {
                // A crash may leave an uncommitted partial copy. It is in our
                // private operation directory, but still must be a regular file.
                if !staging.absent(&part.name)? {
                    staging.stamp(&part.name)?;
                    staging.dir.remove_file(&part.name).map_err(err)?;
                }
                crate::storage::check_file_size(&destination.path, part.source.size)?;
                let input = source.open_file(&part.name)?;
                let mut output = create_file(&staging, &part.name)?;
                let (count, hash) = copy_hashed(
                    input.take(
                        part.source
                            .size
                            .checked_add(1)
                            .ok_or("Archive part too large")?,
                    ),
                    &mut output,
                )?;
                output.sync_all().map_err(err)?;
                let written = FileStamp::of(&output.metadata().map_err(err)?)?;
                drop(output);
                if count != part.source.size {
                    return Err("Archive size changed while copying; source retained".into());
                }
                // The staged bytes were hashed as they were written, so proving
                // the copy needs no second pass over the new file.
                if hash != part.sha256 {
                    return Err("Copied archive checksum differs; source retained".into());
                }
                check_part_proven(&source, part, &part.source, proven)?;
                // The hash describes the handle we just closed. The path must
                // still name that inode and size before the proof is trusted.
                // Windows may finalize mtime at close, so do not compare the
                // full stamp across the close; the proof stamp is `written`.
                let stamp = staging.stamp(&part.name)?;
                if stamp.identity != written.identity || stamp.size != written.size {
                    return Err("Staged copy changed; source retained".into());
                }
                proven.push(VerifiedPart {
                    name: part.name.clone(),
                    stamp: written.clone(),
                });
                journal.parts[index].target = Some(written);
                staging.sync()?;
                journals.write(journal)?;
            }
            let part = &journal.parts[index];
            check_part_proven(
                &staging,
                part,
                part.target.as_ref().ok_or("Missing copy identity")?,
                proven,
            )?;
            rename(&staging, &part.name, &destination, &part.name)?;
            destination.sync()?;
            staging.sync()?;
        }
        cleanup_migration_sources(store, journals, journal, &source, &destination, proven)?;
        // Empty-only removal never sweeps unexpected files.
        #[cfg(windows)]
        remove_owned_directory(&destination, &mut Some(staging), &staging_name)?;
        #[cfg(not(windows))]
        destination.dir.remove_dir(&staging_name).map_err(err)?;
        destination.sync()?;
    }
    verify_manifest(&destination, journal, proven)?;
    journals.remove(&journal.id)
}
fn verify_manifest(
    directory: &Directory,
    journal: &Journal,
    proven: &[VerifiedPart],
) -> Result<(), String> {
    // Every destination part is, by stat identity, the exact file this process
    // hashed after libzim's full content checksum passed. The exact part set and
    // archive identity are still verified; only the redundant second content
    // pass is skipped. Replay has no proof and keeps the full checksum.
    let cohort_proven = journal.parts.iter().all(|part| {
        proven.iter().any(|verified| {
            verified.name == part.name
                && directory.stamp(&part.name).is_ok_and(|stamp| stamp == verified.stamp)
        })
    });
    let files = if cohort_proven {
        verified_parts(directory, &journal.primary, &journal.book_id)?
    } else {
        verified(directory, &journal.primary, &journal.book_id)?
    };
    if files.len() != journal.parts.len()
        || files
            .iter()
            .zip(&journal.parts)
            .any(|((name, _), part)| name != &part.name)
    {
        return Err(
            "Journal does not describe the exact verified archive set; no source cleanup performed"
                .into(),
        );
    }
    // Verification and the journal are publication authority, including replay.
    // Cross-volume copies start excluded and must not inherit stale settings.
    #[cfg(target_os = "ios")]
    for (name, expected) in &files {
        let file = directory.open_file(name)?;
        if FileStamp::of(&Metadata::from_file(&file).map_err(err)?)? != *expected {
            return Err("Verified migration archive changed before backup policy".into());
        }
        crate::mobile_backup::archive_policy(&file, &directory.path.join(name))?;
    }
    Ok(())
}
fn cleanup_migration_sources(
    store: &Store,
    journals: &Journals,
    journal: &mut Journal,
    source: &Directory,
    destination: &Directory,
    proven: &[VerifiedPart],
) -> Result<(), String> {
    cleanup_migration_sources_with_rename(
        store,
        journals,
        journal,
        source,
        destination,
        proven,
        move_exclusive,
    )
}
fn cleanup_migration_sources_with_rename(
    store: &Store,
    journals: &Journals,
    journal: &mut Journal,
    source: &Directory,
    destination: &Directory,
    proven: &[VerifiedPart],
    mut rename: impl FnMut(&Directory, &str, &Directory, &str) -> Result<(), String>,
) -> Result<(), String> {
    verify_manifest(destination, journal, proven)?;
    commit_path(store, journal, &destination.path.join(&journal.primary))?;
    for part in &journal.parts {
        check_part_proven(
            destination,
            part,
            part.target.as_ref().ok_or("Missing destination identity")?,
            proven,
        )?;
    }
    source.unchanged()?;
    destination.unchanged()?;
    let staging_name = format!(".kiwix-cleanup-{}", journal.id);
    if source.absent(&staging_name)? && journal.cleanup_staging_identity.is_some() {
        if journal
            .parts
            .iter()
            .all(|part| source.absent(&part.name) == Ok(true))
        {
            return Ok(());
        }
        return Err(
            "Source cleanup staging directory disappeared; remaining files were preserved".into(),
        );
    }
    let staging = private_child(
        source,
        &staging_name,
        journal.cleanup_staging_identity.is_none(),
    )?;
    if let Some(identity) = &journal.cleanup_staging_identity {
        if identity != &staging.identity {
            return Err("Source cleanup staging directory was replaced".into());
        }
    } else {
        // A crash before recording the identity may leave an empty directory,
        // but no unjournaled entry is ever adopted for deletion.
        if staging.dir.entries().map_err(err)?.next().is_some() {
            return Err(format!(
                "Unowned source cleanup entries preserved at {}",
                staging.path.display()
            ));
        }
        journal.cleanup_staging_identity = Some(staging.identity.clone());
        journals.write(journal)?;
    }
    let staging_identity = staging.identity.clone();
    // Windows releases (takes) the capability just before removing its
    // directory; Unix keeps it for the whole operation, exactly as before.
    #[cfg_attr(not(windows), allow(unused_mut, reason = "only Windows releases the staging capability early"))]
    let mut staging_slot = Some(staging);
    let result = (|| {
        let staging = staging_slot
            .as_ref()
            .ok_or("Source cleanup staging capability unavailable")?;
        // Recover already staged entries, and validate the complete remaining
        // set before moving anything else out of the public source directory.
        for part in &journal.parts {
            if !staging.absent(&part.name)? {
                check_part_proven(staging, part, &part.source, proven)?;
                if !source.absent(&part.name)? {
                    return Err(
                        "Source name reappeared during cleanup; both files were preserved".into(),
                    );
                }
            } else if !source.absent(&part.name)? {
                check_part_proven(source, part, &part.source, proven)?;
            }
        }
        for part in &journal.parts {
            if staging.absent(&part.name)? && !source.absent(&part.name)? {
                source.unchanged()?;
                staging.unchanged()?;
                // Capture the directory entry atomically, then inspect the
                // captured identity. Never unlink a checked public pathname.
                rename(source, &part.name, staging, &part.name)?;
                check_part_proven(staging, part, &part.source, proven)?;
            }
        }
        staging.sync()?;
        source.sync()?;
        for part in &journal.parts {
            if !staging.absent(&part.name)? {
                check_part_proven(staging, part, &part.source, proven)?;
            }
        }
        staging.unchanged()?;
        for part in &journal.parts {
            if !staging.absent(&part.name)? {
                staging.dir.remove_file(&part.name).map_err(err)?;
                staging.sync()?;
            }
        }
        #[cfg(windows)]
        remove_owned_directory(source, &mut staging_slot, &staging_name)?;
        #[cfg(not(windows))]
        source.dir.remove_dir(&staging_name).map_err(err)?;
        source.sync()
    })();
    if let Err(error) = result {
        let staging_path = source.path.join(&staging_name);
        // Only Windows can reach the error path after releasing the staging
        // capability; restore solely from the same verified directory.
        let staging = match staging_slot {
            Some(staging) => Ok(staging),
            None => reopen_child(source, &staging_name, &staging_identity),
        };
        let mut restoration = Vec::new();
        match &staging {
            Ok(staging) => {
                for part in &journal.parts {
                    // Include mismatching captured entries: they are not ours to
                    // delete. Exclusive restoration cannot overwrite a new source.
                    if staging.absent(&part.name) == Ok(false) {
                        if let Err(error) = move_exclusive(staging, &part.name, source, &part.name) {
                            restoration.push(error);
                        }
                    }
                }
                let _ = staging.sync();
            }
            Err(error) if source.absent(&staging_name) != Ok(true) => restoration.push(error.clone()),
            Err(_) => {}
        }
        let _ = source.sync();
        return Err(format!(
            "{error}. Retained cleanup entries are recoverable at {}. {}",
            crate::storage::display_path(&staging_path),
            restoration.join("; ")
        ));
    }
    Ok(())
}
fn destination_subdirectory(root: &Directory, relative: &Path) -> Result<Directory, String> {
    let mut path = root.path.clone();
    let mut directory = root.dir.try_clone().map_err(err)?;
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err("Unsafe managed archive relative directory".into());
        };
        match directory.create_dir(name) {
            Ok(()) => {
                #[cfg(unix)]
                crate::storage::sync_cap_directory(&directory).map_err(err)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(err(e)),
        }
        directory = directory.open_dir_nofollow(name).map_err(err)?;
        path.push(name);
    }
    root.unchanged()?;
    Directory::open(&path)
}
fn ensure_directory(path: &Path) -> Result<Directory, String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::CurDir | Component::ParentDir))
    {
        return Err("Download directory must be an absolute normalized path".into());
    }
    for ancestor in path.ancestors() {
        match std::fs::symlink_metadata(ancestor) {
            Ok(_) => {
                let root = Directory::open(ancestor)?;
                return destination_subdirectory(&root, path.strip_prefix(ancestor).map_err(err)?);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(err(e)),
        }
    }
    Err("Download directory has no accessible filesystem root".into())
}

/// Only managed, available archives below the *previous* configured download
/// directory are offered. Imported archives and unrelated/NAS paths are skipped.
/// Caller saves the new download setting first and obtains explicit user consent.
pub fn move_archives(
    store: Arc<Store>,
    from_directory: PathBuf,
    destination: PathBuf,
) -> Result<Value, String> {
    let _operation = OPERATIONS
        .lock()
        .map_err(|_| "Library operation lock unavailable")?;
    if store.settings()?.read_only {
        return Err("Read-only mode cannot move archives".into());
    }
    let from = Directory::open(&from_directory)?;
    if Path::new(&store.settings()?.download_directory) != destination {
        return Err("Save the new download directory before moving previous downloads".into());
    }
    let to = ensure_directory(&destination)?;
    if from.identity == to.identity {
        return Ok(json!({"moved": [], "errors": [], "skipped": 0}));
    }
    if destination.starts_with(&from_directory) || from_directory.starts_with(&destination) {
        return Err("Download migration directories cannot contain one another".into());
    }
    let journals = Journals::open(&store, true)?.ok_or("Cannot create migration journals")?;
    let state = store.read()?;
    // Windows records library paths in verbatim form (`\\?\E:\…`) while the
    // settings directory is usually plain (`E:\…`); `Path::starts_with`
    // treats those prefixes as different, so match either spelling.
    #[cfg(windows)]
    let from_alias = std::fs::canonicalize(&from.path).ok();
    let source_root = |path: &Path| -> Option<PathBuf> {
        if path.starts_with(&from.path) {
            return Some(from.path.clone());
        }
        #[cfg(windows)]
        if let Some(alias) = from_alias.as_ref().filter(|alias| path.starts_with(alias)) {
            return Some(alias.clone());
        }
        None
    };
    // Archives on another volume are copied. Refuse the whole move up front
    // when they cannot all fit, before hours of hashing and copying. Sizes
    // come from the identity-only part listing; an archive that cannot be
    // listed is not counted and reports its own error below.
    let mut copied = 0u64;
    for book in &state.books {
        let path = Path::new(&book.path);
        if !(book.managed && book.available) || source_root(path).is_none() {
            continue;
        }
        let Some(Ok(source)) = path.parent().map(Directory::open) else { continue };
        if source.identity.device == to.identity.device {
            continue;
        }
        let Ok(primary) = archive_basename(path) else { continue };
        let Ok(parts) = verified_parts(&source, &primary, &book.metadata.id) else { continue };
        for (_, stamp) in parts {
            copied = copied.checked_add(stamp.size).ok_or("Archive part lengths overflow")?;
        }
    }
    ensure_copy_space(free_space(&to.path), copied, &to.path)
        .map_err(|error| format!("{error}. Nothing was moved"))?;
    let mut moved = Vec::new();
    let mut errors = Vec::new();
    let mut skipped = 0;
    for book in state.books {
        let path = Path::new(&book.path);
        let root = if book.managed && book.available { source_root(path) } else { None };
        let Some(root) = root else {
            skipped += 1;
            continue;
        };
        let result = (|| {
            let relative = path.strip_prefix(&root).map_err(err)?;
            let parent = relative.parent().ok_or("Archive has no relative parent")?;
            let target = destination_subdirectory(&to, parent)?;
            let source = Directory::open(path.parent().ok_or("Archive has no parent")?)?;
            let primary = archive_basename(path)?;
            let files = verified(&source, &primary, &book.metadata.id)?;
            let mut parts = Vec::with_capacity(files.len());
            // Digests computed here prove these exact bytes to the rename,
            // staging and manifest stages below; no stage re-reads them.
            let mut proven = Vec::with_capacity(files.len());
            for (name, stamp) in files {
                if !target.absent(&name)? {
                    return Err(format!(
                        "Destination already exists: {}",
                        target.path.join(name).display()
                    ));
                }
                let (stamp, sha256) = digest(&source, &name, Some(&stamp))?;
                proven.push(VerifiedPart {
                    name: name.clone(),
                    stamp: stamp.clone(),
                });
                parts.push(Part {
                    name,
                    source: stamp,
                    sha256,
                    target: None,
                });
            }
            let mut journal = Journal {
                version: 1,
                id: Uuid::new_v4().to_string(),
                owner: journals.owner.clone(),
                book_id: book.metadata.id.clone(),
                title: book.metadata.title.clone(),
                source_directory: source.path.clone(),
                destination_directory: target.path.clone(),
                source_directory_identity: source.identity.clone(),
                destination_directory_identity: target.identity.clone(),
                primary,
                same_filesystem: source.identity.device == target.identity.device,
                staging_identity: None,
                cleanup_staging_identity: None,
                parts,
            };
            journals.write(&journal)?;
            finish_move(&store, &journals, &mut journal, &mut proven)
                .map_err(|error| failed_move(&store, &journals, &journal, error))?;
            Ok::<_, String>(target.path.join(&journal.primary))
        })();
        match result {
            Ok(target) => moved.push(json!({"id":book.metadata.id, "title":book.metadata.title, "from":book.path, "to":target})),
            Err(error) => errors.push(json!({"id":book.metadata.id, "title":book.metadata.title, "error":error})),
        }
    }
    Ok(json!({"moved":moved,"errors":errors,"skipped":skipped}))
}

/// Startup recovery returns visible per-archive outcomes. It never repairs,
/// deletes exports, creates ownership markers or writes journals in read-only mode.
pub fn recover(store: &Store) -> Result<Value, String> {
    let _operation = OPERATIONS
        .lock()
        .map_err(|_| "Library operation lock unavailable")?;
    if store.settings()?.read_only {
        return Ok(json!({"moved":[],"errors":[],"skipped":0,"readOnly":true}));
    }
    let profile = Directory::open(&store.data_dir().canonicalize().map_err(err)?)?;
    if !profile.absent("attachment-exports-v1")? {
        cleanup_exports(&private_child(&profile, "attachment-exports-v1", false)?)?;
    }
    let Some(journals) = Journals::open(store, false)? else {
        return Ok(json!({"moved":[],"errors":[],"skipped":0}));
    };
    let mut moved = Vec::new();
    let mut errors = Vec::new();
    let mut entries = Vec::new();
    for entry in journals.directory.dir.entries().map_err(err)? {
        if entries.len() >= 1024 {
            return Err(
                "Too many migration journals; automatic recovery stopped without sweeping files"
                    .into(),
            );
        }
        let name = entry.map_err(err)?.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.ends_with(".json") {
            entries.push(name.to_string());
        }
    }
    entries.sort();
    for name in entries {
        let parsed = (|| {
            canonical_uuid(
                name.strip_suffix(".json")
                    .ok_or("Invalid journal filename")?,
            )?;
            let mut bytes = Vec::new();
            journals
                .directory
                .open_file(&name)?
                .take(MAX_JOURNAL + 1)
                .read_to_end(&mut bytes)
                .map_err(err)?;
            if bytes.len() as u64 > MAX_JOURNAL {
                return Err("Oversized migration journal".into());
            }
            #[cfg(desktop)]
            let journal: Journal = if store.portable_mode() {
                let mut value: Value = serde_json::from_slice(&bytes).map_err(err)?;
                store.portable_rebase_saved_json(crate::profile_paths::RecordKind::Migration, &mut value)?;
                serde_json::from_value(value).map_err(err)?
            } else {
                serde_json::from_slice(&bytes).map_err(err)?
            };
            #[cfg(not(desktop))]
            let journal: Journal = serde_json::from_slice(&bytes).map_err(err)?;
            if name != format!("{}.json", journal.id) {
                return Err("Migration journal filename mismatch".into());
            }
            Ok::<_, String>(journal)
        })();
        let mut journal = match parsed {
            Ok(journal) => journal,
            Err(error) => {
                errors.push(json!({"id":name,"title":name,"error":error}));
                continue;
            }
        };
        let result = (|| {
            if journal_orphaned(&journal, &journals, store)? {
                // Nothing can finish this move. Clear what an unfinished copy
                // left in its own staging folder where that is provably all
                // there is; every archive file stays where it is.
                let leftover = match clear_unpublished_copy(&journal) {
                    Ok(_) => String::new(),
                    Err(error) => format!(" {error}."),
                };
                journals.remove(&journal.id)?;
                return Err(format!(
                    "This archive is no longer in the library, so its unfinished move was dropped. Any of its files already in either folder were left in place.{leftover}"
                ));
            }
            // Replay has no in-process digest: every part is re-hashed.
            finish_move(store, &journals, &mut journal, &mut Vec::new())
                .map_err(|error| failed_move(store, &journals, &journal, error))
        })();
        // The directories, not the archive file: a warning names the two
        // folders the person chose.
        let from = crate::storage::display_path(&journal.source_directory);
        let to = crate::storage::display_path(&journal.destination_directory);
        match result {
            Ok(()) => moved.push(json!({"id":journal.book_id,"title":journal.title,"from":journal.source_directory.join(&journal.primary),"to":journal.destination_directory.join(&journal.primary)})),
            Err(error) => errors.push(json!({"id":name,"title":journal.title,"from":from,"to":to,"error":error})),
        }
    }
    Ok(json!({"moved":moved,"errors":errors,"skipped":0}))
}

/// The extension is generated from an exact MIME allowlist, never an archive
/// filename. No HTML/SVG/XML, executables, scripts, URLs or arbitrary OS schemes.
fn attachment_extension(mime: &str) -> Option<&'static str> {
    match mime.split(';').next()?.trim().to_ascii_lowercase().as_str() {
        "application/pdf" => Some("pdf"),
        "application/epub+zip" => Some("epub"),
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        "image/bmp" => Some("bmp"),
        "image/tiff" => Some("tiff"),
        "image/avif" => Some("avif"),
        "audio/mpeg" => Some("mp3"),
        "audio/mp4" | "audio/x-m4a" => Some("m4a"),
        "audio/ogg" => Some("oga"),
        "audio/flac" | "audio/x-flac" => Some("flac"),
        "audio/wav" | "audio/x-wav" => Some("wav"),
        "audio/webm" => Some("weba"),
        "video/mp4" => Some("mp4"),
        "video/webm" => Some("webm"),
        "video/ogg" => Some("ogv"),
        "video/quicktime" => Some("mov"),
        "video/mpeg" => Some("mpeg"),
        "text/plain" => Some("txt"),
        "application/rtf" | "text/rtf" => Some("rtf"),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => Some("docx"),
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => Some("xlsx"),
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => Some("pptx"),
        "application/vnd.oasis.opendocument.text" => Some("odt"),
        "application/vnd.oasis.opendocument.spreadsheet" => Some("ods"),
        "application/vnd.oasis.opendocument.presentation" => Some("odp"),
        _ => None,
    }
}
fn signature_matches(extension: &str, bytes: &[u8]) -> bool {
    match extension {
        "pdf" => bytes.starts_with(b"%PDF-"),
        "epub" | "docx" | "xlsx" | "pptx" | "odt" | "ods" | "odp" => {
            bytes.starts_with(b"PK\x03\x04")
        }
        "png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "jpg" => bytes.starts_with(b"\xff\xd8\xff"),
        "gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        "webp" => bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"),
        "bmp" => bytes.starts_with(b"BM"),
        "tiff" => bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*"),
        "flac" => bytes.starts_with(b"fLaC"),
        "oga" | "ogv" => bytes.starts_with(b"OggS"),
        "wav" => bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE"),
        "mp3" => {
            bytes.starts_with(b"ID3")
                || bytes
                    .get(..2)
                    .is_some_and(|b| b[0] == 0xff && b[1] & 0xe0 == 0xe0)
        }
        "webm" | "weba" => bytes.starts_with(b"\x1aE\xdf\xa3"),
        "avif" | "mp4" | "m4a" | "mov" => {
            bytes.get(4..8) == Some(b"ftyp")
                || (extension == "mov" && bytes.get(4..8) == Some(b"moov"))
        }
        "mpeg" => bytes.starts_with(b"\0\0\x01"),
        "rtf" => bytes.starts_with(b"{\\rtf"),
        "txt" => !bytes.contains(&0),
        _ => false,
    }
}

#[cfg(mobile)]
pub fn mobile_owned_directory(profile: &Path, path: &Path) -> Result<(), String> {
    if !path.starts_with(profile) {
        return Err("Choose a folder inside Kiwix's own storage".into());
    }
    Directory::open(path)?.unchanged()
}

/// A historical pathname is only a lookup hint. Re-admission requires this
/// publication proof to match the same retained inode that receives policy.
#[cfg(any(target_os = "ios", test))]
pub(crate) struct BackupEvidence {
    pub size: u64,
    pub identity: Option<(u64, u64)>,
    pub sha256: Option<[u8; 32]>,
}

#[cfg(any(target_os = "ios", test))]
impl BackupEvidence {
    fn matches(evidence: &[Self], file: &File) -> Result<bool, String> {
        use std::io::{Seek, SeekFrom};
        let metadata = Metadata::from_file(file).map_err(err)?;
        if !metadata.is_file() || metadata.nlink() != 1 { return Ok(false); }
        let before = FileStamp::of(&metadata)?;
        let eligible = |proof: &&Self| proof.size == before.size
            && proof.identity.is_none_or(|identity| identity == (metadata.dev(), metadata.ino()))
            && (proof.identity.is_some() || proof.sha256.is_some());
        if !evidence.iter().any(|proof| eligible(&proof)) { return Ok(false); }
        FileExt::try_flock_shared(file).map_err(|error| format!("Archive is busy: {error}"))?;
        let identity_proven = evidence.iter().filter(eligible)
            .any(|proof| proof.identity.is_some() && proof.sha256.is_none());
        if !identity_proven {
            let mut input = file;
            input.seek(SeekFrom::Start(0)).map_err(err)?;
            let mut input = input.take(before.size.checked_add(1).ok_or("Archive size overflow")?);
            let mut digest = Sha256::new();
            let mut buffer = [0u8; 64 * 1024];
            let mut size = 0u64;
            loop {
                let count = input.read(&mut buffer).map_err(err)?;
                if count == 0 { break; }
                size += count as u64;
                digest.update(&buffer[..count]);
            }
            let actual: [u8; 32] = digest.finalize().into();
            if size != before.size || !evidence.iter().filter(eligible).any(|proof| proof.sha256 == Some(actual)) {
                return Ok(false);
            }
        }
        Ok(FileStamp::of(&Metadata::from_file(file).map_err(err)?)? == before)
    }
}

#[cfg(any(target_os = "ios", test))]
fn retained_backup_archive_valid(
    directory: &Path, files: &[(String, File)], uuid: &str,
) -> Result<bool, String> {
    let archive = match Archive::open_files(directory, files) {
        Ok(archive) => archive,
        Err(_) => return Ok(false), // A stale completion confers no archive authority.
    };
    let Ok(metadata) = archive.metadata() else { return Ok(false); };
    if metadata.id != uuid { return Ok(false); }
    Ok(archive.check_integrity().unwrap_or(false))
}

/// Read only a schema-owned acquisition record through the profile capability.
/// This does not construct a manager, start recovery, or grant network admission.
#[cfg(target_os = "ios")]
pub(crate) fn backup_record<T: serde::de::DeserializeOwned>(
    store: &Store, components: &[&str], limit: u64, kind: crate::mobile_rebase::RecordKind,
) -> Result<Option<T>, String> {
    let mut path = store.data_dir().to_path_buf();
    for component in components {
        filename(component)?;
        path.push(component);
    }
    if !path.try_exists().map_err(err)? { return Ok(None); }
    let directory = Directory::open(path.parent().ok_or("A saved backup could not be read. Restart the app")?)?;
    let file = directory.open_file(components.last().ok_or("A saved backup could not be read. Restart the app")?)?;
    let metadata = Metadata::from_file(&file).map_err(err)?;
    FileStamp::of(&metadata)?;
    if metadata.len() > limit { return Err("Acquisition backup evidence exceeds its state limit".into()); }
    let mut value = crate::acquisition::read_history_json(file, limit).map_err(err)?
        .ok_or("Acquisition backup evidence exceeds its state limit")?;
    store.mobile_rebase_saved_json(kind, &mut value)?;
    directory.unchanged()?;
    serde_json::from_value(value).map(Some).map_err(err)
}

/// Enumerate every supported profile-owned location without following links.
/// The visitor decides admission from Store/publication evidence, never names.
#[cfg(any(target_os = "ios", test))]
fn visit_backup_files(
    profile: &Path, mut visit: impl FnMut(&File, &Path) -> Result<(), String>,
) -> Result<(), String> {
    let mut pending = vec![Directory::open(profile)?];
    while let Some(directory) = pending.pop() {
        directory.unchanged()?;
        for entry in directory.dir.entries().map_err(err)? {
            let entry = entry.map_err(err)?;
            let kind = entry.file_type().map_err(err)?;
            if kind.is_symlink() { continue; }
            let Ok(name) = entry.file_name().into_string() else { continue; };
            if kind.is_dir() {
                let dir = match directory.dir.open_dir_nofollow(&name) {
                    Ok(dir) => dir,
                    Err(error) if matches!(error.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory) => continue,
                    #[cfg(unix)]
                    Err(error) if error.raw_os_error() == Some(libc::ELOOP) => continue,
                    Err(error) => return Err(err(error)),
                };
                let identity = Identity::of(&dir.dir_metadata().map_err(err)?);
                pending.push(Directory { path: directory.path.join(&name), dir, identity });
            } else if kind.is_file() {
                let mut options = OpenOptions::new();
                options.read(true).follow(FollowSymlinks::No);
                #[cfg(unix)]
                {
                    use cap_std::fs::OpenOptionsExt;
                    options.custom_flags(libc::O_NONBLOCK);
                }
                let file = match directory.dir.open_with(&name, &options) {
                    Ok(file) => file,
                    Err(error) if matches!(error.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory) => continue,
                    #[cfg(unix)]
                    Err(error) if error.raw_os_error() == Some(libc::ELOOP) => continue,
                    Err(error) => return Err(err(error)),
                };
                let metadata = file.metadata().map_err(err)?;
                let file = file.into_std();
                if metadata.is_file() && metadata.nlink() == 1 {
                    visit(&file, &directory.path.join(&name))?;
                }
            }
        }
        directory.unchanged()?;
    }
    Ok(())
}

/// These namespaces identify owned payloads for conservative EXCLUSION only.
/// They never grant admission: preferences/secrets outside payload namespaces
/// are untouched, and hidden names receive exactly the same proof requirements.
#[cfg(any(target_os = "ios", test))]
fn backup_payload_path(profile: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(profile) else { return false; };
    if relative.starts_with("maps/map-parts") { return true; }
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else { return false; };
    let bytes = name.as_bytes();
    if bytes.len() >= 4 && bytes[bytes.len() - 4..].eq_ignore_ascii_case(b".zim") { return true; }
    if bytes.len() >= 6 && bytes[bytes.len() - 6..bytes.len() - 2].eq_ignore_ascii_case(b".zim")
        && bytes[bytes.len() - 2..].iter().all(u8::is_ascii_alphabetic)
    {
        return true;
    }
    let operation = name.strip_prefix(".kiwix-").and_then(|name| name.strip_suffix(".part"))
        .or_else(|| name.strip_prefix(".streetzim-").and_then(|name| name.strip_suffix(".part")))
        .or_else(|| name.strip_prefix(".kiwix-publish-"));
    operation.is_some_and(|id| id.len() == 36 && Uuid::parse_str(id).is_ok())
}

#[cfg(target_os = "ios")]
pub(crate) fn reconcile_archive_backup(store: &Store) -> Result<(), String> {
    let profile = store.data_dir();
    let mut handled = HashSet::new();
    // Store admission proves completed native imports, including hidden names;
    // provider originals and missing/unavailable books never gain eligibility.
    for book in store.read()?.books {
        let path = Path::new(&book.path);
        if book.source_reference.is_some() || !path.starts_with(profile)
            || !path.try_exists().map_err(err)? { continue; }
        // A stale library pathname is not admission for a replacement inode.
        let Ok(retained) = retain_archive_files(&book) else { continue; };
        let mut marked = true;
        for (name, file) in &retained.files {
            marked &= crate::mobile_backup::admitted(file, &retained.path.join(name))?;
        }
        if !marked && !retained_backup_archive_valid(&retained.path, &retained.files, &book.metadata.id)? {
            continue;
        }
        retained_source_unchanged(&retained)?;
        for (name, file) in &retained.files {
            let path = retained.path.join(name);
            crate::mobile_backup::archive_policy(file, &path)?;
            handled.insert(path);
        }
        retained_source_unchanged(&retained)?;
    }
    let mut completed: std::collections::HashMap<PathBuf, Vec<BackupEvidence>> = std::collections::HashMap::new();
    let evidence = crate::acquisition::backup_evidence(store)?.into_iter()
        .chain(crate::torrent::backup_evidence(store)?)
        .chain(crate::maps::backup_evidence(store)?)
        .chain(crate::localswarm::backup_evidence(store)?);
    for (path, proof) in evidence {
        completed.entry(path).or_default().push(proof);
    }
    visit_backup_files(profile, |file, path| {
        if handled.contains(path) { return Ok(()); }
        if crate::mobile_backup::admitted(file, path)? {
            return crate::mobile_backup::archive_policy(file, path);
        }
        if !backup_payload_path(profile, path) { return Ok(()); }
        let verified = match completed.get(path) {
            Some(evidence) => BackupEvidence::matches(evidence, file)?,
            None => false,
        };
        if verified {
            crate::mobile_backup::archive_policy(file, path)
        } else {
            // Completion status, naming, and self-validity alone cannot promote
            // a historical replacement. The native conditional transaction
            // rechecks admission so this stale scan cannot undo publication.
            crate::mobile_backup::exclude_unadmitted_payload(file, path)
        }
    })
}

#[cfg(mobile)]
pub fn mobile_archive_directory(profile: &Path) -> Result<PathBuf, String> {
    let profile = Directory::open(profile)?;
    Ok(private_child(&profile, "archives", true)?.path)
}

/// Native providers receive only fresh application-owned capabilities, never an
/// arbitrary caller pathname or a content URI interpreted as a filesystem path.
#[cfg(mobile)]
pub fn mobile_directory(profile: &Path, collection: &str) -> Result<PathBuf, String> {
    let profile = Directory::open(profile)?;
    let collection = private_child(&profile, collection, true)?;
    let name = Uuid::new_v4().to_string();
    let mut builder = DirBuilder::new();
    #[cfg(unix)]
    {
        use cap_std::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    collection.dir.create_dir_with(&name, &builder).map_err(err)?;
    collection.sync()?;
    Ok(private_child(&collection, &name, false)?.path)
}

#[cfg(mobile)]
pub fn mobile_import_paths<'a>(root: &Path, paths: impl IntoIterator<Item = &'a str>) -> Result<(), String> {
    let root = Directory::open(root)?;
    let mut unique = HashSet::new();
    for raw in paths {
        let path = Path::new(raw);
        if !path.starts_with(&root.path) || path == root.path {
            return Err("Native import returned a path outside its owned staging directory".into());
        }
        let parent = Directory::open(path.parent().ok_or("Imported document has no directory")?)?;
        let name = basename(path)?;
        parent.stamp(&name)?;
        parent.unchanged()?;
        if !unique.insert(raw.to_lowercase()) {
            return Err("Native import returned duplicate document paths".into());
        }
    }
    root.unchanged()?;
    root.sync()?;
    Ok(())
}

/// A selected split cohort is one archive, not one import per physical chunk.
/// Keep every copied chunk on disk, but admit only its .zimaa primary to libzim.
#[cfg(mobile)]
pub fn mobile_primary_archives(paths: Vec<String>) -> Result<Vec<String>, String> {
    if paths.iter().all(|path|path.get(path.len().saturating_sub(4)..).is_some_and(|suffix|suffix.eq_ignore_ascii_case(".zim"))) {
        return Ok(paths);
    }
    let primaries: HashSet<_> = paths.iter().filter(|path| {
        let name=path.to_ascii_lowercase();
        name.ends_with(".zim")||name.ends_with(".zimaa")
    }).map(|path|path.to_lowercase()).collect();
    for raw in &paths {
        let path=Path::new(raw);
        let name=basename(path)?;
        let lower=name.to_ascii_lowercase();
        if lower.ends_with(".zim")||lower.ends_with(".zimaa") {continue;}
        let suffix=lower.rsplit_once(".zim").map(|(_,suffix)|suffix).ok_or("Selected document is not a ZIM archive or exact split part")?;
        if suffix.len()!=2||!suffix.bytes().all(|byte|byte.is_ascii_lowercase()) {
            return Err("Selected document is not an exact split archive part".into());
        }
        let primary=path.with_file_name(format!("{}aa",&name[..name.len()-2]));
        if !primaries.contains(&primary.to_string_lossy().to_lowercase()) {
            return Err(format!("Split archive selection is incomplete: {} requires its .zimaa primary and full cohort",path.display()));
        }
    }
    Ok(paths.into_iter().filter(|path| {
        let name=path.to_ascii_lowercase();
        name.ends_with(".zim")||name.ends_with(".zimaa")
    }).collect())
}

#[cfg(mobile)]
pub fn mobile_export_path(profile: &Path, suggested_name: &str) -> Result<PathBuf, String> {
    filename(suggested_name)?;
    Ok(mobile_directory(profile, "mobile-exports-v1")?.join(suggested_name))
}

#[cfg(mobile)]
pub fn mobile_write_export(profile: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let path = mobile_export_path(profile, name)?;
    let directory = Directory::open(path.parent().ok_or("Export has no directory")?)?;
    let mut file = create_file(&directory, name)?;
    let result = (|| {
        file.write_all(bytes).map_err(err)?;
        file.sync_all().map_err(err)?;
        directory.stamp(name)?;
        directory.unchanged()?;
        directory.sync()
    })();
    result.map_err(|error| format!("Preparing mobile export failed; owned output retained at {}: {error}", path.display()))?;
    Ok(path)
}

#[cfg(mobile)]
pub struct MobileExport {
    collection: Directory,
    directory: Directory,
    directory_name: String,
    name: String,
    stamp: FileStamp,
}
#[cfg(mobile)]
impl MobileExport {
    pub fn pin(profile: &Path, path: &Path) -> Result<Self, String> {
        let profile = Directory::open(profile)?;
        let collection = private_child(&profile, "mobile-exports-v1", false)?;
        let parent = path.parent().ok_or("Export has no directory")?;
        let directory_name = basename(parent)?;
        canonical_uuid(&directory_name)?;
        if parent != collection.path.join(&directory_name) {
            return Err("Kiwix will not clean up files outside its own export folder".into());
        }
        let directory = private_child(&collection, &directory_name, false)?;
        let name = basename(path)?;
        let stamp = directory.stamp(&name)?;
        directory.unchanged()?;
        Ok(Self { collection, directory, directory_name, name, stamp })
    }
    pub fn cleanup(self) -> Result<(), String> {
        self.collection.unchanged()?;
        self.directory.unchanged()?;
        if self.directory.stamp(&self.name)? != self.stamp {
            return Err(format!("Prepared export changed identity; retained at {}", self.directory.path.display()));
        }
        self.directory.dir.remove_file(&self.name).map_err(err)?;
        self.directory.sync()?;
        // Fails rather than removing an unexpected file in this private directory.
        self.collection.dir.remove_dir(&self.directory_name).map_err(err)?;
        self.collection.sync()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedAttachment {
    pub path: PathBuf,
    pub mime: String,
    pub size: u64,
    pub retained_days: u32,
}

/// Only call after a trusted application-chrome click and explicit confirmation.
/// The caller resolves URL encoding once and passes the canonical libzim path.
/// Files live for 30 days, not just until this call returns: external readers may
/// open lazily or hold the document open. Never clean an export opened this run.
pub fn prepare_attachment(
    archive: &Archive,
    canonical_entry_path: &str,
    profile: &Path,
) -> Result<PreparedAttachment, String> {
    if canonical_entry_path.is_empty()
        || canonical_entry_path.len() > 64 * 1024
        || canonical_entry_path.contains('\0')
    {
        return Err("Invalid canonical attachment path".into());
    }
    let entry = archive.entry(canonical_entry_path)?;
    let extension = attachment_extension(&entry.mime).ok_or("This attachment type cannot be opened safely in an external application. Use Save attachment instead.")?;
    if entry.size == 0 || entry.size > MAX_ATTACHMENT {
        return Err("External attachments must be between 1 byte and 512 MiB. Use Save attachment for larger files.".into());
    }
    let first = archive.read(&entry.path, 0, entry.size.min(CHUNK))?;
    if !signature_matches(extension, &first) {
        return Err("Attachment contents do not match the permitted document/media type. Use Save attachment instead.".into());
    }
    let profile = Directory::open(&profile.canonicalize().map_err(err)?)?;
    let exports = private_child(&profile, "attachment-exports-v1", true)?;
    cleanup_exports(&exports)?;
    let name = format!("{}.{}", Uuid::new_v4(), extension);
    let mut output = create_file(&exports, &name)?;
    let result = (|| {
        output.write_all(&first).map_err(err)?;
        let mut offset = first.len() as u64;
        while offset < entry.size {
            let bytes = archive.read(&entry.path, offset, (entry.size - offset).min(CHUNK))?;
            if bytes.is_empty() {
                return Err("Attachment ended before its declared size".into());
            }
            output.write_all(&bytes).map_err(err)?;
            offset += bytes.len() as u64;
        }
        output.sync_all().map_err(err)?;
        drop(output);
        exports.sync()?;
        exports.unchanged()?;
        let destination = exports.path.join(&name);
        Ok(PreparedAttachment { path: destination, mime: entry.mime, size: entry.size, retained_days: 30 })
    })();
    if result.is_err() {
        let _ = exports.dir.remove_file(&name);
    }
    result
}
fn cleanup_exports(exports: &Directory) -> Result<(), String> {
    for (index, entry) in exports.dir.entries().map_err(err)?.enumerate() {
        if index >= 4096 {
            break;
        }
        let entry = entry.map_err(err)?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some((id, extension)) = name.rsplit_once('.') else {
            continue;
        };
        if canonical_uuid(id).is_err()
            || !matches!(
                extension,
                "pdf"
                    | "epub"
                    | "png"
                    | "jpg"
                    | "gif"
                    | "webp"
                    | "bmp"
                    | "tiff"
                    | "avif"
                    | "mp3"
                    | "m4a"
                    | "oga"
                    | "flac"
                    | "wav"
                    | "weba"
                    | "mp4"
                    | "webm"
                    | "ogv"
                    | "mov"
                    | "mpeg"
                    | "txt"
                    | "rtf"
                    | "docx"
                    | "xlsx"
                    | "pptx"
                    | "odt"
                    | "ods"
                    | "odp"
            )
        {
            continue;
        }
        let file = match exports.open_file(name) {
            Ok(file) => file,
            Err(_) => continue,
        };
        let modified = file.metadata().map_err(err)?.modified().map_err(err)?;
        let cutoff = *EXPORT_SESSION_STARTED;
        if cutoff.duration_since(modified).unwrap_or_default() <= EXPORT_RETENTION {
            continue;
        }
        // Advisory locking protects cooperating readers; OS unlink semantics do
        // not interrupt already-open Unix descriptors. Windows denies busy files.
        if file.try_flock_exclusive().is_err() {
            continue;
        }
        let before = exports.stamp(name)?;
        if before.modified_ns
            >= cutoff
                .duration_since(UNIX_EPOCH)
                .map_err(err)?
                .as_nanos()
                .saturating_sub(EXPORT_RETENTION.as_nanos())
        {
            continue;
        }
        if exports.stamp(name)? == before {
            let _ = exports.dir.remove_file(name);
        }
    }
    Ok(())
}

/// Test-only fault injection shared by every `move_exclusive` implementation.
#[cfg(test)]
fn simulated_move_failure() -> Result<(), String> {
    let mut failure = MOVE_FAILURE_AT.lock().unwrap();
    if let Some((thread, remaining)) = *failure {
        if thread == std::thread::current().id() {
            if remaining <= 1 {
                *failure = None;
                return Err("Exclusive archive rename: simulated cross-device move".into());
            }
            *failure = Some((thread, remaining - 1));
        }
    }
    Ok(())
}

/// *Exclusive* cross-directory rename, anchored to both open directory
/// handles. It never overwrites and never copies. Where the filesystem cannot
/// rename exclusively (NFS, FUSE without rename2) the shared primitive moves
/// the entry by an equally exclusive link and a captured unlink through the
/// same two handles.
#[cfg(any(target_os = "macos", target_os = "ios", target_os = "linux", target_os = "android"))]
fn move_exclusive(
    source: &Directory,
    from: &str,
    destination: &Directory,
    to: &str,
) -> Result<(), String> {
    filename(from)?;
    filename(to)?;
    #[cfg(test)]
    simulated_move_failure()?;
    let from = std::ffi::CString::new(from).map_err(err)?;
    let to = std::ffi::CString::new(to).map_err(err)?;
    crate::publication::rename_noreplace_at(&source.dir, &from, &destination.dir, &to)
        .map_err(|error| format!("Exclusive archive rename: {error}"))
}
#[cfg(windows)]
fn move_exclusive(
    source: &Directory,
    from: &str,
    destination: &Directory,
    to: &str,
) -> Result<(), String> {
    use cap_std::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    filename(from)?;
    filename(to)?;
    #[cfg(test)]
    simulated_move_failure()?;
    let mut options = OpenOptions::new();
    options
        .access_mode(0x0001_0000 | 0x0000_0080)
        .follow(FollowSymlinks::No)
        .custom_flags(0x0020_0000);
    let file = source.dir.open_with(from, &options).map_err(err)?;
    if !file.metadata().map_err(err)?.is_file() {
        return Err("Rename source must be regular".into());
    }
    crate::storage::rename_into_directory_noreplace(
        file.as_raw_handle(),
        destination.dir.as_raw_handle(),
        std::ffi::OsStr::new(to),
    )
    .map_err(err)
}

#[cfg(test)]
#[path = "library_files_tests.rs"]
mod tests;
