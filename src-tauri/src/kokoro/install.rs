// SPDX-License-Identifier: GPL-3.0-or-later
//! Install a checked archive into a private directory and publish its ownership
//! record. The directory name never changes. An incomplete install has no
//! published record, so the engine cannot observe a partially extracted model.
#![forbid(unsafe_code)]

use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use cap_std::fs::{Dir, Metadata, OpenOptions};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fs::File,
    io::{self, BufRead, BufReader, Read, Seek, Write},
    path::{Component, Path, PathBuf},
};
use tokio_util::sync::CancellationToken;

const PENDING: &str = ".voice-pending-";
const PREVIOUS: &str = "voice-previous-v1.jsonl";
const EARLIER: &str = "voice-earlier-generation-v1";
const DETACHED: &str = "voice-legacy-preserved-v1";
const REMOVING: &str = "voice-removing-v1.jsonl";
const INSTALLED: &str = "voice-installed-v1.jsonl";
const LOCK: &str = ".voice-install-v1.lock";
const TAR_BYTES: u64 = 221_801_472;
const TAR_SHA: &str = "ce23b8a10c500f7897abdcfbe1e35d711235fc6e8abb1228914dc71bb5fd192e";
const MAX_RECORD: u64 = 256 * 1024;
const MAX_ENTRIES: usize = 1024;
const REQUIRED: [&str; 5] = [
    "model.fp16.onnx",
    "voices.bin",
    "tokens.txt",
    "lexicon-us-en.txt",
    "lexicon-gb-en.txt",
];

type Result<T> = io::Result<T>;
type Id = (u64, u64);

fn changed() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "A natural voice installation entry changed. Its files were preserved.",
    )
}

fn id(m: &Metadata) -> Id {
    (m.dev(), m.ino())
}

fn regular(m: &Metadata) -> bool {
    m.is_file() && !m.is_symlink() && m.nlink() == 1
}

fn check_cancel(cancel: &CancellationToken) -> Result<()> {
    if cancel.is_cancelled() {
        Err(io::Error::other("The voice model download was cancelled"))
    } else {
        Ok(())
    }
}

fn file(dir: &Dir, name: &OsStr, create: bool) -> Result<File> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(create)
        .create_new(create)
        .follow(FollowSymlinks::No);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use cap_std::fs::OpenOptionsExt;
        options
            .custom_flags(0x0020_0000)
            .share_mode(if create { 1 | 4 } else { 1 | 2 | 4 });
    }
    let opened = dir.open_with(name, &options)?;
    if !regular(&opened.metadata()?) {
        return Err(changed());
    }
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt as UnixMetadata;
        let m = opened.metadata()?;
        if m.uid() != crate::fs_ext::effective_uid() || m.mode() & 0o022 != 0 {
            return Err(permissions());
        }
    }
    #[cfg(target_vendor = "apple")]
    if crate::storage::has_extended_file_security(&opened)? {
        return Err(permissions());
    }
    #[cfg(windows)]
    crate::private_sync_identity::check_cache_file_access(&opened).map_err(|_| permissions())?;
    Ok(opened.into_std())
}

fn private_directory(dir: &Dir) -> Result<()> {
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt as UnixMetadata;
        let m = dir.dir_metadata()?;
        if m.uid() != crate::fs_ext::effective_uid() || m.mode() & 0o022 != 0 {
            return Err(permissions());
        }
    }
    #[cfg(target_vendor = "apple")]
    if crate::storage::has_extended_file_security(dir)? {
        return Err(permissions());
    }
    #[cfg(windows)]
    crate::private_sync_identity::check_cache_directory_access(dir).map_err(|_| permissions())?;
    Ok(())
}

fn permissions() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied,
        "Use a local voice model folder owned by your account, without write access for other users or an extended Apple access list. Existing files were preserved.")
}

fn checked_dir(parent: &Dir, name: impl AsRef<Path>) -> Result<Dir> {
    let dir = parent.open_dir_nofollow(name)?;
    private_directory(&dir)?;
    Ok(dir)
}

fn new_directory(dir: &Dir, name: &OsStr) -> Result<Dir> {
    #[cfg_attr(windows, allow(unused_mut))]
    let mut options = cap_std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use cap_std::fs::DirBuilderExt;
        options.mode(0o700);
    }
    dir.create_dir_with(name, &options)?;
    checked_dir(dir, name)
}

