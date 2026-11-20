// SPDX-License-Identifier: GPL-3.0-or-later
//! Logical ZIM bytes over an admitted, retained exact cohort. The source owns
//! provider leases as well as descriptors until the response retires.
#![forbid(unsafe_code)]
use crate::archive_source::RetainedSource;
use std::io::{self, Read, Seek, SeekFrom};

pub(crate) struct Download {
    source: RetainedSource,
    lengths: Vec<u64>,
    identity: crate::integrity::ContentIdentity,
    pub size: u64,
}
impl Download {
    pub fn new(mut source: RetainedSource, expected: u64) -> Result<Self, String> {
        source.unchanged()?;
        let identity = source.identity()?;
        let lengths = source.files().iter().map(|(_, file)| {
            file.metadata().map(|metadata| metadata.len()).map_err(|error| error.to_string())
        }).collect::<Result<Vec<_>, _>>()?;
        let size = lengths.iter().try_fold(0u64, |sum, size| sum.checked_add(*size))
            .ok_or("Archive part lengths overflow")?;
        if size != expected || size == 0 || lengths.contains(&0) {
            return Err("Archive parts do not match the admitted ZIM length".into());
        }
        let mut header = [0; 80];
        read_parts(source.files_mut(), &lengths, 0, &mut header).map_err(|e| e.to_string())?;
        if &header[..4] != b"ZIM\x04"
            || u64::from_le_bytes(header[72..80].try_into().map_err(|_| "Invalid ZIM header")?)
                .checked_add(16) != Some(size)
        {
            return Err("Archive cohort does not end at its declared ZIM checksum boundary".into());
        }
        source.unchanged()?;
        if source.identity()? != identity { return Err("Archive changed while opening".into()); }
        Ok(Self { source, lengths, size, identity })
    }
    pub(crate) fn identity(&self) -> &crate::integrity::ContentIdentity { &self.identity }
    pub(crate) fn source_names(&self) -> impl Iterator<Item = &str> {
        self.source.files().iter().map(|(name, _)| name.as_str())
    }
    pub(crate) fn has_strong_validator(&self) -> bool {
        match &self.source {
            RetainedSource::Filesystem(_) => true,
            #[cfg(mobile)]
            RetainedSource::Provider(_) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    self.source.files().iter().all(|(_, file)| file.metadata()
                        .is_ok_and(|metadata| metadata.ino() != 0 && metadata.ctime() > 0))
                }
                #[cfg(not(unix))]
                { false }
            }
        }
    }
    pub fn read(&mut self, offset: u64, count: u64) -> io::Result<Vec<u8>> {
        self.source.unchanged().map_err(io::Error::other)?;
        if self.source.identity().map_err(io::Error::other)? != self.identity {
            return Err(io::Error::other("Archive source identity changed"));
        }
        if self.source.files().iter().zip(&self.lengths).any(|((_, file), expected)| {
            file.metadata().map_or(true, |metadata| metadata.len() != *expected)
        }) {
            return Err(io::Error::other("Archive part length changed"));
        }
        if offset.checked_add(count).is_none_or(|end| end > self.size) || count > 1024 * 1024 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid logical archive range"));
        }
        let mut bytes = vec![0; count as usize];
        read_parts(self.source.files_mut(), &self.lengths, offset, &mut bytes)?;
        self.source.unchanged().map_err(io::Error::other)?;
        if self.source.identity().map_err(io::Error::other)? != self.identity {
            return Err(io::Error::other("Archive source identity changed during read"));
        }
        Ok(bytes)
    }
}
fn read_parts(files: &mut [(String, std::fs::File)], lengths: &[u64], mut offset: u64, mut output: &mut [u8]) -> io::Result<()> {
    for ((_, file), length) in files.iter_mut().zip(lengths) {
        if offset >= *length { offset -= length; continue; }
        let count = ((*length - offset).min(output.len() as u64)) as usize;
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut output[..count])?;
        output = &mut output[count..];
        offset = 0;
        if output.is_empty() { return Ok(()); }
    }
    if output.is_empty() { Ok(()) } else { Err(io::Error::new(io::ErrorKind::UnexpectedEof, "Truncated ZIM cohort")) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ranges_cross_every_part_boundary_without_reading_surplus_files() {
        let root = std::env::temp_dir().join(format!("kiwix-share-parts-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let mut files = Vec::new();
        for (index, data) in [b"abc".as_slice(), b"defg", b"hij"].into_iter().enumerate() {
            let path = root.join(index.to_string());
            std::fs::write(&path, data).unwrap();
            files.push((index.to_string(), std::fs::File::open(path).unwrap()));
        }
        for start in 0..10 {
            for end in start + 1..=10 {
                let mut bytes = vec![0; end - start];
                read_parts(&mut files, &[3, 4, 3], start as u64, &mut bytes).unwrap();
                assert_eq!(bytes, b"abcdefghij"[start..end]);
            }
        }
        let mut bytes = [0; 11];
        assert_eq!(read_parts(&mut files, &[3, 4, 3], 0, &mut bytes).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
        drop(files);
        std::fs::remove_dir_all(root).unwrap();
    }
}
