#![no_main]
use libfuzzer_sys::fuzz_target;
use std::{io, sync::Arc};
use zimru::{Archive, ArchiveLimits, ClusterReadLimits};
use zimru::archive::PositionedInput;

#[derive(Debug)]
struct Bytes(Vec<u8>);

impl PositionedInput for Bytes {
    fn read_exact_at(&self, offset: u64, destination: &mut [u8]) -> io::Result<()> {
        let end = offset.checked_add(destination.len() as u64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "offset overflow"))?;
        let end = usize::try_from(end).map_err(|_| io::ErrorKind::UnexpectedEof)?;
        let start = usize::try_from(offset).map_err(|_| io::ErrorKind::UnexpectedEof)?;
        let source = self.0.get(start..end).ok_or(io::ErrorKind::UnexpectedEof)?;
        destination.copy_from_slice(source);
        Ok(())
    }
}

fuzz_target!(|data: &[u8]| {
    let limits = ArchiveLimits {
        max_metadata_record_bytes: 1 << 20,
        max_entries: 10_000,
        max_clusters: 1_000,
        cluster: Some(ClusterReadLimits {
            max_stored_bytes: 64 << 20,
            max_decoded_bytes: 64 << 20,
            xz_memory_bytes: 64 << 20,
            zstd_window_log: 27,
        }),
    };
    let input = Arc::new(Bytes(data.to_vec()));
    let Ok(archive) = Archive::from_positioned(input, data.len() as u64, limits) else { return; };
    let _ = archive.entry_count();
    let _ = archive.title_count();
    let _ = archive.main_entry();
    let _ = archive.entry_by_url_index(0);
    let _ = archive.entry_by_title_index(0);
    let _ = archive.check();
});
