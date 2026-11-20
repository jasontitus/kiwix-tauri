// SPDX-License-Identifier: GPL-3.0-or-later
//! Composition helpers: persistence, bounded fetches, capacity preflight and chunk reading.
#![forbid(unsafe_code)]
#[allow(unused_imports)]
use super::*;

pub(super) struct CompositionFailure { pub(super) message: String, pub(super) retain: bool, pub(super) checkpoint_current: bool }
impl CompositionFailure {
    pub(super) fn invalid(message: &str) -> Self { Self { message: message.into(), retain: false, checkpoint_current: false } }
}
impl From<String> for CompositionFailure {
    fn from(message: String) -> Self { Self { message, retain: true, checkpoint_current: true } }
}
impl From<&str> for CompositionFailure {
    fn from(message: &str) -> Self { message.to_owned().into() }
}

pub(super) struct MapWorkActivity<'a>(pub(super) &'a std::sync::atomic::AtomicBool);
impl<'a> MapWorkActivity<'a> {
    pub(super) fn new(active: &'a std::sync::atomic::AtomicBool) -> Self {
        active.store(true, std::sync::atomic::Ordering::Release);
        Self(active)
    }
}
impl Drop for MapWorkActivity<'_> {
    fn drop(&mut self) { self.0.store(false, std::sync::atomic::Ordering::Release); }
}

pub(super) struct OwnedPartialGuard<'a> {
    pub(super) manager: &'a MapManager,
    pub(super) id: &'a str,
    pub(super) record: OwnedPartial,
    pub(super) armed: bool,
}

impl Drop for OwnedPartialGuard<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let manager = self.manager;
        let id = self.id;
        if matches!(
            reclaim_partial(&mut self.record, |record| {
                manager.checkpoint_partial(id, Some(record.clone()))
            }),
            Ok(PartialRecovery::Reclaimed)
        ) {
            let _ = self.manager.checkpoint_partial(self.id, None);
        }
    }
}

pub(super) fn persist(path: &Path, job: &MapJob) -> Result<(), String> {
    use std::io::Write;
    let temp = path.with_extension(format!("{}.tmp", Uuid::new_v4()));
    let cleanup;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|e| e.to_string())?;
    cleanup = PartialFile(temp.clone());
    let bytes = serde_json::to_vec(job).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_STATE {
        return Err("Map job history exceeds its state limit; previous claims were retained".into());
    }
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    fs::rename(&temp, path).map_err(|e| format!("Cannot publish map job state: {e}"))?;
    #[cfg(unix)]
    fs::File::open(path.parent().ok_or("Map state file has no parent")?)
        .and_then(|directory| directory.sync_all())
        .map_err(|e| format!("Cannot sync map job state directory: {e}"))?;
    drop(cleanup);
    Ok(())
}

pub(super) struct PartialFile(pub(super) PathBuf);
impl Drop for PartialFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub(super) async fn fetch_json<T: DeserializeOwned + Send + 'static>(
    client: &reqwest::Client,
    url: Url,
    control: &Control,
) -> Result<T, String> {
    let response = tokio::select! {
        biased;
        _ = control.cancelled() => return Err(CANCELLED.into()),
        result = client.get(url).header("Accept", "application/json").header("Accept-Encoding", "identity").send() => result.map_err(|e| format!("StreetZim request failed: {e}"))?,
    };
    if response.status() != reqwest::StatusCode::OK {
        return Err(format!(
            "StreetZim JSON returned HTTP {}",
            response.status()
        ));
    }
    let bytes = bounded_body(response, MAX_JSON, control, |_, _| {}).await?;
    let control = control.clone();
    tokio::task::spawn_blocking(move || {
        control.check()?;
        #[cfg(test)]
        if let Some(hook) = control.before_json_parse.lock().unwrap().take() {
            hook();
        }
        let result =
            serde_json::from_slice::<T>(&bytes).map_err(|e| format!("Invalid StreetZim JSON: {e}"));
        control.check()?;
        result
    })
    .await
    .map_err(|e| format!("StreetZim JSON worker failed: {e}"))?
}

