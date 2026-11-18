// SPDX-License-Identifier: GPL-3.0-or-later
//! Metadata, title enumeration and structural checks for the application reader.
#![forbid(unsafe_code)]

use crate::archive::{
    archive_error, public_path, resolve, ArchiveMetadata, TitleBatch, ENTRY_NOT_FOUND,
};
use crate::archive_search::Parts;
use std::collections::HashSet;
use std::io;
use std::ops::Range;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use zimru::archive::PositionedInput;
use zimru::{Archive, DirectoryRecord, Dirent, Entry, Error};

fn optional_entry(z: &Archive, namespace: u8, path: &str) -> Result<Option<Entry>, String> {
    match z.entry_by_ns_path(namespace, path) {
        Ok(entry) => Ok(Some(entry)),
        Err(Error::EntryNotFound) => Ok(None),
        Err(error) => Err(archive_error(error)),
    }
}

fn metadata_value(z: &Archive, name: &str) -> Result<Option<String>, String> {
    let Some(entry) = optional_entry(z, b'M', name)? else {
        return Ok(None);
    };
    let item = resolve(entry)?.get_item(false).map_err(archive_error)?;
    let size = item.size().map_err(archive_error)?;
    if size > 1 << 20 {
        return Err("Archive metadata exceeds the 1 MiB limit".into());
    }
    let mut data = vec![0; size as usize];
    if item.read_at(0, &mut data).map_err(archive_error)? != data.len() {
        return Err("Truncated archive metadata".into());
    }
    String::from_utf8(data)
        .map(Some)
        .map_err(|_| "Invalid UTF-8 in archive metadata".into())
}

// Counter records can contain parameters (e.g. text/html;charset=utf-8=12).
// A semicolon ends a record once the count follows its last equals sign.
fn counts(counter: &str) -> (u64, u64) {
    let mut seen = HashSet::new();
    let mut article = 0u64;
    let mut media = 0u64;
    let mut rest = counter;
    while !rest.is_empty() {
        let mut width = rest.find(';').unwrap_or(rest.len());
        if !rest[..width].contains('=') {
            loop {
                if width == rest.len() {
                    return (article, media);
                }
                let start = width + 1;
                width = start + rest[start..].find(';').unwrap_or(rest.len() - start);
                if rest[start..width].bytes().filter(|&b| b == b'=').count() == 2 {
                    break;
                }
            }
        }
        let record = &rest[..width];
        rest = rest[width..].strip_prefix(';').unwrap_or_default();
        if let Some((mime, value)) = record.rsplit_once('=') {
            if let Ok(n) = value.trim_start().parse::<u32>() {
                if seen.insert(mime) {
                    if mime.starts_with("text/html") {
                        article += u64::from(n);
                    }
                    if ["image/", "video/", "audio/"]
                        .iter()
                        .any(|prefix| mime.starts_with(prefix))
                    {
                        media += u64::from(n);
                    }
                }
            }
        }
    }
    (article, media)
}

fn front_listing(z: &Archive) -> Result<bool, String> {
    Ok(z.header().uses_new_namespaces()
        && optional_entry(z, b'X', "listing/titleOrdered/v1")?.is_some())
}

pub(super) fn main_entry(z: &Archive) -> Result<Entry, String> {
    if let Some(entry) = optional_entry(z, b'W', "mainPage")? {
        return Ok(entry);
    }
    if !z.has_main_entry() {
        return Err(format!("{ENTRY_NOT_FOUND}: No main page"));
    }
    z.main_entry().map_err(archive_error)
}