fn sync(dir: &Dir) -> Result<()> {
    #[cfg(unix)]
    crate::storage::sync_cap_directory(dir)?;
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

fn open_root(path: &Path, create: bool) -> Result<Dir> {
    let (parent, name) = (
        path.parent().ok_or_else(changed)?,
        path.file_name().ok_or_else(changed)?,
    );
    let parent = crate::storage::open_directory_nofollow(parent)?;
    private_directory(&parent)?;
    match checked_dir(&parent, name) {
        Err(e) if create && e.kind() == io::ErrorKind::NotFound => {
            let result = new_directory(&parent, name);
            match result {
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => checked_dir(&parent, name),
                Ok(dir) => {
                    sync(&parent)?;
                    Ok(dir)
                }
                err => err,
            }
        }
        result => result,
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    #[serde(default)]
    bundled: bool,
    record_id: Id,
    directory: String,
    parent_id: Id,
    directory_id: Id,
}

impl Header {
    fn open(&self, parent: &Dir) -> Result<Dir> {
        if self.parent_id != id(&parent.dir_metadata()?) {
            return Err(changed());
        }
        let uuid = self
            .directory
            .strip_prefix(".voice-pack-")
            .ok_or_else(changed)?;
        if uuid::Uuid::parse_str(uuid)
            .map_err(|_| changed())?
            .to_string()
            != uuid
        {
            return Err(changed());
        }
        let dir = checked_dir(parent, &self.directory)?;
        if id(&dir.dir_metadata()?) != self.directory_id {
            return Err(changed());
        }
        Ok(dir)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Member {
    path: PathBuf,
    identity: Id,
    // Directories have no size. Files record the expected final length before
    // writing, so an interrupted partial file can still be identified.
    bytes: Option<u64>,
}

struct WrittenMember {
    path: PathBuf,
    bytes: u64,
    hash: chksum_hash_sha2_256::Update,
}

struct Journal {
    writing: Option<WrittenMember>,
    header: Header,
    members: BTreeMap<PathBuf, Member>,
    file: File,
}

fn relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .to_str()
            .is_some_and(|s| s.len() <= 512 && !s.contains(['\\', ':', '\0']))
        && path.components().all(|c| matches!(c, Component::Normal(_)))
}

fn read_line<T: serde::de::DeserializeOwned>(reader: &mut impl BufRead) -> Result<Option<T>> {
    let mut line = Vec::new();
    if reader.take(2048).read_until(b'\n', &mut line)? == 0 {
        return Ok(None);
    }
    if line.last() != Some(&b'\n') {
        return Err(changed());
    }
    serde_json::from_slice(&line)
        .map(Some)
        .map_err(|_| changed())
}

fn encode_record(mut out: impl Write, row: &impl Serialize) -> Result<()> {
    serde_json::to_writer(&mut out, row)?;
    out.write_all(b"\n")
}

fn write_record(file: &mut File, row: &impl Serialize) -> Result<()> {
    encode_record(&mut *file, row)?;
    file.sync_data()
}

impl Journal {
    fn pending_name(&self) -> String {
        format!("{PENDING}{}.jsonl", self.header.directory)
    }

    fn create(parent: &Dir, bundled: bool) -> Result<(Self, Dir)> {
        let directory = format!(".voice-pack-{}", uuid::Uuid::new_v4());
        let dir = new_directory(parent, directory.as_ref())?;
        let draft = format!(".voice-header-{}", uuid::Uuid::new_v4());
        let mut handle = file(parent, draft.as_ref(), true)?;
        let record_id = id(&Metadata::from_file(&handle)?);
        let header = Header {
            bundled,
            record_id,
            directory,
            parent_id: id(&parent.dir_metadata()?),
            directory_id: id(&dir.dir_metadata()?),
        };
        let pending = format!("{PENDING}{}.jsonl", header.directory);
        // Only a fully flushed header can acquire its recoverable name. A kill
        // earlier may leave an unreferenced private header or empty directory.
        write_record(&mut handle, &header)?;
        let verified =
            crate::publication::VerifiedFile::new(handle.try_clone()?, handle.metadata()?.len())?;
        crate::publication::publish_verified_noreplace_in(
            parent,
            draft.as_ref(),
            pending.as_ref(),
            &verified,
        )?;
        sync(parent)?;
        Ok((
            Self {
                header,
                writing: None,
                members: BTreeMap::new(),
                file: handle,
            },
            dir,
        ))
    }

    fn load(parent: &Dir, name: &str) -> Result<Self> {
        let handle = file(parent, name.as_ref(), false)?;
        if handle.metadata()?.len() > MAX_RECORD {
            return Err(changed());
        }
        let mut reader = BufReader::new((&handle).take(MAX_RECORD + 1));
        let header: Header = read_line(&mut reader)?.ok_or_else(changed)?;
        if id(&Metadata::from_file(&handle)?) != header.record_id
            || id(&parent.dir_metadata()?) != header.parent_id
            || header.directory == super::MODEL_DIR
        {
            return Err(changed());
        }
        let expected = layout()
            .filter(|(p, _)| *p != Path::new("."))
            .map(|(p, info)| (p, info.map(|(size, _)| size)))
            .collect::<BTreeMap<_, _>>();
        let mut members = BTreeMap::new();
        while let Some(member) = read_line::<Member>(&mut reader)? {
            if members.len() >= MAX_ENTRIES
                || expected.get(member.path.as_path()) != Some(&member.bytes)
                || !relative(&member.path)
                || members.insert(member.path.clone(), member).is_some()
            {
                return Err(changed());
            }
        }
        Ok(Self {
            header,
            writing: None,
            members,
            file: handle,
        })
    }

    fn add(&mut self, path: &Path, metadata: &Metadata, bytes: Option<u64>) -> Result<()> {
        if self.members.len() >= MAX_ENTRIES || self.members.contains_key(path) {
            return Err(changed());
        }
        let row = Member {
            path: path.to_owned(),
            identity: id(metadata),
            bytes,
        };
        write_record(&mut self.file, &row)?;
        self.members.insert(row.path.clone(), row);
        Ok(())
    }
}

fn subdir(members: &BTreeMap<PathBuf, Member>, root: &Dir, path: &Path) -> Result<Dir> {
    let mut dir = root.try_clone()?;
    let mut prefix = PathBuf::new();
    for component in path.components() {
        let Component::Normal(name) = component else {
            return Err(changed());
        };
        prefix.push(name);
        let expected = members.get(&prefix).ok_or_else(changed)?;
        dir = checked_dir(&dir, name)?;
        if expected.bytes.is_some() || expected.identity != id(&dir.dir_metadata()?) {
            return Err(changed());
        }
    }
    Ok(dir)
}

fn validate(members: &BTreeMap<PathBuf, Member>, dir: &Dir, complete: bool) -> Result<()> {
    let mut count = 0;
    let mut stack = vec![(PathBuf::new(), dir.try_clone()?)];
    while let Some((prefix, dir)) = stack.pop() {
        for entry in dir.entries()? {
            let name = entry?.file_name();
            let path = prefix.join(&name);
            let m = dir.symlink_metadata(&name)?;
            let wanted = members
                .get(&path)
                .or_else(|| {
                    if complete {
                        return None;
                    }
                    members.values().find(|v| {
                        v.bytes.is_some()
                            && v.identity == id(&m)
                            && v.path.parent() == Some(prefix.as_path())
                            && OsStr::new(&capture_name(v.identity)) == name
                    })
                })
                .ok_or_else(changed)?;
            if id(&m) != wanted.identity || m.is_symlink() {
                return Err(changed());
            }
            if let Some(size) = wanted.bytes {
                if !regular(&m) || (complete && m.len() != size) || m.len() > size {
                    return Err(changed());
                }
            } else {
                let child = checked_dir(&dir, &name)?;
                if id(&child.dir_metadata()?) != wanted.identity {
                    return Err(changed());
                }
                stack.push((path, child));
            }
            count += 1;
        }
    }
    if complete
        && (count != members.len()
            || !REQUIRED
                .iter()
                .all(|n| members.get(Path::new(n)).is_some_and(|m| m.bytes.is_some()))
            || members
                .get(Path::new("espeak-ng-data"))
                .is_none_or(|m| m.bytes.is_some()))
    {
        return Err(changed());
    }
    Ok(())
}

fn capture_name(identity: Id) -> String {
    format!(".voice-remove-{:016x}-{:016x}", identity.0, identity.1)
}

// A kill between capture and unlink leaves the same deterministic, owned
// name. Resuming never renames or unlinks any other occupant of either slot.
fn remove_owned_file(dir: &Dir, name: &OsStr, identity: Id) -> Result<()> {
    let captured = capture_name(identity);
    let retained = match file(dir, captured.as_ref(), false) {
        Ok(saved) => {
            let m = Metadata::from_file(&saved)?;
            if id(&m) != identity {
                return Err(changed());
            }
            crate::publication::VerifiedFile::new(saved, m.len())?
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let opened = file(dir, name, false)?;
            let m = Metadata::from_file(&opened)?;
            if id(&m) != identity {
                return Err(changed());
            }
            let retained = crate::publication::VerifiedFile::new(opened, m.len())?;
            crate::publication::publish_verified_noreplace_in(
                dir,
                Path::new(name),
                captured.as_ref(),
                &retained,
            )?;
            retained
        }
        Err(e) => return Err(e),
    };
    if !retained.matches_in(dir, captured.as_ref())? {
        return Err(changed());
    }
    dir.remove_file(captured)
}

fn remove_record(parent: &Dir, name: &str, journal: Journal) -> Result<()> {
    let record_id = id(&Metadata::from_file(&journal.file)?);
    drop(journal.file);
    remove_owned_file(parent, name.as_ref(), record_id)?;
    sync(parent)
}

// Keep every changed or unknown entry. Empty directories are removed through
// their open capabilities. An interrupted cleanup may resume after some entries
// have gone.
fn require_record(parent: &Dir, name: &str, journal: &Journal) -> Result<()> {
    let record = crate::publication::VerifiedFile::new(
        journal.file.try_clone()?,
        journal.file.metadata()?.len(),
    )?;
    if !record.matches_in(parent, name.as_ref())? {
        return Err(changed());
    }
    Ok(())
}

fn cleanup(parent: &Dir, name: &str, journal: Journal) -> Result<()> {
    require_record(parent, name, &journal)?;
    let dir = match journal.header.open(parent) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return remove_record(parent, name, journal)
        }
        result => result?,
    };
    validate(&journal.members, &dir, false)?;
    remove_members(&journal.members, &dir)?;
    dir.remove_open_dir()?;
    remove_record(parent, name, journal)
}

fn remove_members(members: &BTreeMap<PathBuf, Member>, dir: &Dir) -> Result<()> {
    for (path, member) in members.iter().rev() {
        let containing = match subdir(members, dir, path.parent().ok_or_else(changed)?) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            result => result?,
        };
        let name = path.file_name().ok_or_else(changed)?;
        match containing.symlink_metadata(name) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                if member.bytes.is_some() && containing.try_exists(capture_name(member.identity))? {
                    remove_owned_file(&containing, name, member.identity)?;
                }
                continue;
            }
            result => {
                if id(&result?) != member.identity {
                    return Err(changed());
                }
            }
        }
        if member.bytes.is_some() {
            remove_owned_file(&containing, name, member.identity)?;
        } else {
            let child = checked_dir(&containing, name)?;
            if id(&child.dir_metadata()?) != member.identity {
                return Err(changed());
            }
            child.remove_open_dir()?;
        }
    }
    Ok(())
}

// Move only our retained, explicitly named journal out of the admission slot.
// A directory with unrelated or unprovable contents stays in place.
fn verify_remaining(
    journal: &Journal,
    directory: &Dir,
    reference: Option<(&Dir, &Journal)>,
    cancel: &CancellationToken,
) -> Result<()> {
    let members = &journal.members;
    for (path, info) in layout() {
        check_cancel(cancel)?;
        let Some(row) = members.get(path) else {
            continue;
        };
        let Some((size, digest)) = info else { continue };
        let parent = match subdir(members, directory, path.parent().ok_or_else(changed)?) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            r => r?,
        };
        let name = Path::new(path.file_name().ok_or_else(changed)?);
        let capture = capture_name(row.identity);
        let actual = if parent.try_exists(name)? {
            name
        } else if parent.try_exists(&capture)? {
            Path::new(&capture)
        } else {
            continue;
        };
        let held = file(&parent, actual.as_os_str(), false)?;
        let metadata = Metadata::from_file(&held)?;
        if id(&metadata) != row.identity || metadata.len() > size {
            return Err(changed());
        }
        let live = journal
            .writing
            .as_ref()
            .filter(|written| written.path == path);
        if metadata.len() == size || live.is_some() {
            let live_digest;
            let (bytes, expected) = if let Some(written) = live {
                live_digest = written.hash.digest().to_hex_lowercase();
                (written.bytes, live_digest.as_str())
            } else {
                (size, digest)
            };
            authenticate_owned_member(&parent, actual, bytes, expected, row.identity, cancel)?;
        } else {
            // An interrupted writer did not seal this file. On an authenticated
            // retry, compare its actual prefix to the pinned full member.
            let (verified_dir, journal) = reference.ok_or_else(changed)?;
            let container = subdir(
                &journal.members,
                verified_dir,
                path.parent().ok_or_else(changed)?,
            )?;
            let original = authenticated_file(&container, name, size, digest, Some(cancel))?;
            let mut expected = original.file().try_clone()?;
            expected.rewind()?;
            let mut observed = held;
            let stamp = crate::integrity::ContentIdentity::from_handles(std::iter::once(&observed))
                .map_err(io::Error::other)?;
            let mut remaining = metadata.len();
            let (mut left, mut right) = (vec![0; 64 * 1024], vec![0; 64 * 1024]);
            while remaining != 0 {
                check_cancel(cancel)?;
                let n = remaining.min(left.len() as u64) as usize;
                expected.read_exact(&mut left[..n])?;
                observed.read_exact(&mut right[..n])?;
                if left[..n] != right[..n] {
                    return Err(changed());
                }
                remaining -= n as u64;
            }
            if stamp
                != crate::integrity::ContentIdentity::from_handles(std::iter::once(&observed))
                    .map_err(io::Error::other)?
            {
                return Err(changed());
            }
        }
    }
    Ok(())
}