pub(super) async fn bounded_body(
    response: reqwest::Response,
    maximum: usize,
    control: &Control,
    mut progress: impl FnMut(u64, u64),
) -> Result<Vec<u8>, String> {
    if response
        .content_length()
        .is_some_and(|len| len > maximum as u64)
    {
        return Err("StreetZim response exceeds allowed length".into());
    }
    if response
        .headers()
        .get(reqwest::header::CONTENT_ENCODING)
        .is_some_and(|v| v != "identity")
    {
        return Err("Unexpected compressed StreetZim response".into());
    }
    let mut bytes =
        Vec::with_capacity(response.content_length().unwrap_or(0).min(maximum as u64) as usize);
    let mut stream = response.bytes_stream();
    progress(0, maximum as u64);
    loop {
        let next = tokio::select! {
            biased;
            _ = control.cancelled() => return Err(CANCELLED.into()),
            next = stream.next() => next,
        };
        let Some(chunk) = next else {
            break;
        };
        let chunk = chunk.map_err(|e| format!("StreetZim transfer failed: {e}"))?;
        if chunk.len() > maximum - bytes.len() {
            return Err("StreetZim response exceeds allowed length".into());
        }
        bytes.extend_from_slice(&chunk);
        progress(bytes.len() as u64, maximum as u64);
    }
    control.check()?;
    Ok(bytes)
}

pub(super) fn same_volume(a: &Path, b: &Path) -> Result<bool, String> {
    // cap-fs-ext exposes native volume identity on Windows as well as Unix.
    // Assuming a shared volume would skip admission on a separate cache drive.
    let metadata = |path: &Path| {
        Dir::open_ambient_dir(path, cap_std::ambient_authority())
            .and_then(|directory| directory.dir_metadata())
            .map_err(|e| format!("Inspecting map volume identity: {e}"))
    };
    Ok(metadata(a)?.dev() == metadata(b)?.dev())
}

pub(super) fn check_capacity(
    output: u64,
    cache: u64,
    destination: &Path,
    cache_dir: &Path,
) -> Result<(), String> {
    check_capacity_with(output, cache, destination, cache_dir, None)
}

pub(super) fn check_capacity_with(
    output: u64,
    cache: u64,
    destination: &Path,
    cache_dir: &Path,
    available_override: Option<u64>,
) -> Result<(), String> {
    let shared = same_volume(destination, cache_dir)?;
    let need = output
        .checked_add(if shared { cache } else { 0 })
        .ok_or("Map disk requirement overflow")?;
    let available_output = match available_override {
        Some(available) => available,
        None => crate::fs_ext::available_space(destination)
            .map_err(|e| format!("Cannot check map destination space: {e}"))?,
    };
    let available_cache = match available_override {
        Some(available) => available,
        None => crate::fs_ext::available_space(cache_dir)
            .map_err(|e| format!("Cannot check map cache space: {e}"))?,
    };
    if available_output < need || available_cache < cache {
        return Err("Insufficient disk space for map and downloaded parts".into());
    }
    Ok(())
}