pub(super) fn metadata(z: &Archive, indexes: (bool, bool)) -> Result<ArchiveMetadata, String> {
    let modern = z.header().uses_new_namespaces();
    let value = |key| metadata_value(z, key).map(Option::unwrap_or_default);
    let (mut article_count, media_count) = match metadata_value(z, "Counter")? {
        Some(counter) => counts(&counter),
        None => {
            let n = |ns| {
                z.entry_count_in_namespace(ns)
                    .map(u64::from)
                    .map_err(archive_error)
            };
            (n(if modern { b'C' } else { b'A' })?, n(b'I')? + n(b'J')?)
        }
    };
    if front_listing(z)? {
        article_count = u64::from(z.title_count().map_err(archive_error)?);
    }
    let main_path = if z.has_main_entry() {
        public_path(&resolve(main_entry(z)?)?, modern)?
    } else {
        String::new()
    };
    Ok(ArchiveMetadata {
        id: z.uuid().to_string(),
        title: value("Title")?,
        description: value("Description")?,
        language: value("Language")?,
        creator: value("Creator")?,
        publisher: value("Publisher")?,
        date: value("Date")?,
        name: value("Name")?,
        tags: value("Tags")?
            .split(';')
            .filter(|tag| !tag.is_empty())
            .map(str::to_owned)
            .collect(),
        size: z.file_len(),
        article_count,
        media_count,
        has_fulltext_index: indexes.0,
        has_title_index: indexes.1,
        main_path,
    })
}

fn title_range(z: &Archive) -> Result<Range<u32>, String> {
    if front_listing(z)? {
        return Ok(0..z.title_count().map_err(archive_error)?);
    }
    z.namespace_range(if z.header().uses_new_namespaces() {
        b'C'
    } else {
        b'A'
    })
    .map_err(archive_error)
}

pub(super) fn title_batch(z: &Archive, offset: u32, limit: u32) -> Result<TitleBatch, String> {
    if !(1..=256).contains(&limit) || offset > i32::MAX as u32 - limit {
        return Err("Title batch limit must be 1..256 and offset must fit signed 32 bits".into());
    }
    let range = title_range(z)?;
    let total = range.end - range.start;
    if offset > total {
        return Err("Title batch offset exceeds title count".into());
    }
    let start = range.start + offset;
    let mut titles = Vec::new();
    for i in start..start.saturating_add(limit).min(range.end) {
        let entry = z.title_record(i).map_err(archive_error)?;
        if entry.title().len() > 16 << 10 {
            return Err("Spelling title exceeds 16 KiB limit".into());
        }
        titles.push(entry.title().to_owned());
    }
    Ok(TitleBatch { titles, total })
}

pub(super) fn random_entry(z: &Archive) -> Result<Entry, String> {
    let modern = z.header().uses_new_namespaces();
    let range = if modern {
        0..z.title_count().map_err(archive_error)?
    } else {
        z.namespace_range(b'A').map_err(archive_error)?
    };
    if range.is_empty() {
        return Err(format!("{ENTRY_NOT_FOUND}: No article"));
    }
    let sample = getrandom::u32().map_err(|e| e.to_string())?;
    let offset = ((u64::from(sample) * u64::from(range.end - range.start)) >> 32) as u32;
    let i = range.start + offset;
    if modern {
        z.entry_by_title_index(i)
    } else {
        z.entry_by_url_index(i)
    }
    .map_err(archive_error)
}

fn check_cancel(cancel: Option<&CancellationToken>) -> Result<(), String> {
    if cancel.is_some_and(CancellationToken::is_cancelled) {
        return Err("Integrity check cancelled".into());
    }
    Ok(())
}

// Read tables in bounded chunks; large archives can have millions of entries.
fn check_table<const N: usize>(
    z: &Archive,
    base: u64,
    count: u32,
    cancel: Option<&CancellationToken>,
    mut check: impl FnMut(u32, [u8; N]) -> Result<bool, String>,
) -> Result<bool, String> {
    let mut buf = [0; 16384];
    let mut row = 0;
    while row < count {
        check_cancel(cancel)?;
        let rows = (count - row).min((buf.len() / N) as u32);
        let buf = &mut buf[..rows as usize * N];
        let Some(at) = base.checked_add(u64::from(row) * N as u64) else {
            return Ok(false);
        };
        if z.read_at(at, buf).map_err(archive_error)? != buf.len() {
            return Ok(false);
        }
        for &value in buf.as_chunks::<N>().0 {
            if row % 64 == 0 {
                check_cancel(cancel)?;
            }
            if !check(row, value)? {
                return Ok(false);
            }
            row += 1;
        }
    }
    Ok(true)
}

