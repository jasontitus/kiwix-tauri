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

// The limits and decoded-cluster cache budget of `open_source` in
// src-tauri/src/archive.rs (desktop values); keep the two in step. The app also
// rejects inconsistent table extents (`validate_layout`) before it reads
// anything else, so this target reaches a superset of the inputs the app admits.
const MAX_STRING: usize = 1 << 20;
const MAX_CLUSTERS: u32 = 8 * 1024 * 1024;
const CLUSTER_CACHE_MAX_BYTES: usize = 32 << 20;

fuzz_target!(|data: &[u8]| {
    let limits = ArchiveLimits {
        max_metadata_record_bytes: 2 * MAX_STRING + 512,
        max_clusters: MAX_CLUSTERS,
        cluster: Some(ClusterReadLimits {
            max_decoded_bytes: 64 << 20,
            max_stored_bytes: 65 << 20,
            xz_memory_bytes: 128 << 20,
            zstd_window_log: 26,
        }),
        ..Default::default()
    };
    let input = Arc::new(Bytes(data.to_vec()));
    let Ok(archive) = Archive::from_positioned(input, data.len() as u64, limits) else { return; };
    archive.set_cluster_cache_max_bytes(CLUSTER_CACHE_MAX_BYTES);
    let entries = archive.entry_count();
    let _ = archive.title_count();
    let _ = archive.main_entry();
    let _ = archive.entry_by_title_index(0);
    // Directory entries alone never reach a cluster. Resolve the first entries to
    // their blobs: that decodes zstd/xz clusters and walks the blob offset tables,
    // as serving an article does.
    let mut buffer = [0u8; 4096];
    for index in 0..entries.min(64) {
        let Ok(entry) = archive.entry_by_url_index(index) else { continue; };
        let Ok(item) = entry.get_item(true) else { continue; };
        let _ = item.size();
        let _ = item.read_at(0, &mut buffer);
    }
    let _ = archive.title_prefix_range(b'C', "a");
    for cluster in 0..archive.cluster_count().min(4) {
        let _ = archive.validate_cluster_references(cluster, &[]);
    }
    let _ = archive.check();
});