fn recover(
    parent: &Dir,
    reference: Option<(&Dir, &Journal)>,
    cancel: &CancellationToken,
) -> Result<()> {
    check_cancel(cancel)?;
    for entry in parent.entries()? {
        check_cancel(cancel)?;
        let name = entry?.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with(PENDING) && !name.starts_with(".voice-remove-") {
            continue;
        }
        // Torn, unknown or substituted records are preserved. Their unique
        // names cannot block a new explicit install.
        let Ok(journal) = Journal::load(parent, name) else {
            continue;
        };
        if name == capture_name(journal.header.record_id)
            && !parent.try_exists(&journal.header.directory)?
        {
            remove_owned_file(parent, name.as_ref(), journal.header.record_id)?;
            sync(parent)?;
            continue;
        }
        if reference
            .is_some_and(|(_, current)| current.header.record_id == journal.header.record_id)
        {
            continue;
        }
        if name != journal.pending_name() {
            continue;
        }
        let dir = match journal.header.open(parent) {
            Ok(dir) => dir,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                remove_record(parent, name, journal)?;
                continue;
            }
            Err(_) => continue,
        };
        if validate(&journal.members, &dir, false).is_ok()
            && verify_remaining(&journal, &dir, reference, cancel).is_ok()
        {
            drop(dir);
            cleanup(parent, name, journal)?;
        }
    }
    check_cancel(cancel)
}

fn check_space(available: u64, download: bool) -> Result<()> {
    let required = 240 * 1024 * 1024 + if download { super::MODEL_BYTES } else { 0 };
    if available < required {
        return Err(io::Error::other(format!("Free at least {} MiB on the app storage volume and retry the natural voice installation.", required.div_ceil(1024 * 1024))));
    }
    Ok(())
}

pub(super) struct Lease {
    parent: Dir,
    _lock: File,
    root: PathBuf,
    previous: Option<crate::publication::VerifiedFile>,
}

impl Lease {
    fn open(path: &Path, create: bool) -> Result<Self> {
        let parent = open_root(path, create)?;
        let lock = match file(&parent, LOCK.as_ref(), true) {
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                file(&parent, LOCK.as_ref(), false)?
            }
            r => r?,
        };
        if lock.metadata()?.len() != 0 {
            return Err(changed());
        }
        use crate::fs_ext::FileExt;
        lock.try_flock_exclusive().map_err(|e| {
            io::Error::other(format!(
                "Another natural voice operation may be active. Try again after it finishes. {e}"
            ))
        })?;
        let lease = Self {
            parent,
            _lock: lock,
            root: path.to_owned(),
            previous: None,
        };
        lease.unchanged()?;
        Ok(lease)
    }

    fn unchanged(&self) -> Result<()> {
        let rebound = open_root(&self.root, false)?;
        if id(&rebound.dir_metadata()?) != id(&self.parent.dir_metadata()?)
            || id(&rebound.symlink_metadata(LOCK)?) != id(&Metadata::from_file(&self._lock)?)
        {
            return Err(changed());
        }
        Ok(())
    }

    pub(super) fn space_for(&self, download: bool) -> Result<()> {
        self.unchanged()?;
        check_space(crate::fs_ext::available_space(&self.root)?, download)
    }

    /// Never reopen the received bytes by a filesystem name.
    pub(super) fn scratch(&self) -> Result<File> {
        self.unchanged()?;
        let name = format!(".voice-receive-{}", uuid::Uuid::new_v4());
        #[cfg(unix)]
        {
            let handle = file(&self.parent, name.as_ref(), true)?;
            let identity = crate::publication::VerifiedFile::new(handle.try_clone()?, 0)?;
            if !identity.matches_in(&self.parent, name.as_ref())? {
                return Err(changed());
            }
            self.parent.remove_file(name)?;
            Ok(handle)
        }
        #[cfg(windows)]
        {
            use cap_std::fs::OpenOptionsExt;
            let mut opts = OpenOptions::new();
            opts.read(true)
                .write(true)
                .create_new(true)
                .follow(FollowSymlinks::No)
                .share_mode(0)
                .custom_flags(0x100 | 0x0400_0000);
            Ok(self.parent.open_with(name, &opts)?.into_std())
        }
    }

    #[cfg(test)]
    pub(super) fn admit(path: &Path) -> Result<Self> {
        Self::for_install(path, false, &CancellationToken::new())
    }

    pub(super) fn for_install(
        path: &Path,
        explicit: bool,
        cancel: &CancellationToken,
    ) -> Result<Self> {
        check_cancel(cancel)?;
        let mut lease = Self::open(path, true)?;
        recover_previous(&lease.parent, cancel)?;
        if lease.parent.try_exists(REMOVING)? {
            return Err(io::Error::other(
                "A previous voice removal is unfinished. Use Remove in Settings to retry cleanup.",
            ));
        }
        if lease.parent.try_exists(INSTALLED)? {
            if !explicit {
                return Err(io::Error::other(
                    "A natural voice installation is already present.",
                ));
            }
            let previous = Journal::load(&lease.parent, INSTALLED)?;
            lease.previous = Some(crate::publication::VerifiedFile::new(
                previous.file.try_clone()?,
                previous.file.metadata()?.len(),
            )?);
        }
        if [
            super::MODEL_DIR,
            super::BUNDLED_MARKER,
            super::BUNDLED_INSTALLING,
        ]
        .iter()
        .try_fold(false, |v, n| {
            lease.parent.try_exists(n).map(|exists| v || exists)
        })? && !explicit {
            return Err(io::Error::other("An older voice installation was preserved. Choose Install or Repair in Settings to continue."));
        }
        recover(&lease.parent, None, cancel)?;
        Ok(lease)
    }
}

fn move_record(parent: &Dir, source: &str, target: &str, journal: &Journal) -> Result<()> {
    let held = crate::publication::VerifiedFile::new(
        journal.file.try_clone()?,
        journal.file.metadata()?.len(),
    )?;
    crate::publication::publish_verified_noreplace_in(
        parent,
        source.as_ref(),
        target.as_ref(),
        &held,
    )?;
    sync(parent)
}

fn preserve_record(parent: &Dir, name: &str, journal: &Journal) -> Result<()> {
    mark(parent, EARLIER)?;
    move_record(
        parent,
        name,
        &format!(".voice-preserved-{}.jsonl", uuid::Uuid::new_v4()),
        journal,
    )
}

fn authenticate_owned_member(
    dir: &Dir,
    name: &Path,
    bytes: u64,
    digest: &str,
    identity: Id,
    cancel: &CancellationToken,
) -> Result<()> {
    let checked = authenticated_file(dir, name, bytes, digest, Some(cancel))?;
    if id(&Metadata::from_file(checked.file())?) != identity || !checked.matches_in(dir, name)? {
        return Err(changed());
    }
    Ok(())
}

fn verify_complete_files(dir: &Dir, journal: &Journal, cancel: &CancellationToken) -> Result<()> {
    // Every published payload must still resolve through its expected name.
    // Partial cleanup alone accepts missing members or removal capture names.
    for (path, info) in layout() {
        let Some((size, digest)) = info else { continue };
        check_cancel(cancel)?;
        let member = journal.members.get(path).ok_or_else(changed)?;
        let parent = subdir(&journal.members, dir, path.parent().ok_or_else(changed)?)?;
        authenticate_owned_member(
            &parent,
            Path::new(path.file_name().ok_or_else(changed)?),
            size,
            digest,
            member.identity,
            cancel,
        )?;
    }
    Ok(())
}

fn verified_generation(parent: &Dir, journal: &Journal, cancel: &CancellationToken) -> Result<()> {
    let dir = journal.header.open(parent)?;
    if journal.members.len() != layout().count() - 1 {
        return Err(changed());
    }
    validate(&journal.members, &dir, true)?;
    verify_complete_files(&dir, journal, cancel)?;
    validate(&journal.members, &dir, true)
}

// A storage or permission failure cannot establish that the user's selection
// is damaged. Leave both records where they are until verification can run.
fn verification_outcome(result: Result<()>) -> Result<bool> {
    match result {
        Ok(()) => Ok(true),
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::InvalidData
                    | io::ErrorKind::NotFound
                    | io::ErrorKind::NotADirectory
                    | io::ErrorKind::IsADirectory
            ) =>
        {
            Ok(false)
        }
        Err(e) => Err(e),
    }
}

fn recover_previous(parent: &Dir, cancel: &CancellationToken) -> Result<()> {
    check_cancel(cancel)?;
    let previous = match Journal::load(parent, PREVIOUS) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        r => r?,
    };
    let current = match Journal::load(parent, INSTALLED) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return move_record(parent, PREVIOUS, INSTALLED, &previous)
        }
        r => r?,
    };
    if current.header.directory_id == previous.header.directory_id {
        return Err(changed());
    }
    if !verification_outcome(verified_generation(parent, &current, cancel))? {
        let previous_valid = verification_outcome(verified_generation(parent, &previous, cancel))?;
        check_cancel(cancel)?;
        if previous_valid {
            preserve_record(parent, INSTALLED, &current)?;
            return move_record(parent, PREVIOUS, INSTALLED, &previous);
        }
        return preserve_record(parent, PREVIOUS, &previous);
    }
    // A complete authenticated selection exists. Resume reclamation of the
    // previous generation only when its remaining members can be verified.
    let remaining = previous.header.open(parent).and_then(|dir| {
        validate(&previous.members, &dir, false)?;
        verify_remaining(&previous, &dir, None, cancel)
    });
    check_cancel(cancel)?;
    if remaining
        .as_ref()
        .is_err_and(|e| e.kind() == io::ErrorKind::NotFound)
        && !parent.try_exists(&previous.header.directory)?
    {
        return remove_record(parent, PREVIOUS, previous);
    }
    if verification_outcome(remaining)? {
        reclaim_previous(parent, &current, previous)
    } else {
        preserve_record(parent, PREVIOUS, &previous)
    }
}

fn reclaim_previous(parent: &Dir, current: &Journal, previous: Journal) -> Result<()> {
    // Both content verifications can take seconds. Rebind the selected record
    // and its directory immediately before reclaiming the fallback's payload.
    require_record(parent, INSTALLED, current)?;
    current.header.open(parent)?;
    cleanup(parent, PREVIOUS, previous)
}