fn in_order(prev: &mut Option<(u8, String)>, namespace: u8, text: &str, strict: bool) -> bool {
    if let Some((ns, value)) = prev {
        let order = (*ns, value.as_str()).cmp(&(namespace, text));
        if order.is_gt() || strict && order.is_eq() {
            return false;
        }
        *ns = namespace;
        value.clear();
        value.push_str(text);
    } else {
        *prev = Some((namespace, text.to_owned()));
    }
    true
}

#[derive(Debug)]
struct IntegrityInput {
    source: Arc<dyn PositionedInput>,
    cancel: CancellationToken,
}

impl PositionedInput for IntegrityInput {
    fn read_exact_at(&self, mut offset: u64, buf: &mut [u8]) -> io::Result<()> {
        for chunk in buf.chunks_mut(64 << 10) {
            if self.cancel.is_cancelled() {
                return Err(io::Error::other("Integrity check cancelled"));
            }
            self.source.read_exact_at(offset, chunk)?;
            offset = offset
                .checked_add(chunk.len() as u64)
                .ok_or_else(|| io::Error::other("Archive offset overflow"))?;
        }
        Ok(())
    }
}

pub(super) fn structure(
    parts: &Arc<Parts>,
    cancel: Option<&CancellationToken>,
) -> Result<bool, String> {
    check_cancel(cancel)?;
    let source: Arc<dyn PositionedInput> = match cancel {
        Some(cancel) => Arc::new(IntegrityInput {
            source: parts.clone(),
            cancel: cancel.clone(),
        }),
        None => parts.clone(),
    };
    // Validate the current directory records even if a browsing handle has
    // already cached an older directory page or decompressed cluster.
    let result = crate::archive::open_source(source, parts.len())
        .and_then(|archive| check_structure(&archive, cancel));
    check_cancel(cancel)?;
    result
}