pub(super) fn preflight(
    recipe: &Recipe,
    local: bool,
    cache: &Path,
    destination: &Path,
    control: &Control,
    remaining_output: u64,
    cache_bytes: u64,
) -> Result<(), String> {
    // The composed archive is one file; downloaded source/cache totals are not.
    crate::storage::check_file_size(destination, recipe.size)?;
    check_capacity_with(remaining_output, 0, destination, cache, control.available_bytes())?;
    let mut ranges: HashMap<String, Vec<(u64, u64)>> = HashMap::new();
    // The base UUID header is verified even if no segment references it.
    if !local {
        ranges.entry("base".into()).or_default().push((0, 1));
    }
    for segment in &recipe.segments {
        control.check()?;
        if !matches!(segment.src.as_str(), "base" | "pack") || (segment.src == "base" && local) {
            continue;
        }
        let key = segment.source_key()?;
        let source = recipe.source(&key)?;
        let off = segment.off.ok_or("Missing source offset")?;
        let len = segment.len.ok_or("Missing source length")?;
        if len > 0 {
            ranges
                .entry(key)
                .or_default()
                .push((off / source.chunk, (off + len - 1) / source.chunk + 1));
        }
    }
    let mut touched = HashSet::new();
    let mut missing = 0u64;
    for (key, ranges) in ranges {
        let source = recipe.source(&key)?;
        for (start, end) in Intervals::new(ranges, control)?.0 {
            for index in start..end {
                control.check()?;
                let len = source.chunk.min(source.size - index * source.chunk);
                if len > MAX_CACHE_BYTES { continue; }
                let name = format!("{}-{len}", source.sha256[index as usize]);
                if touched.insert(name.clone())
                    && !fs::metadata(cache.join(name))
                        .is_ok_and(|meta| meta.is_file() && meta.len() == len)
                {
                    missing = missing
                        .checked_add(len)
                        .ok_or("Map cache requirement overflow")?;
                }
            }
        }
    }
    // Only growth beyond the retained bounded cache needs additional free space.
    // Oversized chunks stay in the existing bounded RAM working set.
    let missing = missing.min(MAX_CACHE_BYTES.saturating_sub(cache_bytes));
    check_capacity_with(remaining_output, missing, destination, cache, control.available_bytes())?;
    let shared = same_volume(destination, cache)?;
    let output_remaining = remaining_output
        .checked_add(if shared { missing } else { 0 })
        .ok_or("Map disk requirement overflow")?;
    let identity = serde_json::to_string(&(
        control.space_identity.as_deref(),
        &recipe.uuid,
        &recipe.md5,
        &recipe.file_md5,
        recipe.output_stem(),
    ))
    .map_err(|e| e.to_string())?;
    let mut acknowledgements = (*control.space_acknowledgements).clone();
    acknowledgements.check(
        destination, output_remaining, &identity, false, control.available_bytes(),
    )?;
    if !shared && missing > 0 {
        acknowledgements.check(
            cache, missing, &identity, true, control.available_bytes(),
        )?;
    }
    Ok(())
}

// One opaque UI token retains each exact request/volume/remaining-byte approval.
// Initial composition and a later local-base fallback have distinct requirements;
// retaining both prevents an explicit retry from alternating their warnings.
#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct MapSpaceAcknowledgements {
    pub(super) attempt: Option<String>,
    pub(super) output: Vec<String>,
    pub(super) cache: Vec<String>,
}

impl MapSpaceAcknowledgements {
    pub(super) fn check(
        &mut self,
        directory: &Path,
        remaining: u64,
        identity: &str,
        cache_volume: bool,
        available_override: Option<u64>,
    ) -> Result<(), String> {
        let Err(error) = crate::disk_warning::check_with_available(
            directory,
            Some(remaining),
            identity,
            None,
            available_override,
        ) else {
            return Ok(());
        };
        let Some(json) = error.strip_prefix("LOW_DISK_SPACE:") else {
            return Err(error);
        };
        let mut warning: serde_json::Value =
            serde_json::from_str(json).map_err(|_| error.clone())?;
        let token = warning["acknowledgement"]
            .as_str()
            .ok_or_else(|| error.clone())?
            .to_owned();
        let acknowledgements = if cache_volume { &mut self.cache } else { &mut self.output };
        if acknowledgements.contains(&token) { return Ok(()); }
        acknowledgements.push(token);
        warning["acknowledgement"] =
            serde_json::to_string(self).map_err(|e| e.to_string())?.into();
        Err(format!("LOW_DISK_SPACE:{warning}"))
    }
}

