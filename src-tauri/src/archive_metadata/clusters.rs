// SPDX-License-Identifier: GPL-3.0-or-later
//! Verify clusters from one retained archive with a bounded desktop worker budget.
#![forbid(unsafe_code)]
use super::{archive_error, check_cancel, Archive, CancellationToken, Error};
use std::sync::atomic::{AtomicBool, Ordering};
use zimru::ClusterReadLimits;

const MAX_WORKERS: usize = 4;
const BATCH: usize = 4096;

// The four small jobs together admit 16 MiB decoded, 16 MiB stored and 32 MiB
// of declared Zstandard windows. This decoder uses its output as history.
// Other codecs and larger cases wait until the small jobs have all joined.
const SMALL: ClusterReadLimits = ClusterReadLimits {
    max_stored_bytes: 4 << 20,
    max_decoded_bytes: 4 << 20,
    xz_memory_bytes: 1 << 20,
    zstd_window_log: 23,
};

fn worker_count(count: usize) -> usize {
    if cfg!(any(
        target_os = "android",
        target_os = "ios",
        target_pointer_width = "32"
    )) || count < 128
    {
        return 1;
    }
    match std::thread::available_parallelism() {
        Ok(n) if n.get() >= MAX_WORKERS => MAX_WORKERS,
        _ => 1,
    }
}

pub(super) fn check(
    z: &Archive,
    references: &[u32],
    cancel: Option<&CancellationToken>,
) -> Result<bool, String> {
    check_with_workers(z, references, cancel, worker_count(references.len()))
}

fn check_with_workers(
    z: &Archive,
    references: &[u32],
    cancel: Option<&CancellationToken>,
    workers: usize,
) -> Result<bool, String> {
    for (batch, references) in references.chunks(BATCH).enumerate() {
        let base = (batch * BATCH) as u32;
        let mut needs_serial = [true; BATCH];
        let remaining = &mut needs_serial[..references.len()];
        if workers > 1
            && samples_include_zstd(z, base, references.len(), cancel)?
            && !small_batch(z, references, base, remaining, cancel, workers)?
        {
            return Ok(false);
        }
        for (row, &count) in references.iter().enumerate() {
            if remaining[row] {
                check_cancel(cancel)?;
                let idx = base + row as u32;
                let off = z.cluster_offset(idx).map_err(archive_error)?;
                if !(80..z.header().checksum_pos).contains(&off) {
                    return Ok(false);
                }
                let greatest = count.checked_sub(1);
                match z.validate_cluster_references(idx, greatest.as_slice()) {
                    Err(Error::BadBlobIndex { .. }) => return Ok(false),
                    result => result.map_err(archive_error)?,
                }
            }
        }
    }
    check_cancel(cancel)?;
    Ok(true)
}