fn selection(parent: &Dir) -> Result<File> {
    match file(parent, INSTALLED.as_ref(), false) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => file(parent, PREVIOUS.as_ref(), false),
        r => r,
    }
}

pub(super) fn bundled_record(path: &Path) -> bool {
    let get = || -> Result<bool> {
        let parent = open_root(path, false)?;
        let handle = selection(&parent)?;
        let header: Header = read_line(&mut BufReader::new(&handle))?.ok_or_else(changed)?;
        if id(&Metadata::from_file(&handle)?) != header.record_id {
            return Err(changed());
        }
        header.open(&parent)?;
        Ok(header.bundled)
    };
    get().unwrap_or(false)
}

fn existing_marker(dir: &Dir, name: &OsStr) -> Result<File> {
    let checked = file(dir, name, false)?;
    // Windows flushes only through a handle with write access, and `file`
    // opens an existing entry read-only. Rebind the checked marker by identity
    // while it is still open; append access cannot alter bytes already present.
    #[cfg(windows)]
    let checked = {
        use cap_std::fs::OpenOptionsExt;
        let mut options = OpenOptions::new();
        options
            .read(true)
            .append(true)
            .follow(FollowSymlinks::No)
            .custom_flags(0x0020_0000)
            .share_mode(1 | 2 | 4);
        let flushable = dir.open_with(name, &options)?.into_std();
        if id(&Metadata::from_file(&flushable)?) != id(&Metadata::from_file(&checked)?) {
            return Err(changed());
        }
        flushable
    };
    Ok(checked)
}

fn mark(dir: &Dir, marker_name: &str) -> Result<()> {
    let name = marker_name.as_ref();
    let marker = match file(dir, name, true) {
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => existing_marker(dir, name)?,
        result => result?,
    };
    if marker.metadata()?.len() != 0 {
        return Err(changed());
    }
    marker.sync_all()?;
    sync(dir)
}

pub(super) fn preserved_legacy(path: &Path) -> (bool, bool) {
    let Ok(parent) = open_root(path, false) else {
        return (false, false);
    };
    let model = parent.try_exists(super::MODEL_DIR).unwrap_or(false)
        && [DETACHED, INSTALLED, PREVIOUS]
            .iter()
            .any(|name| parent.try_exists(name).unwrap_or(false));
    let model = model
        || parent.try_exists(PREVIOUS).unwrap_or(false)
        || parent.try_exists(EARLIER).unwrap_or(false);
    let audio = checked_dir(&parent, "cache")
        .and_then(|dir| Ok(dir.entries()?.next().is_some()))
        .unwrap_or_else(|e| e.kind() != io::ErrorKind::NotFound)
        || parent
            .try_exists("voice-audio-preserved-v1")
            .unwrap_or(false);
    (model, audio)
}

pub(super) fn removal_pending(path: &Path) -> bool {
    open_root(path, false)
        .and_then(|parent| parent.try_exists(REMOVING))
        .unwrap_or(false)
}

pub(super) fn location(path: &Path) -> Result<PathBuf> {
    let parent = open_root(path, false)?;
    if parent.try_exists(REMOVING)? {
        return Err(io::Error::other(
            "The previous voice removal did not finish.",
        ));
    }
    let handle = match selection(&parent) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            if parent.try_exists(DETACHED)? {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "The previous natural voice installation was detached.",
                ));
            }
            let dir = checked_dir(&parent, super::MODEL_DIR)?;
            check_required(&dir)?;
            return Ok(path.join(super::MODEL_DIR));
        }
        result => result?,
    };
    let header: Header = read_line(&mut BufReader::new(&handle))?.ok_or_else(changed)?;
    if id(&Metadata::from_file(&handle)?) != header.record_id
        || header.directory == super::MODEL_DIR
    {
        return Err(changed());
    }
    let dir = header.open(&parent)?;
    check_required(&dir)?;
    Ok(path.join(header.directory))
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct SourceStamp(Id, crate::integrity::ContentIdentity);

pub(super) fn source_stamp(path: &Path) -> Result<SourceStamp> {
    let location = location(path)?;
    let dir = checked_dir(
        &open_root(path, false)?,
        location.file_name().ok_or_else(changed)?,
    )?;
    let inputs = ["model.fp16.onnx", "tokens.txt", "voices.bin"]
        .map(|name| file(&dir, name.as_ref(), false));
    let inputs = inputs.into_iter().collect::<Result<Vec<_>>>()?;
    let identity =
        crate::integrity::ContentIdentity::from_handles(inputs.iter()).map_err(io::Error::other)?;
    Ok(SourceStamp(id(&dir.dir_metadata()?), identity))
}

fn check_required(dir: &Dir) -> Result<()> {
    for name in REQUIRED {
        if !regular(&dir.symlink_metadata(name)?) {
            return Err(changed());
        }
    }
    checked_dir(dir, "espeak-ng-data")?;
    Ok(())
}

pub(super) fn delete_model(path: &Path) -> Result<()> {
    let lease = match Lease::open(path, false) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        result => result?,
    };
    let parent = &lease.parent;
    match Journal::load(parent, REMOVING) {
        Ok(journal) => {
            persist_removal_choice(parent, &journal)?;
            return match removable_generation(parent, &journal) {
                Ok(()) => cleanup(parent, REMOVING, journal),
                Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                    preserve_record(parent, REMOVING, &journal)
                }
                Err(e) => Err(e),
            };
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    recover_previous(parent, &CancellationToken::new())?;
    let journal = match Journal::load(parent, INSTALLED) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            if parent.try_exists(super::BUNDLED_MARKER)?
                || parent.try_exists(super::BUNDLED_INSTALLING)?
            {
                mark(parent, super::BUNDLED_REMOVED)?;
            }
            if parent.try_exists(super::MODEL_DIR)? {
                mark(parent, DETACHED)?;
            }
            recover(parent, None, &CancellationToken::new())?;
            return Ok(());
        }
        result => result?,
    };
    match removable_generation(parent, &journal) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::InvalidData => {
            persist_removal_choice(parent, &journal)?;
            return preserve_record(parent, INSTALLED, &journal);
        }
        Err(e) => return Err(e),
    }
    let verified = crate::publication::VerifiedFile::new(
        journal.file.try_clone()?,
        journal.file.metadata()?.len(),
    )?;
    crate::publication::publish_verified_noreplace_in(
        parent,
        INSTALLED.as_ref(),
        REMOVING.as_ref(),
        &verified,
    )?;
    sync(parent)?;
    persist_removal_choice(parent, &journal)?;
    cleanup(parent, REMOVING, journal)
}

fn removable_generation(parent: &Dir, journal: &Journal) -> Result<()> {
    let dir = match journal.header.open(parent) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        result => result?,
    };
    validate(&journal.members, &dir, false)?;
    verify_remaining(journal, &dir, None, &CancellationToken::new())
}

fn persist_removal_choice(parent: &Dir, journal: &Journal) -> Result<()> {
    if journal.header.bundled {
        mark(parent, super::BUNDLED_REMOVED)?;
    }
    if parent.try_exists(super::MODEL_DIR)? {
        mark(parent, DETACHED)?;
    }
    Ok(())
}

pub(super) fn extract(lease: Lease, archive: File, control: &super::DownloadControl) -> Result<()> {
    let reader = bzip2_turbo_rs::io::Decoder::new(BufReader::with_capacity(64 * 1024, archive));
    install(lease, reader, false, control)
}

fn install(
    lease: Lease,
    reader: impl Read,
    bundled: bool,
    control: &super::DownloadControl,
) -> Result<()> {
    lease.unchanged()?;
    let parent = &lease.parent;
    if bundled && !control.explicit_bundled && parent.try_exists(super::BUNDLED_REMOVED)? {
        return Err(io::Error::other(
            "The bundled natural voice model was previously removed.",
        ));
    }
    let path = &lease.root;
    let cancel = &control.cancel;
    let (mut journal, directory) = Journal::create(parent, bundled)?;
    let result =
        extract_members(reader, bundled, &directory, &mut journal, cancel).and_then(|()| {
            check_cancel(cancel)?;
            recover(parent, Some((&directory, &journal)), cancel)?;
            publish_install(&lease, &journal, control, source_stamp)
        });
    drop(directory);
    if let Err(e) = result {
        let pending = journal.pending_name();
        let saved = crate::publication::VerifiedFile::new(
            journal.file.try_clone()?,
            journal.file.metadata()?.len(),
        )?;
        if saved.matches_in(parent, pending.as_ref())? {
            discard_draft(parent, &pending, journal, cancel).map_err(|cleanup| {
                io::Error::other(format!(
                    "{e}. Voice staging was preserved at {}: {cleanup}",
                    path.display()
                ))
            })?;
        }
        return Err(e);
    }
    if lease.previous.is_some() {
        // The new selection is durable. A failed reclamation leaves the
        // previous record available for retry at the next exclusive operation.
        let _ = recover_previous(parent, cancel);
    }
    Ok(())
}

fn discard_draft(
    parent: &Dir,
    pending: &str,
    journal: Journal,
    cancel: &CancellationToken,
) -> Result<()> {
    let dir = journal.header.open(parent)?;
    validate(&journal.members, &dir, false)?;
    // Quickly discard empty staging even after cancellation. Leave large
    // interrupted writes for an authenticated retry when the worker is retired.
    if journal.members.values().any(|m| m.bytes.is_some()) {
        verify_remaining(&journal, &dir, None, cancel)?;
    }
    drop(dir);
    cleanup(parent, pending, journal)
}