/// One entry per masked SHA-256/length, including the base UUID header. Borrow
/// recipe hashes rather than copying them or estimating by 16 MiB division.
pub(super) struct BlockProgress<'a> {
    pub(super) required: HashMap<(&'a str, u64), usize>,
    pub(super) states: Vec<u8>,
    pub(super) ready: u64,
    pub(super) downloaded: u64,
}

impl<'a> BlockProgress<'a> {
    pub(super) fn new(recipe: &'a Recipe, control: &Control) -> Result<Self, String> {
        let mut required = HashMap::new();
        required.insert((recipe.base.sha256[0].as_str(), recipe.base.chunk.min(recipe.base.size)), 0);
        let mut states = vec![0];
        let mut reads = 1u64;
        for segment in &recipe.segments {
            control.check()?;
            if !matches!(segment.src.as_str(), "base" | "pack") { continue; }
            let source = recipe.source(&segment.source_key()?)?;
            let off = segment.off.ok_or("Missing source offset")?;
            let len = segment.len.ok_or("Missing source length")?;
            if len == 0 { continue; }
            let first = off / source.chunk;
            let last = (off + len - 1) / source.chunk;
            reads = reads.checked_add(last - first + 1).ok_or("Map block count overflow")?;
            if reads > MAX_ESTIMATE_CHUNK_READS {
                return Err("Map recipe is too complex to count blocks safely".into());
            }
            for index in first..=last {
                control.check()?;
                let next = states.len();
                if let std::collections::hash_map::Entry::Vacant(entry) = required.entry((
                    source.sha256[index as usize].as_str(),
                    source.chunk.min(source.size - index * source.chunk))) {
                    entry.insert(next);
                    states.push(0);
                }
            }
        }
        Ok(Self { required, states, ready: 0, downloaded: 0 })
    }

    pub(super) fn restore(&mut self, states: &[u8]) -> Result<(), String> {
        if states.len() != self.states.len() || states.iter().any(|state| !matches!(*state, 0 | 1 | 3)) {
            return Err("Saved map block checkpoint does not match the exact recipe".into());
        }
        self.states.copy_from_slice(states);
        self.ready = states.iter().filter(|state| **state & 1 != 0).count() as u64;
        self.downloaded = states.iter().filter(|state| **state & 2 != 0).count() as u64;
        Ok(())
    }

    pub(super) fn mark(
        &mut self, hash: &str, len: u64, network: bool,
        manager: &MapManager, id: &str, control: &Control,
    ) -> Result<(), String> {
        control.check()?;
        let index = *self.required.get(&(hash, len)).ok_or("Verified map block is not required by its recipe")?;
        let state = &mut self.states[index];
        let mut changed = false;
        if *state & 1 == 0 { *state |= 1; self.ready += 1; changed = true; }
        if network && *state & 2 == 0 { *state |= 2; self.downloaded += 1; changed = true; }
        if changed {
            manager.update(id, |job| {
                job.blocks_ready = Some(self.ready);
                job.blocks_downloaded = Some(self.downloaded);
            });
        }
        Ok(())
    }
}

/// A read-only local base candidate. Candidates are hints: every reused chunk
/// is still verified against the recipe digest. The path is opened per read so
/// a composition never holds one descriptor per candidate for its whole
/// lifetime, and the admitted inode is re-checked so a replaced file is never
/// read as if it were the candidate that passed admission.
pub(super) struct LocalCandidate {
    pub(super) path: PathBuf,
    pub(super) identity: FileIdentity,
}

impl LocalCandidate {
    /// Admit `path` when it is a readable regular file at least `minimum` bytes
    /// long, pinning the inode that admission observed.
    pub(super) async fn open(path: PathBuf, minimum: u64) -> Option<Self> {
        let file = tokio::fs::File::open(&path).await.ok()?;
        let metadata = file.metadata().await.ok()?;
        if !metadata.is_file() || metadata.len() < minimum {
            return None;
        }
        let identity = candidate_identity(&file).await.ok()?;
        Some(Self { path, identity })
    }
}