// Raw and XZ batches have nothing to do in the small Zstd workers. Sampling
// controls scheduling only. Every unverified cluster still takes the serial path.
fn samples_include_zstd(
    z: &Archive,
    base: u32,
    count: usize,
    cancel: Option<&CancellationToken>,
) -> Result<bool, String> {
    for row in (0..count).step_by(count.div_ceil(4)) {
        check_cancel(cancel)?;
        if cluster_kind(z, base + row as u32)? == Some(5) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn small_batch(
    z: &Archive,
    references: &[u32],
    base: u32,
    remaining: &mut [bool],
    cancel: Option<&CancellationToken>,
    workers: usize,
) -> Result<bool, String> {
    // Every worker borrows the same reader, including its lazily initialized sorted
    // offset table. The flags cover only this batch, even on a multi-million-cluster file.
    let stop = AtomicBool::new(false);
    let chunk = remaining.len().div_ceil(workers);
    let (first, rest) = remaining.split_at_mut(chunk);
    std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(workers - 1);
        for (i, flags) in rest.chunks_mut(chunk).enumerate() {
            let row = (i + 1) * chunk;
            let stop = &stop;
            let spawn = std::thread::Builder::new()
                .name("archive-clusters".into())
                .spawn_scoped(scope, move || {
                    small_chunk(
                        z,
                        &references[row..row + flags.len()],
                        base + row as u32,
                        flags,
                        cancel,
                        stop,
                    )
                });
            match spawn {
                Ok(handle) => handles.push(handle),
                // The complete ordinary verification path still owns these untouched flags.
                Err(_) => break,
            }
        }
        let mut result = small_chunk(z, &references[..first.len()], base, first, cancel, &stop);
        for handle in handles {
            let other = handle
                .join()
                .map_err(|_| "Archive verification worker failed".to_string())
                .and_then(|r| r);
            if matches!(result, Ok(true)) {
                result = other;
            }
        }
        check_cancel(cancel)?;
        result
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SmallResult {
    Valid,
    RetrySerial,
    Invalid,
}

fn small_chunk(
    z: &Archive,
    references: &[u32],
    base: u32,
    flags: &mut [bool],
    cancel: Option<&CancellationToken>,
    stop: &AtomicBool,
) -> Result<bool, String> {
    let result = (|| {
        for (row, (needed, &count)) in flags.iter_mut().zip(references).enumerate() {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            check_cancel(cancel)?;
            match small_cluster(z, base + row as u32, count)? {
                SmallResult::Valid => *needed = false,
                SmallResult::RetrySerial => (),
                SmallResult::Invalid => return Ok(false),
            }
        }
        Ok(true)
    })();
    if !matches!(result, Ok(true)) {
        stop.store(true, Ordering::Relaxed);
    }
    result
}

fn cluster_kind(z: &Archive, idx: u32) -> Result<Option<u8>, String> {
    let at = z.cluster_offset(idx).map_err(archive_error)?;
    if !(80..z.header().checksum_pos).contains(&at) {
        return Ok(None);
    }
    let mut kind = [0];
    if z.read_at(at, &mut kind).map_err(archive_error)? != 1 {
        return Err(archive_error(Error::Truncated(at)));
    }
    Ok(Some(kind[0] & 0xf))
}

fn small_cluster(z: &Archive, idx: u32, references: u32) -> Result<SmallResult, String> {
    match cluster_kind(z, idx)? {
        None => return Ok(SmallResult::Invalid),
        Some(5) => (),
        Some(_) => return Ok(SmallResult::RetrySerial),
    }
    match z.cluster_with_limits(idx, SMALL) {
        Ok(cluster) if references <= cluster.blob_count() => Ok(SmallResult::Valid),
        Ok(_) => Ok(SmallResult::Invalid),
        Err(error) if crate::archive::decoder_limit(&error) => Ok(SmallResult::RetrySerial),
        Err(error) => Err(archive_error(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io, sync::Arc, time::Duration};
    use zimru::archive::PositionedInput;

    #[derive(Debug)]
    struct Bytes(Vec<u8>);
    impl PositionedInput for Bytes {
        fn read_exact_at(&self, offset: u64, out: &mut [u8]) -> io::Result<()> {
            let start = usize::try_from(offset).map_err(io::Error::other)?;
            let end = start
                .checked_add(out.len())
                .ok_or(io::ErrorKind::UnexpectedEof)?;
            out.copy_from_slice(self.0.get(start..end).ok_or(io::ErrorKind::UnexpectedEof)?);
            Ok(())
        }
    }

    // A standard single-segment Zstd frame. The front raw block supplies the blob
    // offsets and the following run-length blocks produce the requested bytes.
    fn frame(bytes: u32) -> Vec<u8> {
        let mut v = vec![5, 0x28, 0xb5, 0x2f, 0xfd, 0xa0];
        v.extend_from_slice(&(bytes + 8).to_le_bytes());
        v.extend_from_slice(&[64 + u8::from(bytes == 0), 0, 0]);
        v.extend_from_slice(&8u32.to_le_bytes());
        v.extend_from_slice(&(bytes + 8).to_le_bytes());
        let mut left = bytes;
        while left != 0 {
            let n = left.min(128 << 10);
            left -= n;
            let header = (n << 3) | 2 | u32::from(left == 0);
            v.extend_from_slice(&header.to_le_bytes()[..3]);
            v.push(b'x');
        }
        v
    }

    fn file(clusters: &[Vec<u8>]) -> Bytes {
        let n = clusters.len();
        let mut b = vec![0; 81 + 16 * n];
        b[..4].copy_from_slice(b"ZIM\x04");
        b[4] = 6;
        b[24..28].copy_from_slice(&(n as u32).to_le_bytes());
        b[28..32].copy_from_slice(&(n as u32).to_le_bytes());
        let table = 81 + 8 * n;
        for (at, val) in [(32, 81), (40, u64::MAX), (48, table as u64), (56, 80)] {
            b[at..at + 8].copy_from_slice(&val.to_le_bytes());
        }
        b[64..72].fill(0xff);
        for (i, cluster) in clusters.iter().enumerate() {
            let offset = b.len() as u64;
            b[table + i * 8..table + i * 8 + 8].copy_from_slice(&offset.to_le_bytes());
            b.extend_from_slice(cluster);
        }
        let end = b.len() as u64;
        b[72..80].copy_from_slice(&end.to_le_bytes());
        b.extend_from_slice(&md5::compute(&b).0);
        Bytes(b)
    }

    fn archive(clusters: &[Vec<u8>]) -> Archive {
        let data = file(clusters);
        let len = data.0.len() as u64;
        crate::archive::open_source(Arc::new(data), len).unwrap()
    }

    #[test]
    fn big_valid_clusters_fall_back_and_keep_the_same_reference_checks() {
        let z = archive(&[frame(3), frame(5 << 20)]);
        assert!(matches!(
            small_cluster(&z, 0, 1).unwrap(),
            SmallResult::Valid
        ));
        assert!(matches!(
            small_cluster(&z, 1, 1).unwrap(),
            SmallResult::RetrySerial
        ));
        for n in [1, 4] {
            assert!(check_with_workers(&z, &[1, 1], None, n).unwrap());
            assert!(!check_with_workers(&z, &[2, 1], None, n).unwrap());
            assert!(!check_with_workers(&z, &[1, 2], None, n).unwrap());
        }
    }

    #[test]
    fn non_zstd_sample_uses_the_complete_serial_verifier() {
        let mut v = vec![vec![1, 8, 0, 0, 0, 8, 0, 0, 0]; 32];
        v[5] = frame(1);
        let z = archive(&v);
        let mut refs = vec![1; 32];
        assert!(!samples_include_zstd(&z, 0, 32, None).unwrap());
        assert!(check_with_workers(&z, &refs, None, 4).unwrap());
        refs[5] = 2;
        assert!(!check_with_workers(&z, &refs, None, 4).unwrap());
    }

    #[test]
    fn misdeclared_oversized_and_incomplete_frames_fail_both_paths() {
        let mut misdeclared = frame(10);
        misdeclared[6..10].copy_from_slice(&8u32.to_le_bytes());
        let mut short = frame(1);
        short.pop();
        for v in [misdeclared, frame(65 << 20), short] {
            let z = archive(&[v]);
            let serial = check_with_workers(&z, &[1], None, 1).unwrap_err();
            let parallel = check_with_workers(&z, &[1], None, 4).unwrap_err();
            assert_eq!(serial, parallel);
        }
    }

    #[test]
    fn cancelled_parallel_verification_joins_every_active_reader() {
        use std::sync::{atomic::AtomicUsize, mpsc};
        #[derive(Debug)]
        struct Paused {
            source: Bytes,
            token: CancellationToken,
            entered: mpsc::SyncSender<()>,
            active: AtomicUsize,
            start: u64,
        }
        impl PositionedInput for Paused {
            fn read_exact_at(&self, offset: u64, out: &mut [u8]) -> io::Result<()> {
                if offset >= self.start && out.len() == 1 {
                    self.active.fetch_add(1, Ordering::SeqCst);
                    let _ = self.entered.try_send(());
                    while !self.token.is_cancelled() {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    self.active.fetch_sub(1, Ordering::SeqCst);
                    return Err(io::Error::other("integrity read cancelled"));
                }
                self.source.read_exact_at(offset, out)
            }
        }
        let bytes = file(&vec![frame(32); 8]);
        let len = bytes.0.len() as u64;
        let (entered, waiting) = mpsc::sync_channel(4);
        let cancel = CancellationToken::new();
        let input = Arc::new(Paused {
            source: bytes,
            token: cancel.clone(),
            entered,
            active: AtomicUsize::new(0),
            start: 81 + 16 * 8,
        });
        let z = crate::archive::open_source(input.clone(), len).unwrap();
        std::thread::scope(|scope| {
            let worker =
                scope.spawn(|| small_batch(&z, &[1; 8], 0, &mut [true; 8], Some(&cancel), 4));
            let first = waiting.recv_timeout(Duration::from_secs(2));
            let second = waiting.recv_timeout(Duration::from_secs(2));
            cancel.cancel();
            let result = worker.join().unwrap();
            first.unwrap();
            second.unwrap();
            assert_eq!(result.unwrap_err(), "Integrity check cancelled");
        });
        assert_eq!(input.active.load(Ordering::SeqCst), 0);
    }
}