fn publish_install(
    lease: &Lease,
    journal: &Journal,
    control: &super::DownloadControl,
    read_selection: impl FnOnce(&Path) -> Result<SourceStamp>,
) -> Result<()> {
    let parent = &lease.parent;
    check_cancel(&control.cancel)?;
    validate(&journal.members, &journal.header.open(parent)?, true)?;
    let verified = crate::publication::VerifiedFile::new(
        journal.file.try_clone()?,
        journal.file.metadata()?.len(),
    )?;
    let mut published = control.publishing.lock().map_err(|_| changed())?;
    check_cancel(&control.cancel)?;
    lease.unchanged()?;
    if let Some(previous) = &lease.previous {
        crate::publication::publish_verified_noreplace_in(
            parent,
            INSTALLED.as_ref(),
            PREVIOUS.as_ref(),
            previous,
        )?;
        sync(parent)?;
    }
    let commit = crate::publication::publish_verified_noreplace_in(
        parent,
        journal.pending_name().as_ref(),
        INSTALLED.as_ref(),
        &verified,
    )
    .and_then(|()| {
        sync(parent)?;
        journal.header.open(parent)?;
        lease.unchanged()?;
        // A selected journal overrides any earlier removal or legacy fallback.
        // Keep fallback choices intact so rollback and death cannot lose them.
        let selected = read_selection(&lease.root)?;
        if selected.0 != journal.header.directory_id {
            return Err(changed());
        }
        *control.selected.lock().map_err(|_| changed())? = Some(selected);
        Ok(())
    });
    if let Err(error) = commit {
        if verified.matches_in(parent, INSTALLED.as_ref())? {
            crate::publication::publish_verified_noreplace_in(parent, INSTALLED.as_ref(), journal.pending_name().as_ref(), &verified)
                .and_then(|()| sync(parent))
                .map_err(|rollback| io::Error::other(format!("{error}. The installed voice could not be restored. Keep the files in {} and retry removal: {rollback}", lease.root.display())))?;
        }
        if lease.previous.is_some() {
            recover_previous(parent, &CancellationToken::new())?;
        }
        return Err(error);
    }
    *published = true;
    Ok(())
}

#[cfg(any(target_os = "android", test))]
pub(super) fn bundled(
    lease: Lease,
    reader: impl Read,
    control: &super::DownloadControl,
) -> Result<()> {
    install(lease, reader, true, control)
}

#[cfg(test)]
pub(super) fn partial_fixture(root: &Path, prefix: &[u8]) -> PathBuf {
    let lease = Lease::admit(root).unwrap();
    let (mut journal, dir) = Journal::create(&lease.parent, false).unwrap();
    let mut member = file(&dir, "tokens.txt".as_ref(), true).unwrap();
    journal
        .add(
            "tokens.txt".as_ref(),
            &Metadata::from_file(&member).unwrap(),
            Some(687),
        )
        .unwrap();
    member.write_all(prefix).unwrap();
    root.join(journal.header.directory)
}

// This application accepts one immutable distributor release. Use its known member
// order, names and lengths instead of parsing any path, type or size from TAR.
// The expanded hash includes every raw header and padding byte.
fn layout() -> impl Iterator<Item = (&'static Path, Option<(u64, &'static str)>)> {
    include_str!("pack-v1.tsv").lines().map(|row| {
        let (bytes, rest) = row.split_once('\t').expect("pack layout size");
        let (digest, path) = rest.split_once('\t').expect("pack layout path");
        (
            Path::new(path),
            (digest != "-").then(|| (bytes.parse().expect("pack layout length"), digest)),
        )
    })
}

struct Stream<'a, R> {
    reader: R,
    cancel: &'a CancellationToken,
    hash: Option<chksum_hash_sha2_256::Update>,
    remaining: u64,
}

impl<R: Read> Stream<'_, R> {
    fn read_exact(&mut self, bytes: &mut [u8]) -> Result<()> {
        check_cancel(self.cancel)?;
        self.remaining = self
            .remaining
            .checked_sub(bytes.len() as u64)
            .ok_or_else(changed)?;
        self.reader.read_exact(bytes)?;
        if let Some(hash) = &mut self.hash {
            hash.update(bytes);
        }
        Ok(())
    }

    fn padding(&mut self, count: usize) -> Result<()> {
        let mut block = [0; 1024];
        let bytes = block.get_mut(..count).ok_or_else(changed)?;
        self.read_exact(bytes)?;
        if bytes.iter().any(|&b| b != 0) {
            return Err(changed());
        }
        Ok(())
    }

    fn finish(mut self) -> Result<()> {
        if self.remaining != 0
            || self
                .hash
                .is_some_and(|h| h.digest().to_hex_lowercase() != TAR_SHA)
        {
            return Err(changed());
        }
        check_cancel(self.cancel)?;
        // Read through the final bzip block checksum and reject extra members/data.
        if self.reader.read(&mut [0])? != 0 {
            return Err(changed());
        }
        Ok(())
    }
}

fn extract_members(
    reader: impl Read,
    bundled: bool,
    root: &Dir,
    journal: &mut Journal,
    cancel: &CancellationToken,
) -> Result<()> {
    let mut stream = Stream {
        reader,
        cancel,
        hash: (!bundled).then(chksum_hash_sha2_256::new),
        remaining: if bundled {
            layout()
                .filter_map(|(_, info)| info.map(|(size, _)| size))
                .sum()
        } else {
            TAR_BYTES
        },
    };
    let mut block = [0; 512];
    let mut buffer = vec![0; 64 * 1024];
    for (path, info) in layout() {
        if !bundled {
            stream.read_exact(&mut block)?;
        }
        if path == Path::new(".") {
            continue;
        }
        if !relative(path) {
            return Err(changed());
        }
        let dir = subdir(&journal.members, root, path.parent().ok_or_else(changed)?)?;
        let name = path.file_name().ok_or_else(changed)?;
        if let Some((size, digest)) = info {
            journal.writing = Some(WrittenMember {
                path: path.to_owned(),
                bytes: 0,
                hash: chksum_hash_sha2_256::new(),
            });
            let mut out = file(&dir, name, true)?;
            journal.add(path, &Metadata::from_file(&out)?, Some(size))?;
            let written = journal.writing.as_mut().ok_or_else(changed)?;
            let mut remaining = size;
            while remaining != 0 {
                let n = remaining.min(buffer.len() as u64) as usize;
                stream.read_exact(&mut buffer[..n])?;
                out.write_all(&buffer[..n])?;
                written.hash.update(&buffer[..n]);
                written.bytes += n as u64;
                remaining -= n as u64;
            }
            if written.hash.digest().to_hex_lowercase() != digest {
                return Err(changed());
            }
            if !bundled {
                stream.padding(((512 - size % 512) % 512) as usize)?;
            }
            out.sync_all()?;
            journal.writing = None;
        } else {
            let child = new_directory(&dir, name)?;
            journal.add(path, &child.dir_metadata()?, None)?;
        }
        sync(&dir)?;
    }
    if !bundled {
        stream.padding(1024)?;
    }
    stream.finish()?;
    sync(root)
}