/// Identity of a candidate opened outside the destination capability, read
/// through its open handle: Windows has no device/inode in path metadata, and
/// `cap_std` reports the volume serial number and file index from the handle.
async fn candidate_identity(file: &tokio::fs::File) -> io::Result<FileIdentity> {
    let handle = file.try_clone().await?.into_std().await;
    let metadata = tokio::task::spawn_blocking(move || cap_std::fs::Metadata::from_file(&handle))
        .await
        .map_err(io::Error::other)??;
    Ok(FileIdentity { device: metadata.dev(), inode: metadata.ino() })
}

pub(super) struct ChunkReader<'a> {
    pub(super) manager: &'a MapManager,
    pub(super) id: &'a str,
    pub(super) control: &'a Control,
    pub(super) recipe: &'a Recipe,
    pub(super) masks: &'a HashMap<String, Intervals>,
    pub(super) urls: &'a HashMap<String, Url>,
    pub(super) local: Vec<LocalCandidate>,
    // Chunk index -> candidate that verified it, and chunk indices no admitted
    // candidate can serve. Both keep a repeated request from rescanning every
    // candidate and re-reading/hashing the same bytes again.
    pub(super) local_verified: HashMap<u64, usize>,
    pub(super) local_failed: HashSet<u64>,
    // The candidate that verified the most recent chunk. A cold base is served
    // by one candidate; trying it first avoids reading and hashing every other
    // candidate for every chunk.
    pub(super) last_local: Option<usize>,
    pub(super) destination: Arc<MapDirectory>,
    pub(super) last: Option<(String, Arc<Vec<u8>>)>,
    pub(super) written: u64,
    pub(super) fallback_space_admitted: bool,
    pub(super) cache: MapChunkCache,
    pub(super) blocks: BlockProgress<'a>,
}