fn check_structure(z: &Archive, cancel: Option<&CancellationToken>) -> Result<bool, String> {
    check_cancel(cancel)?;
    let h = z.header();
    if !z.has_checksum() {
        return Ok(false);
    }
    let end = h.checksum_pos;
    // One u32 per admitted cluster, at most 32 MiB. Validating the greatest
    // member suffices because each cluster's valid members start at zero.
    let mut referenced = Vec::new();
    referenced
        .try_reserve_exact(h.cluster_count as usize)
        .map_err(|_| "Insufficient memory to verify archive cluster references")?;
    referenced.resize(h.cluster_count as usize, 0u32);
    let mut previous = None;
    let valid = check_table(z, h.url_ptr_pos, h.entry_count, cancel, |i, bytes| {
        let off = u64::from_le_bytes(bytes);
        if off < 80 || end.saturating_sub(off) < 11 {
            return Ok(false);
        }
        let entry = z.directory_record(i).map_err(archive_error)?;
        if !in_order(&mut previous, entry.namespace(), entry.path(), true) {
            return Ok(false);
        }
        Ok(match entry {
            DirectoryRecord::Entry(Dirent::Article(a)) => {
                let Some(greatest) = referenced.get_mut(a.cluster as usize) else {
                    return Ok(false);
                };
                let Some(count) = a.blob.checked_add(1) else {
                    return Ok(false);
                };
                *greatest = (*greatest).max(count);
                (a.mimetype as usize) < z.mime_list().len()
            }
            DirectoryRecord::Entry(Dirent::Redirect(r)) => r.redirect_index < h.entry_count,
            DirectoryRecord::Retired { .. } => true,
        })
    })?;
    if !valid {
        return Ok(false);
    }
    if h.title_ptr_pos != u64::MAX {
        let mut previous = None;
        let valid = check_table(z, h.title_ptr_pos, h.entry_count, cancel, |_, bytes| {
            let idx = u32::from_le_bytes(bytes);
            if idx >= h.entry_count {
                return Ok(false);
            }
            let entry = z.directory_record(idx).map_err(archive_error)?;
            Ok(in_order(
                &mut previous,
                entry.namespace(),
                entry.title(),
                false,
            ))
        })?;
        if !valid {
            return Ok(false);
        }
    }
    if h.title_ptr_pos == u64::MAX || front_listing(z)? {
        let length = z.title_count().map_err(archive_error)?;
        let mut previous = None;
        for row in 0..length {
            if row % 64 == 0 {
                check_cancel(cancel)?;
            }
            let entry = z.title_record(row).map_err(archive_error)?;
            if !in_order(&mut previous, entry.namespace(), entry.title(), false) {
                return Ok(false);
            }
        }
    }
    for idx in 0..h.cluster_count {
        check_cancel(cancel)?;
        let off = z.cluster_offset(idx).map_err(archive_error)?;
        if off < 80 || off >= end {
            return Ok(false);
        }
        let greatest = referenced[idx as usize].checked_sub(1);
        match z.validate_cluster_references(idx, greatest.as_slice()) {
            Err(Error::BadBlobIndex { .. }) => return Ok(false),
            result => result.map_err(archive_error)?,
        }
    }
    check_cancel(cancel)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_directory_markers_verify_without_becoming_articles() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/retired-markers.zim");
        let archive = crate::archive::Archive::open(&path).unwrap();
        assert!(archive.check_integrity().unwrap());
        assert_eq!(
            archive.read("A/Article", 0, 1024).unwrap(),
            b"<html>active</html>"
        );
    }

    #[test]
    fn missing_member_of_a_valid_cluster_fails_integrity() {
        let mut bytes = include_bytes!("../tests/fixtures/retired-markers.zim").to_vec();
        let table = u64::from_le_bytes(bytes[32..40].try_into().unwrap()) as usize;
        let entry = u64::from_le_bytes(bytes[table..table + 8].try_into().unwrap()) as usize;
        bytes[entry + 12..entry + 16].copy_from_slice(&9u32.to_le_bytes());
        let end = bytes.len() - 16;
        let checksum = md5::compute(&bytes[..end]).0;
        bytes[end..].copy_from_slice(&checksum);
        let file = std::env::temp_dir().join(format!(
            "kiwix-invalid-reference-{}.zim",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&file, bytes).unwrap();
        let archive = crate::archive::Archive::open(&file).unwrap();
        let result = archive.check_integrity();
        drop(archive);
        std::fs::remove_file(file).unwrap();
        assert!(!result.unwrap());
    }

    #[test]
    fn cancelling_a_large_input_read_stops_before_the_next_chunk() {
        #[derive(Debug)]
        struct CancelOnRead(CancellationToken);
        impl PositionedInput for CancelOnRead {
            fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
                assert_eq!((offset, buf.len()), (8, 64 << 10));
                self.0.cancel();
                Ok(())
            }
        }
        let cancel = CancellationToken::new();
        let input = IntegrityInput {
            source: Arc::new(CancelOnRead(cancel.clone())),
            cancel,
        };
        assert_eq!(
            input
                .read_exact_at(8, &mut vec![0; 96 << 10])
                .unwrap_err()
                .to_string(),
            "Integrity check cancelled"
        );
    }

    #[test]
    fn mime_counter_keeps_parameters_and_the_first_count_for_each_type() {
        assert_eq!(
            counts("text/html=2;image/png=3;text/html;charset=utf-8=5;image/png=7;audio/ogg=1"),
            (7, 4)
        );
        assert_eq!(counts("video/mp4=7;text/html;charset=utf-8"), (0, 7));
        assert_eq!(
            counts("text/html=bad;image/jpeg=2;malformed;audio/ogg=7"),
            (0, 2)
        );
    }
}