/// Check and retain the three files referenced by the model configuration.
/// Sherpa opens their names separately. Pre/post validation detects persistent
/// replacements; same-account transient substitution in a native open remains possible.
pub(super) struct EngineFiles {
    pub(super) dir: PathBuf,
    root: PathBuf,
    parent: Dir,
    directory: Dir,
    inputs: Vec<(&'static Path, crate::publication::VerifiedFile)>,
    stamp: crate::integrity::ContentIdentity,
}

impl EngineFiles {
    pub(super) fn unchanged(&self) -> Result<()> {
        if id(&open_root(&self.root, false)?.dir_metadata()?) != id(&self.parent.dir_metadata()?) {
            return Err(changed());
        }
        let name = self.dir.file_name().ok_or_else(changed)?;
        if id(&checked_dir(&self.parent, name)?.dir_metadata()?)
            != id(&self.directory.dir_metadata()?)
        {
            return Err(changed());
        }
        let stamp = crate::integrity::ContentIdentity::from_handles(
            self.inputs.iter().map(|(_, input)| input.file()),
        )
        .map_err(io::Error::other)?;
        if stamp != self.stamp {
            return Err(changed());
        }
        for (name, input) in &self.inputs {
            if !input.matches_in(&self.directory, name)? {
                return Err(changed());
            }
        }
        Ok(())
    }
}

fn authenticated_file(
    dir: &Dir,
    name: &Path,
    size: u64,
    digest: &str,
    cancel: Option<&CancellationToken>,
) -> Result<crate::publication::VerifiedFile> {
    let mut source = file(dir, name.as_os_str(), false)?;
    let before = crate::integrity::ContentIdentity::from_handles(std::iter::once(&source))
        .map_err(io::Error::other)?;
    let mut hash = chksum_hash_sha2_256::new();
    if source.metadata()?.len() != size {
        return Err(changed());
    }
    let mut buffer = vec![0; 128 * 1024];
    let mut remaining = size;
    while remaining != 0 {
        if let Some(token) = cancel {
            check_cancel(token)?;
        }
        let n = remaining.min(buffer.len() as u64) as usize;
        source.read_exact(&mut buffer[..n])?;
        hash.update(&buffer[..n]);
        remaining -= n as u64;
    }
    if source.read(&mut [0])? != 0 || hash.digest().to_hex_lowercase() != digest {
        return Err(changed());
    }
    if before
        != crate::integrity::ContentIdentity::from_handles(std::iter::once(&source))
            .map_err(io::Error::other)?
    {
        return Err(changed());
    }
    crate::publication::VerifiedFile::new(source, size)
}

pub(super) fn engine_files(path: &Path, cancel: Option<&CancellationToken>) -> Result<EngineFiles> {
    let dir = location(path)?;
    if dir.to_str().is_none() {
        return Err(io::Error::other(
            "The speech engine requires a Unicode model path.",
        ));
    }
    let parent = open_root(path, false)?;
    let directory = checked_dir(&parent, dir.file_name().ok_or_else(changed)?)?;
    let mut inputs = Vec::new();
    for (name, info) in layout().filter(|(p, _)| {
        ["model.fp16.onnx", "voices.bin", "tokens.txt"].contains(&p.to_str().unwrap_or(""))
    }) {
        let (size, digest) = info.ok_or_else(changed)?;
        let source = authenticated_file(&directory, name, size, digest, cancel)?;
        inputs.push((name, source));
    }
    if inputs.len() != 3 {
        return Err(changed());
    }
    let stamp = crate::integrity::ContentIdentity::from_handles(
        inputs.iter().map(|(_, input)| input.file()),
    )
    .map_err(io::Error::other)?;
    let result = EngineFiles {
        stamp,
        dir,
        root: path.to_owned(),
        parent,
        directory,
        inputs,
    };
    result.unchanged()?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use super::super::fixtures;

    #[test]
    fn layout_is_bounded_unique_and_contains_only_preceding_parent_directories() {
        let mut sizes = BTreeMap::new();
        let mut tar_size = 1024;
        let mut dirs = 0;
        let mut files = 0;
        for (path, info) in layout() {
            if path != Path::new(".") {
                assert!(relative(path));
                let parent = path.parent().unwrap();
                if !parent.as_os_str().is_empty() {
                    assert_eq!(sizes.get(parent), Some(&None));
                }
            }
            assert!(sizes.insert(path, info).is_none());
            tar_size += 512;
            if let Some((size, digest)) = info {
                assert_eq!(digest.len(), 64);
                assert!(digest.bytes().all(|b| b.is_ascii_hexdigit()));
                files += 1;
                tar_size += size.div_ceil(512) * 512;
            } else {
                dirs += 1;
            }
        }
        assert_eq!((files, dirs, tar_size), (360, 38, TAR_BYTES));
    }

    #[cfg(unix)]
    #[test]
    fn substituted_model_parent_never_opens_the_target_cache() {
        use std::os::unix::fs::symlink;
        let profile = fixtures::profile().unwrap();
        let base = profile.path().canonicalize().unwrap();
        let target = base.join("unrelated");
        fs::create_dir(&target).unwrap();
        symlink(&target, base.join("models")).unwrap();
        let name = format!("{}.wav", "b".repeat(64));
        assert!(cached_audio(&base.join("models/cache"), name.as_ref()).is_err());
        assert!(!target.join("cache").exists());
    }

    #[test]
    fn concurrent_installer_and_mutated_staging_preserve_unrelated_files() {
        let profile = fixtures::profile().unwrap();
        let root = profile.path().canonicalize().unwrap().join("models");
        let lease = Lease::admit(&root).unwrap();
        assert!(Lease::admit(&root).is_err());
        let (mut journal, directory) = Journal::create(&lease.parent, false).unwrap();
        let generation = root.join(&journal.header.directory);
        let mut source = file(&directory, "tokens.txt".as_ref(), true).unwrap();
        journal
            .add(
                "tokens.txt".as_ref(),
                &Metadata::from_file(&source).unwrap(),
                Some(687),
            )
            .unwrap();
        source.write_all(b"partial").unwrap();
        drop(source);
        drop(directory);
        drop(journal);
        drop(lease);
        fs::rename(generation.join("tokens.txt"), generation.join("original")).unwrap();
        fs::write(generation.join("tokens.txt"), b"other").unwrap();
        let retry = Lease::admit(&root).unwrap();
        assert_eq!(fs::read(generation.join("tokens.txt")).unwrap(), b"other");
        assert_eq!(fs::read(generation.join("original")).unwrap(), b"partial");
        assert!(retry.parent.dir_metadata().is_ok());
        assert!(location(&root).is_err());
    }

    #[test]
    fn an_unverified_partial_is_preserved_until_an_authenticated_retry() {
        let profile = fixtures::profile().unwrap();
        let root = profile.path().canonicalize().unwrap().join("models");
        let lease = Lease::admit(&root).unwrap();
        let (mut journal, dir) = Journal::create(&lease.parent, false).unwrap();
        let generation = root.join(&journal.header.directory);
        let mut member = file(&dir, "tokens.txt".as_ref(), true).unwrap();
        journal
            .add(
                "tokens.txt".as_ref(),
                &Metadata::from_file(&member).unwrap(),
                Some(687),
            )
            .unwrap();
        member.write_all(b"partial").unwrap();
        drop((member, dir, journal, lease));
        fs::write(root.join(".voice-pack-unrelated"), b"keep").unwrap();
        let retry = Lease::admit(&root).unwrap();
        assert_eq!(fs::read(generation.join("tokens.txt")).unwrap(), b"partial");
        assert!(retry.parent.dir_metadata().is_ok());
        assert_eq!(
            fs::read(root.join(".voice-pack-unrelated")).unwrap(),
            b"keep"
        );
    }

    #[test]
    fn truncated_bundled_input_leaves_no_published_or_partial_model() {
        let profile = fixtures::profile().unwrap();
        let root = profile.path().canonicalize().unwrap().join("models");
        let lease = Lease::admit(&root).unwrap();
        let error = bundled(
            lease,
            io::Cursor::new(vec![1; 7]),
            &super::super::DownloadControl::new(),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        assert!(!root.join(INSTALLED).exists());
        let names = root
            .read_dir()
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(names, [OsStr::new(LOCK)]);
    }

    #[test]
    fn torn_headers_and_rows_do_not_authorize_cleanup_or_block_a_new_generation() {
        let profile = fixtures::profile().unwrap();
        let root = profile.path().canonicalize().unwrap().join("models");
        let lease = Lease::admit(&root).unwrap();
        let torn = root.join(format!("{PENDING}unrelated.jsonl"));
        fixtures::write_private(&torn, b"{").unwrap();
        let (mut journal, dir) = Journal::create(&lease.parent, false).unwrap();
        let pending = journal.pending_name();
        journal.file.write_all(b"{broken row").unwrap();
        fs::write(root.join(&journal.header.directory).join("other"), b"keep").unwrap();
        drop((dir, journal, lease));
        let _retry = Lease::admit(&root).unwrap();
        assert_eq!(fs::read(torn).unwrap(), b"{");
        assert!(root.join(pending).is_file());
    }

    #[test]
    fn captured_owned_file_can_resume_and_preserves_a_reused_original_name() {
        let profile = fixtures::profile().unwrap();
        let root = profile.path().canonicalize().unwrap().join("models");
        let lease = Lease::admit(&root).unwrap();
        let original = file(&lease.parent, "entry".as_ref(), true).unwrap();
        let identity = id(&Metadata::from_file(&original).unwrap());
        drop(original);
        let capture = capture_name(identity);
        fs::rename(root.join("entry"), root.join(&capture)).unwrap();
        fs::write(root.join("entry"), b"other").unwrap();
        remove_owned_file(&lease.parent, "entry".as_ref(), identity).unwrap();
        assert_eq!(fs::read(root.join("entry")).unwrap(), b"other");
        assert!(!root.join(capture).exists());
    }

    #[test]
    fn absent_staging_directory_and_captured_journal_resume_after_interruption() {
        let profile = fixtures::profile().unwrap();
        let root = profile.path().canonicalize().unwrap().join("models");
        for captured in [false, true] {
            let lease = Lease::admit(&root).unwrap();
            let (journal, dir) = Journal::create(&lease.parent, false).unwrap();
            let mut name = journal.pending_name();
            let tombstone = capture_name(journal.header.record_id);
            dir.remove_open_dir().unwrap();
            if captured {
                fs::rename(root.join(&name), root.join(&tombstone)).unwrap();
                name = tombstone;
            }
            drop((journal, lease));
            fs::write(root.join("keep"), b"unrelated").unwrap();
            let _lease = Lease::admit(&root).unwrap();
            assert!(!root.join(name).exists());
            assert_eq!(fs::read(root.join("keep")).unwrap(), b"unrelated");
        }
    }

    #[test]
    fn disk_full_during_record_preserves_unverifiable_members() {
        struct Limited<'a>(&'a mut File, usize);
        impl Write for Limited<'_> {
            fn write(&mut self, bytes: &[u8]) -> Result<usize> {
                if self.1 == 0 {
                    return Err(io::Error::from(io::ErrorKind::StorageFull));
                }
                let n = self.0.write(&bytes[..bytes.len().min(self.1)])?;
                self.1 -= n;
                Ok(n)
            }
            fn flush(&mut self) -> Result<()> {
                self.0.flush()
            }
        }
        let profile = fixtures::profile().unwrap();
        let root = profile.path().canonicalize().unwrap().join("models");
        let lease = Lease::admit(&root).unwrap();
        let (mut journal, dir) = Journal::create(&lease.parent, false).unwrap();
        let occupied = root.join(&journal.header.directory).join("tokens.txt");
        fs::write(&occupied, b"incomplete").unwrap();
        let file = dir.open("tokens.txt").unwrap();
        let member = Member {
            path: "tokens.txt".into(),
            identity: id(&file.metadata().unwrap()),
            bytes: Some(687),
        };
        let before = journal.file.metadata().unwrap().len();
        assert!(encode_record(Limited(&mut journal.file, 7), &member).is_err());
        assert_eq!(journal.file.metadata().unwrap().len(), before + 7);
        drop((dir, journal, lease));
        let _retry = Lease::admit(&root).unwrap();
        assert_eq!(fs::read(occupied).unwrap(), b"incomplete");
    }

