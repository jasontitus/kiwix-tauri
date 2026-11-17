// SPDX-License-Identifier: GPL-3.0-or-later
#![deny(unsafe_code)]
//! Cross-transport reuse of partially downloaded archive bytes.
//!
//! A BitTorrent job and a LocalSwarm job can acquire the same archive. When one
//! of them is partially complete, the other may copy byte ranges out of its
//! staged file instead of fetching them from the network again. The donor is
//! only ever read. Every copied range is accepted solely by the receiving
//! transport's own integrity authority: LocalSwarm compares each chunk with its
//! manifest SHA-256 before writing it, and BitTorrent hashes every piece in its
//! initial check before counting it. Donor filenames, sizes, job metadata and
//! archive UUIDs only select candidates; they never authorize bytes.
use std::fs::File;
use std::path::{Path, PathBuf};

/// A staged file owned by another acquisition job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Donor {
    pub path: PathBuf,
    pub size: u64,
    /// Byte ranges `(offset, length)` the owning job has already verified.
    /// `None` means the owner records no range map, so every range is a
    /// candidate for the receiver's own verification.
    pub ranges: Option<Vec<(u64, u64)>>,
}

/// After this many fully written candidate chunks fail verification with none
/// accepted, the donor is treated as different content and abandoned.
pub(crate) const MISMATCH_LIMIT: usize = 64;

/// Opens a donor read-only without following links, and requires a regular,
/// singly linked file of exactly the expected length.
pub(crate) fn open(path: &Path, size: u64) -> Result<File, String> {
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
    let parent = path.parent().ok_or("Partial download has no parent directory")?;
    let name = path.file_name().ok_or("Partial download has no filename")?;
    let directory = crate::storage::open_directory_nofollow(parent).map_err(|e| e.to_string())?;
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    #[cfg(windows)]
    {
        use cap_std::fs::OpenOptionsExt;
        // FILE_FLAG_OPEN_REPARSE_POINT; tolerate the owner's concurrent writer.
        options.custom_flags(0x0020_0000).share_mode(1 | 2 | 4);
    }
    let file = directory.open_with(name, &options).map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    let single = {
        use cap_fs_ext::MetadataExt;
        metadata.nlink() == 1
    };
    if !metadata.is_file() || !single || metadata.len() != size {
        return Err("Partial download is not a matching regular file".into());
    }
    Ok(file.into_std())
}

/// Reads exactly `buffer.len()` bytes at `offset`.
pub(crate) fn read_at(file: &File, offset: u64, buffer: &mut [u8]) -> Result<(), String> {
    let mut filled = 0usize;
    while filled < buffer.len() {
        let position = offset.checked_add(filled as u64).ok_or("Partial download offset overflow")?;
        #[cfg(unix)]
        let count = std::os::unix::fs::FileExt::read_at(file, &mut buffer[filled..], position);
        #[cfg(windows)]
        let count = std::os::windows::fs::FileExt::seek_read(file, &mut buffer[filled..], position);
        match count {
            Ok(0) => return Err("Partial download ended early".into()),
            Ok(count) => filled += count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

/// True when `[offset, offset + length)` lies entirely inside `ranges`.
/// `None` admits every range.
pub(crate) fn covers(ranges: Option<&[(u64, u64)]>, offset: u64, length: u64) -> bool {
    let Some(ranges) = ranges else { return true };
    let Some(end) = offset.checked_add(length) else { return false };
    let mut cursor = offset;
    for &(start, len) in ranges {
        let Some(stop) = start.checked_add(len) else { return false };
        if start <= cursor && cursor < stop {
            cursor = stop;
            if cursor >= end { return true; }
        }
    }
    length == 0
}

/// Coalesces sorted, nonoverlapping `(offset, length)` pieces.
pub(crate) fn coalesce(pieces: impl IntoIterator<Item = (u64, u64)>) -> Vec<(u64, u64)> {
    let mut ranges: Vec<(u64, u64)> = Vec::new();
    for (offset, length) in pieces {
        if length == 0 { continue; }
        match ranges.last_mut() {
            Some((start, len)) if *start + *len == offset => *len += length,
            _ => ranges.push((offset, length)),
        }
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_requires_every_byte() {
        let ranges = coalesce([(0, 4), (4, 4), (16, 8)]);
        assert_eq!(ranges, vec![(0, 8), (16, 8)]);
        assert!(covers(Some(&ranges), 0, 8));
        assert!(covers(Some(&ranges), 2, 4));
        assert!(covers(Some(&ranges), 16, 8));
        assert!(!covers(Some(&ranges), 6, 4));
        assert!(!covers(Some(&ranges), 8, 1));
        assert!(!covers(Some(&ranges), 20, 8));
        assert!(!covers(Some(&ranges), u64::MAX, 2));
        assert!(covers(None, 123, 456));
    }

    #[test]
    fn open_rejects_wrong_length_and_links() {
        let base = std::env::temp_dir().join(format!("kiwix-partial-donor-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&base).unwrap();
        let root = std::fs::canonicalize(&base).unwrap();
        let path = root.join("a.zim");
        std::fs::write(&path, b"0123456789").unwrap();
        let file = open(&path, 10).unwrap();
        let mut buffer = [0u8; 4];
        read_at(&file, 3, &mut buffer).unwrap();
        assert_eq!(&buffer, b"3456");
        assert!(read_at(&file, 8, &mut buffer).is_err());
        assert!(open(&path, 11).is_err());
        #[cfg(unix)]
        {
            let link = root.join("b.zim");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            assert!(open(&link, 10).is_err());
            let hard = root.join("c.zim");
            std::fs::hard_link(&path, &hard).unwrap();
            assert!(open(&path, 10).is_err());
        }
        std::fs::remove_dir_all(&root).unwrap();
    }
}