impl ChunkReader<'_> {
    pub(super) async fn get(&mut self, key: &str, index: u64) -> Result<Arc<Vec<u8>>, String> {
        self.control.check()?;
        self.cache.unchanged()?;
        self.destination.unchanged().map_err(|error| error.to_string())?;
        let source = self.recipe.source(key)?;
        let expected = source
            .sha256
            .get(index as usize)
            .ok_or("Chunk index exceeds source")?;
        let start = index
            .checked_mul(source.chunk)
            .ok_or("Chunk offset overflow")?;
        let len = source.chunk.min(
            source
                .size
                .checked_sub(start)
                .ok_or("Chunk exceeds source")?,
        );
        let name = format!("{expected}-{len}");
        if let Some((last_key, bytes)) = &self.last {
            if *last_key == name {
                return Ok(bytes.clone());
            }
        }
        // Release the previous working chunk before allocating another.
        self.last = None;
        if let Some(file) = self.cache.open_chunk(&name)? {
            if let Ok(bytes) = read_exact_file(file, len, self.control).await {
                if self.control.chunk_sha256(&bytes) == *expected {
                    self.blocks.mark(expected, len, false, self.manager, self.id, self.control)?;
                    return Ok(self.remember(name, bytes));
                }
            }
            self.control.check()?;
            // A content-addressed filename is not proof of correct retained bytes.
            self.cache.discard(&name)?;
        }
        let mask = self.masks.get(key).ok_or("Missing source mask")?;
        if key == "base" && !self.local.is_empty() && !self.local_failed.contains(&index) {
            // Try the candidate that verified this chunk (or the last one that
            // verified anything) first. A cold base otherwise re-reads and
            // re-hashes every earlier candidate for every chunk.
            let preferred = self
                .local_verified
                .get(&index)
                .copied()
                .or(self.last_local);
            let mut order = Vec::with_capacity(self.local.len());
            if let Some(preferred) = preferred {
                order.push(preferred);
            }
            order.extend((0..self.local.len()).filter(|position| Some(*position) != preferred));
            // Reuse one allocation while trying multiple read-only language/base candidates.
            let mut bytes = vec![0u8; len as usize];
            let mut verified = None;
            for position in order {
                self.control.check()?;
                let candidate = &self.local[position];
                let read = tokio::select! {
                    biased;
                    _ = self.control.cancelled() => return Err(CANCELLED.into()),
                    result = async {
                        let mut file = tokio::fs::File::open(&candidate.path).await?;
                        if candidate_identity(&file).await? != candidate.identity {
                            return Err(io::Error::other(
                                "Map base candidate was replaced",
                            ));
                        }
                        file.seek(io::SeekFrom::Start(start)).await?;
                        file.read_exact(&mut bytes).await
                    } => result,
                };
                if read.is_ok() {
                    mask.apply(&mut bytes, start, self.control)?;
                    if self.control.chunk_sha256(&bytes) == *expected {
                        self.local_verified.insert(index, position);
                        self.last_local = Some(position);
                        verified = Some(bytes);
                        break;
                    }
                }
            }
            if verified.is_none() {
                // No admitted candidate serves this chunk; a later segment that
                // references it again must not rescan them.
                self.local_failed.insert(index);
            }
            if let Some(bytes) = verified {
                self.blocks.mark(expected, len, false, self.manager, self.id, self.control)?;
                return Ok(self.remember(name, bytes));
            }
        }
        if key == "base" && !self.local.is_empty() && !self.fallback_space_admitted {
            // A size-qualified local candidate was only a hint. If no candidate
            // verifies this chunk, reserve and confirm every remaining possible
            // base download before any fallback request or cache growth.
            preflight(
                self.recipe, false, &self.manager.inner.cache, &self.destination.path,
                self.control, self.recipe.size.saturating_sub(self.written), self.cache.bytes()?,
            )?;
            self.fallback_space_admitted = true;
        }
        check_capacity(
            self.recipe.size - self.written,
            if len <= MAX_CACHE_BYTES { len.min(MAX_CACHE_BYTES.saturating_sub(self.cache.bytes()?)) } else { 0 },
            &self.destination.path,
            &self.manager.inner.cache,
        )?;
        self.cache.unchanged()?;
        self.destination.unchanged().map_err(|error| error.to_string())?;
        let url = self.urls.get(key).ok_or("Missing source URL")?;
        for attempt in 0..3u64 {
            self.control.check()?;
            let result = self.fetch_range(key, url.clone(), start, len, source.size).await;
            let verified = match result {
                Ok(mut bytes) => {
                    mask.apply(&mut bytes, start, self.control)?;
                    if self.control.chunk_sha256(&bytes) == *expected {
                        Ok(bytes)
                    } else {
                        Err("Downloaded map chunk SHA-256 mismatch".into())
                    }
                }
                Err(error) => Err(error),
            };
            match verified {
                Ok(bytes) => {
                    self.control.check()?;
                    let bytes = Arc::new(bytes);
                    let destination = self.destination.clone();
                    let inner = self.manager.inner.clone();
                    let remaining = self.recipe.size.saturating_sub(self.written);
                    let control = Control::clone(self.control);
                    self.cache.store(&name, bytes.clone(), move || {
                        control.check()?;
                        destination.unchanged().map_err(|error| error.to_string())?;
                        // Eviction has completed. Open-but-unlinked/hard-linked
                        // cache data provides no imaginary physical-space credit.
                        check_capacity(remaining, len, &destination.path, &inner.cache)?;
                        destination.unchanged().map_err(|error| error.to_string())
                    }).await?;
                    self.control.check()?;
                    self.blocks.mark(expected, len, true, self.manager, self.id, self.control)?;
                    self.manager.update(self.id, |j| {
                        j.incoming = 0;
                        j.incoming_total = 0;
                    });
                    return Ok(self.remember_shared(name, bytes));
                }
                Err(error) => {
                    self.control.check()?;
                    if attempt == 2 {
                        return Err(error);
                    }
                    tokio::select! {
                        biased;
                        _ = self.control.cancelled() => return Err(CANCELLED.into()),
                        _ = tokio::time::sleep(Duration::from_secs(attempt + 1)) => {},
                    }
                }
            }
        }
        Err("Map chunk transfer failed".into())
    }

    pub(super) fn remember(&mut self, key: String, bytes: Vec<u8>) -> Arc<Vec<u8>> {
        self.remember_shared(key, Arc::new(bytes))
    }

    pub(super) fn remember_shared(&mut self, key: String, bytes: Arc<Vec<u8>>) -> Arc<Vec<u8>> {
        self.last = Some((key, bytes.clone()));
        bytes
    }

    pub(super) async fn fetch_range(
        &self,
        _key: &str,
        url: Url,
        offset: u64,
        length: u64,
        size: u64,
    ) -> Result<Vec<u8>, String> {
        if length == 0 || length > MAX_CHUNK || offset > size || length > size - offset {
            return Err("Invalid StreetZim range".into());
        }
        let end = offset + length - 1;
        self.manager.update(self.id, |j| {
            j.incoming = 0;
            j.incoming_total = length;
        });
        #[cfg(all(feature = "android-evaluation", target_os = "android", debug_assertions))]
        if !self.control.evaluation_sources.is_empty() {
            // Fixture transport supplies raw ranges, never composed or preverified
            // output. The common mask/SHA/cache path below remains unchanged.
            let file = self.control.evaluation_sources.get(_key).cloned()
                .ok_or("Missing staged map source")?;
            let control = self.control.clone();
            return tokio::task::spawn_blocking(move || {
                use std::os::unix::fs::FileExt;
                if file.metadata().map_err(|error| error.to_string())?.len() != size {
                    return Err("Staged map source size changed".into());
                }
                let mut bytes = vec![0; length as usize];
                let mut read = 0usize;
                while read < bytes.len() {
                    control.check()?;
                    let end = bytes.len().min(read + 1024 * 1024);
                    let count = file.read_at(&mut bytes[read..end], offset + read as u64)
                        .map_err(|error| error.to_string())?;
                    if count == 0 { return Err("Staged map source range was truncated".into()); }
                    read += count;
                }
                control.check()?;
                Ok(bytes)
            }).await.map_err(|error| format!("Staged source worker failed: {error}"))?;
        }
        let client = self.manager.inner.client(self.control).await?;
        let response = tokio::select! {
            biased;
            _ = self.control.cancelled() => return Err(CANCELLED.into()),
            result = client.get(url).header("Range", format!("bytes={offset}-{end}"))
                .header("Accept-Encoding", "identity").send() => result.map_err(|e| format!("StreetZim range request failed: {e}"))?,
        };
        let expected = format!("bytes {offset}-{end}/{size}");
        if response.status() != reqwest::StatusCode::PARTIAL_CONTENT
            || response
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                != Some(expected.as_str())
        {
            return Err("StreetZim source returned an invalid HTTP range response".into());
        }
        let bytes = bounded_body(
            response,
            length as usize,
            self.control,
            |incoming, total| {
                self.manager.update(self.id, |j| {
                    j.incoming = incoming;
                    j.incoming_total = total;
                });
            },
        )
        .await?;
        if bytes.len() as u64 != length {
            return Err("StreetZim source range was truncated".into());
        }
        Ok(bytes)
    }
}

pub(super) async fn read_exact_file(file: fs::File, length: u64, control: &Control) -> Result<Vec<u8>, String> {
    let mut file = tokio::fs::File::from_std(file);
    let mut bytes = vec![0u8; length as usize];
    tokio::select! {
        biased;
        _ = control.cancelled() => return Err(CANCELLED.into()),
        result = file.read_exact(&mut bytes) => { result.map_err(|e| e.to_string())?; },
    }
    // Check EOF, guarding a file growing between metadata and read.
    let mut extra = [0u8; 1];
    if file.read(&mut extra).await.map_err(|e| e.to_string())? != 0 {
        return Err("Cached chunk length changed".into());
    }
    control.check()?;
    Ok(bytes)
}