    #[test]
    fn legacy_removal_detaches_without_deleting_existing_contents() {
        let profile = fixtures::profile().unwrap();
        let root = profile.path().canonicalize().unwrap().join("models");
        let lease = Lease::admit(&root).unwrap();
        drop(lease);
        let old = root.join(super::super::MODEL_DIR);
        fs::create_dir(&old).unwrap();
        fs::write(old.join("tokens.txt"), b"existing").unwrap();
        delete_model(&root).unwrap();
        assert!(preserved_legacy(&root).0);
        assert_eq!(fs::read(old.join("tokens.txt")).unwrap(), b"existing");
        let lease = Lease::for_install(&root, true, &CancellationToken::new()).unwrap();
        assert!(bundled(lease, io::empty(), &super::super::DownloadControl::new()).is_err());
        assert_eq!(fs::read(old.join("tokens.txt")).unwrap(), b"existing");
    }

    #[test]
    fn failed_final_selection_keeps_the_old_fallback_and_factory_removal_choice() {
        let profile = fixtures::profile().unwrap();
        let root = profile.path().canonicalize().unwrap().join("models");
        let lease = Lease::admit(&root).unwrap();
        let old = new_directory(&lease.parent, super::super::MODEL_DIR.as_ref()).unwrap();
        mark(&lease.parent, super::super::BUNDLED_REMOVED).unwrap();
        let original = id(&lease
            .parent
            .symlink_metadata(super::super::BUNDLED_REMOVED)
            .unwrap());
        let (mut journal, dir) = Journal::create(&lease.parent, true).unwrap();
        for name in REQUIRED {
            drop(file(&old, name.as_ref(), true).unwrap());
            let out = file(&dir, name.as_ref(), true).unwrap();
            journal
                .add(name.as_ref(), &Metadata::from_file(&out).unwrap(), Some(0))
                .unwrap();
        }
        new_directory(&old, "espeak-ng-data".as_ref()).unwrap();
        let data = new_directory(&dir, "espeak-ng-data".as_ref()).unwrap();
        journal
            .add(
                "espeak-ng-data".as_ref(),
                &data.dir_metadata().unwrap(),
                None,
            )
            .unwrap();
        let control = super::super::DownloadControl {
            explicit_bundled: true,
            ..super::super::DownloadControl::new()
        };
        let failure = publish_install(&lease, &journal, &control, |_| {
            Err(io::Error::other("selection unavailable"))
        })
        .unwrap_err();
        assert_eq!(failure.to_string(), "selection unavailable");
        assert!(root.join(journal.pending_name()).is_file());
        assert_eq!(location(&root).unwrap(), root.join(super::super::MODEL_DIR));
        assert_eq!(
            id(&lease
                .parent
                .symlink_metadata(super::super::BUNDLED_REMOVED)
                .unwrap()),
            original
        );
        assert!(!root.join(DETACHED).exists());
    }

    #[test]
    fn failed_install_preserves_same_inode_edits_and_removes_unchanged_live_writes() {
        let profile = fixtures::profile().unwrap();
        let root = profile.path().canonicalize().unwrap().join("models");
        let lease = Lease::admit(&root).unwrap();
        for changed in [true, false] {
            let (mut journal, dir) = Journal::create(&lease.parent, false).unwrap();
            let mut file = file(&dir, "tokens.txt".as_ref(), true).unwrap();
            journal
                .add(
                    "tokens.txt".as_ref(),
                    &Metadata::from_file(&file).unwrap(),
                    Some(687),
                )
                .unwrap();
            file.write_all(b"draft").unwrap();
            let mut hash = chksum_hash_sha2_256::new();
            hash.update(b"draft");
            journal.writing = Some(WrittenMember {
                path: "tokens.txt".into(),
                bytes: 5,
                hash,
            });
            let path = root.join(&journal.header.directory).join("tokens.txt");
            // Close the member first: the open staging handle denies other
            // writers on Windows. The edit below still rewrites the same file.
            drop((dir, file));
            if changed {
                fs::write(&path, b"other").unwrap();
            }
            let result = discard_draft(
                &lease.parent,
                &journal.pending_name(),
                journal,
                &CancellationToken::new(),
            );
            if changed {
                assert!(result.is_err());
                assert_eq!(fs::read(&path).unwrap(), b"other");
            } else {
                assert!(result.is_ok());
                assert!(!path.exists());
            }
        }
    }

    #[test]
    fn two_records_with_missing_members_do_not_block_explicit_repair_or_delete() {
        let profile = fixtures::profile().unwrap();
        let root = profile.path().canonicalize().unwrap().join("models");
        let lease = Lease::admit(&root).unwrap();
        let (old, old_dir) = Journal::create(&lease.parent, false).unwrap();
        fs::write(root.join(&old.header.directory).join("mine"), b"previous").unwrap();
        move_record(&lease.parent, &old.pending_name(), PREVIOUS, &old).unwrap();
        let (current, dir) = Journal::create(&lease.parent, false).unwrap();
        let untouched = root.join(&current.header.directory).join("mine");
        fs::write(&untouched, b"current").unwrap();
        move_record(&lease.parent, &current.pending_name(), INSTALLED, &current).unwrap();
        drop((current, dir, old_dir, lease));
        let retry = Lease::for_install(&root, true, &CancellationToken::new()).unwrap();
        assert!(retry.previous.is_some());
        drop(retry);
        delete_model(&root).unwrap();
        assert_eq!(fs::read(untouched).unwrap(), b"current");
        assert_eq!(
            fs::read(root.join(&old.header.directory).join("mine")).unwrap(),
            b"previous"
        );
        assert!(!root.join(INSTALLED).exists());
    }

    #[test]
    fn a_repeated_marker_is_accepted_and_a_changed_one_is_refused() {
        let profile = fixtures::profile().unwrap();
        let root = profile.path().canonicalize().unwrap().join("models");
        let lease = Lease::admit(&root).unwrap();
        mark(&lease.parent, DETACHED).unwrap();
        // A marker is never deleted, so a later operation records it again.
        // Windows flushes only through a writable handle, so the second mark
        // must not fail on the marker the first one made.
        mark(&lease.parent, DETACHED).unwrap();
        assert_eq!(fs::read(root.join(DETACHED)).unwrap(), b"");
        fs::write(root.join(DETACHED), b"other").unwrap();
        assert!(mark(&lease.parent, DETACHED).is_err());
        assert_eq!(fs::read(root.join(DETACHED)).unwrap(), b"other");
    }

    #[test]
    #[ignore = "requires the verified KIWIX_TEST_KOKORO_BUNDLED factory asset"]
    fn durable_previous_is_restored_when_selected_bytes_change_without_a_size_change() {
        let fixture = std::env::var_os("KIWIX_TEST_KOKORO_BUNDLED").unwrap();
        let profile = fixtures::profile().unwrap();
        let root = profile.path().canonicalize().unwrap().join("models");
        let lease = Lease::admit(&root).unwrap();
        let cancel = CancellationToken::new();
        let (mut old, old_dir) = Journal::create(&lease.parent, true).unwrap();
        extract_members(
            File::open(&fixture).unwrap(),
            true,
            &old_dir,
            &mut old,
            &cancel,
        )
        .unwrap();
        move_record(&lease.parent, &old.pending_name(), PREVIOUS, &old).unwrap();
        let (mut new, new_dir) = Journal::create(&lease.parent, true).unwrap();
        extract_members(
            File::open(fixture).unwrap(),
            true,
            &new_dir,
            &mut new,
            &cancel,
        )
        .unwrap();
        move_record(&lease.parent, &new.pending_name(), INSTALLED, &new).unwrap();
        let changed_path = root.join(&new.header.directory).join("tokens.txt");
        let mut bytes = fs::read(&changed_path).unwrap();
        bytes[0] ^= 1;
        fs::write(&changed_path, &bytes).unwrap();
        recover_previous(&lease.parent, &cancel).unwrap();
        assert_eq!(location(&root).unwrap(), root.join(&old.header.directory));
        assert_eq!(fs::read(changed_path).unwrap(), bytes);
        assert!(!root.join(PREVIOUS).exists());
    }

    #[test]
    fn explicit_retry_detaches_a_changed_removal_without_erasing_the_payload() {
        let profile = fixtures::profile().unwrap();
        let root = profile.path().canonicalize().unwrap().join("models");
        let lease = Lease::admit(&root).unwrap();
        let (mut journal, dir) = Journal::create(&lease.parent, true).unwrap();
        let mut out = file(&dir, "tokens.txt".as_ref(), true).unwrap();
        journal
            .add(
                "tokens.txt".as_ref(),
                &Metadata::from_file(&out).unwrap(),
                Some(687),
            )
            .unwrap();
        out.write_all(&[9; 687]).unwrap();
        let target = root.join(&journal.header.directory).join("tokens.txt");
        move_record(&lease.parent, &journal.pending_name(), REMOVING, &journal).unwrap();
        drop((dir, out, journal, lease));
        delete_model(&root).unwrap();
        assert_eq!(fs::read(target).unwrap(), [9; 687]);
        assert!(!removal_pending(&root));
        assert!(root.join(super::super::BUNDLED_REMOVED).is_file());
        assert!(Lease::for_install(&root, true, &CancellationToken::new()).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn inconclusive_model_access_keeps_both_selection_records_for_retry() {
        use std::os::unix::fs::PermissionsExt;
        let profile = fixtures::profile().unwrap();
        let root = profile.path().canonicalize().unwrap().join("models");
        let lease = Lease::admit(&root).unwrap();
        let (old, _) = Journal::create(&lease.parent, false).unwrap();
        let (current, _) = Journal::create(&lease.parent, false).unwrap();
        move_record(&lease.parent, &old.pending_name(), PREVIOUS, &old).unwrap();
        move_record(&lease.parent, &current.pending_name(), INSTALLED, &current).unwrap();
        for generation in [&current.header.directory, &old.header.directory] {
            let path = root.join(generation);
            fs::set_permissions(&path, fs::Permissions::from_mode(0o770)).unwrap();
            let e = recover_previous(&lease.parent, &CancellationToken::new()).unwrap_err();
            assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
            assert_eq!(
                Journal::load(&lease.parent, PREVIOUS)
                    .unwrap()
                    .header
                    .record_id,
                old.header.record_id
            );
            assert_eq!(
                Journal::load(&lease.parent, INSTALLED)
                    .unwrap()
                    .header
                    .record_id,
                current.header.record_id
            );
            assert!(!root.join(EARLIER).exists());
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        assert!(verification_outcome(Err(io::Error::from(io::ErrorKind::Interrupted))).is_err());
    }

    #[test]
    fn complete_authentication_requires_a_member_that_partial_cleanup_can_skip() {
        let profile = fixtures::profile().unwrap();
        let root = profile.path().canonicalize().unwrap().join("models");
        let lease = Lease::admit(&root).unwrap();
        let (mut journal, dir) = Journal::create(&lease.parent, false).unwrap();
        let out = file(&dir, "tokens.txt".as_ref(), true).unwrap();
        journal
            .add(
                "tokens.txt".as_ref(),
                &Metadata::from_file(&out).unwrap(),
                Some(687),
            )
            .unwrap();
        drop(out);
        dir.remove_file("tokens.txt").unwrap();
        let cancel = CancellationToken::new();
        assert!(verify_remaining(&journal, &dir, None, &cancel).is_ok());
        assert_eq!(
            verify_complete_files(&dir, &journal, &cancel)
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
    }

    #[test]
    fn reclaim_keeps_the_fallback_if_the_selection_directory_or_pointer_changed() {
        let profile = fixtures::profile().unwrap();
        let root = profile.path().canonicalize().unwrap().join("models");
        let lease = Lease::admit(&root).unwrap();
        let (old, _) = Journal::create(&lease.parent, false).unwrap();
        let (current, _) = Journal::create(&lease.parent, false).unwrap();
        move_record(&lease.parent, &old.pending_name(), PREVIOUS, &old).unwrap();
        move_record(&lease.parent, &current.pending_name(), INSTALLED, &current).unwrap();
        let selected = root.join(&current.header.directory);
        fs::rename(&selected, root.join("saved-model")).unwrap();
        assert!(reclaim_previous(
            &lease.parent,
            &current,
            Journal::load(&lease.parent, PREVIOUS).unwrap()
        )
        .is_err());
        fs::rename(root.join("saved-model"), &selected).unwrap();
        fs::rename(root.join(INSTALLED), root.join("saved-record")).unwrap();
        fs::write(root.join(INSTALLED), b"unrelated").unwrap();
        assert!(reclaim_previous(
            &lease.parent,
            &current,
            Journal::load(&lease.parent, PREVIOUS).unwrap()
        )
        .is_err());
        assert_eq!(fs::read(root.join(INSTALLED)).unwrap(), b"unrelated");
        assert!(root.join(PREVIOUS).is_file());
        assert!(root.join(&old.header.directory).is_dir());
    }

    #[test]
    fn required_storage_includes_both_download_and_expansion() {
        let limit = 240 * 1024 * 1024;
        assert!(check_space(limit, false).is_ok());
        assert!(check_space(limit - 1, false).is_err());
        assert!(check_space(limit + super::super::MODEL_BYTES, true).is_ok());
        assert!(check_space(limit + super::super::MODEL_BYTES - 1, true).is_err());
    }

    #[cfg(all(unix, desktop))]
    #[test]
    fn desktop_profile_protects_voice_files_and_preserves_existing_shared_profiles() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let profile = base.join("fresh");
        let _store = crate::store::Store::new(profile.clone()).unwrap();
        assert_eq!(mode(&profile), 0o700);
        let models = profile.join("models");
        let lease = Lease::admit(&models).unwrap();
        assert_eq!(mode(&models), 0o700);
        assert_eq!(mode(&models.join(LOCK)), 0o600);
        assert_eq!(lease.scratch().unwrap().metadata().unwrap().permissions().mode() & 0o777, 0o600);
        let cache = models.join("voice-audio-v2");
        let name = format!("{}.wav", "a".repeat(64));
        let wav = super::super::wav_bytes(&[0.25], 24000);
        drop(publish_audio(&cache, name.as_ref(), &wav).unwrap());
        assert_eq!(mode(&cache), 0o700);
        let entries: Vec<_> = fs::read_dir(&cache).unwrap()
            .map(|entry| entry.unwrap().path()).collect();
        assert_eq!(entries.len(), 3);
        assert!(entries.contains(&cache.join(LOCK)));
        assert!(entries.contains(&cache.join(format!("{}.receipt", "a".repeat(64)))));
        assert!(entries.iter().any(|entry| entry.extension() == Some(OsStr::new("wav"))));
        assert!(entries.iter().all(|entry| mode(entry) == 0o600));
        assert!(cached_audio(&cache, name.as_ref()).unwrap().is_some());

        let shared = base.join("shared");
        fs::create_dir(&shared).unwrap();
        fs::set_permissions(&shared, fs::Permissions::from_mode(0o775)).unwrap();
        let sentinel = shared.join("existing");
        fs::write(&sentinel, b"keep").unwrap();
        let _shared_store = crate::store::Store::new(shared.clone()).unwrap();
        let error = Lease::admit(&shared.join("models")).err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(mode(&shared), 0o775);
        assert_eq!(fs::read(sentinel).unwrap(), b"keep");
        assert!(!shared.join("models").exists());
    }

    #[cfg(unix)]
    #[test]
    fn writable_namespace_and_linked_record_are_preserved() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let profile = fixtures::profile().unwrap();
        let root = profile.path().canonicalize().unwrap().join("models");
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o775)).unwrap();
        assert!(Lease::admit(&root).is_err());
        assert!(!root.join(LOCK).exists());
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let other = profile.path().join("my-file");
        fs::write(&other, b"keep").unwrap();
        let decoy = root.join(format!("{PENDING}unrelated.jsonl"));
        symlink(&other, &decoy).unwrap();
        assert!(Lease::admit(&root).is_ok());
        assert!(decoy.symlink_metadata().unwrap().is_symlink());
        assert_eq!(fs::read(other).unwrap(), b"keep");
    }
}

fn publish_bytes(dir: &Dir, name: &Path, data: &[u8]) -> Result<File> {
    let temporary = format!(".voice-data-{}.part", uuid::Uuid::new_v4());
    let mut out = file(dir, temporary.as_ref(), true)?;
    let original = id(&Metadata::from_file(&out)?);
    let result = (|| {
        out.write_all(data)?;
        out.sync_data()?;
        let verified = crate::publication::VerifiedFile::new(out.try_clone()?, data.len() as u64)?;
        crate::publication::publish_verified_noreplace_in(
            dir,
            temporary.as_ref(),
            name,
            &verified,
        )?;
        out.rewind()?;
        out.try_clone()
    })();
    drop(out);
    if dir.try_exists(&temporary)? {
        remove_owned_file(dir, temporary.as_ref(), original)?;
    }
    result
}

fn state_file(dir: &Dir, name: &str) -> Result<(File, Vec<u8>)> {
    let source = file(dir, name.as_ref(), false)?;
    let mut data = Vec::new();
    (&source).take(8193).read_to_end(&mut data)?;
    if data.len() > 8192 {
        return Err(changed());
    }
    let valid = match name {
        super::PERFORMANCE_FILE => {
            serde_json::from_slice::<super::Verdict<super::Performance>>(&data)
                .is_ok_and(|p| p.value.ratio.is_finite() && p.value.ratio > 0.0)
        }
        super::PERFORMANCE_ERROR_FILE => serde_json::from_slice::<super::Verdict<String>>(&data)
            .is_ok_and(|s| !s.value.trim().is_empty()),
        super::BUNDLED_MARKER | super::BUNDLED_INSTALLING | super::BUNDLED_REMOVED => {
            data.is_empty()
        }
        _ => false,
    };
    if !valid {
        return Err(changed());
    }
    Ok((source, data))
}

pub(super) fn read_state(path: &Path, name: &str) -> Result<Vec<u8>> {
    state_file(&open_root(path, false)?, name).map(|(_, data)| data)
}

fn clear_state_in(dir: &Dir, name: &str) -> Result<()> {
    match state_file(dir, name) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        result => {
            let (source, _) = result?;
            let identity = id(&Metadata::from_file(&source)?);
            drop(source);
            remove_owned_file(dir, name.as_ref(), identity)
        }
    }
}

pub(super) fn clear_state(path: &Path, name: &str) -> Result<()> {
    clear_state_in(&open_root(path, false)?, name)
}

pub(super) fn save_state(path: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    if bytes.len() > 8192 {
        return Err(changed());
    }
    let dir = open_root(path, false)?;
    clear_state_in(&dir, name)?;
    publish_bytes(&dir, name.as_ref(), bytes)?;
    sync(&dir)
}

#[path = "audio_cache.rs"]
mod audio_cache;
pub(super) use audio_cache::{cached_audio, clean_cache, publish_audio};
