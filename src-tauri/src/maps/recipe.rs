// SPDX-License-Identifier: GPL-3.0-or-later
//! StreetZim recipe, source and segment models with interval accounting.
#[allow(unused_imports)]
use super::*;

#[derive(Serialize, Deserialize)]
pub(super) struct Recipe {
    pub(super) format: String,
    pub(super) uuid: String,
    pub(super) name: String,
    pub(super) file_name: Option<String>,
    pub(super) size: u64,
    pub(super) md5: String,
    pub(super) file_md5: Option<String>,
    pub(super) base: Source,
    pub(super) packs: HashMap<String, Source>,
    pub(super) segments: Vec<Segment>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Source {
    pub(super) file: String,
    pub(super) size: u64,
    pub(super) uuid: String,
    pub(super) chunk: u64,
    pub(super) sha256: Vec<String>,
    pub(super) mask: Option<Vec<Vec<u64>>>,
    pub(super) url: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Segment {
    pub(super) src: String,
    pub(super) pack: Option<String>,
    pub(super) off: Option<u64>,
    pub(super) len: Option<u64>,
    pub(super) data: Option<String>,
    pub(super) sha256: Option<String>,
}

// Sorted, disjoint half-open ranges. All arithmetic is bounded before addition.
#[derive(Default)]
pub(super) struct Intervals(pub(super) Vec<(u64, u64)>);

impl Intervals {
    pub(super) fn new(mut ranges: Vec<(u64, u64)>, control: &Control) -> Result<Self, String> {
        // Cancellable in-place heapsort keeps O(M log M) bounds without a second mask array.
        fn sift(
            ranges: &mut [(u64, u64)],
            mut root: usize,
            control: &Control,
        ) -> Result<(), String> {
            while root < ranges.len() / 2 {
                control.check()?;
                let mut child = root * 2 + 1;
                if child + 1 < ranges.len() && ranges[child] < ranges[child + 1] {
                    child += 1;
                }
                if ranges[root] >= ranges[child] {
                    break;
                }
                ranges.swap(root, child);
                root = child;
            }
            Ok(())
        }
        for root in (0..ranges.len() / 2).rev() {
            sift(&mut ranges, root, control)?;
        }
        for end in (1..ranges.len()).rev() {
            control.check()?;
            ranges.swap(0, end);
            sift(&mut ranges[..end], 0, control)?;
        }
        let mut count = 0;
        for index in 0..ranges.len() {
            control.check()?;
            let (start, end) = ranges[index];
            if start == end {
                continue;
            }
            if count > 0 && start <= ranges[count - 1].1 {
                ranges[count - 1].1 = ranges[count - 1].1.max(end);
            } else {
                ranges[count] = (start, end);
                count += 1;
            }
        }
        ranges.truncate(count);
        Ok(Self(ranges))
    }

    pub(super) fn first(&self, offset: u64) -> usize {
        self.0.partition_point(|range| range.1 <= offset)
    }

    pub(super) fn overlaps(&self, start: u64, end: u64) -> bool {
        start < end
            && self
                .0
                .get(self.first(start))
                .is_some_and(|range| range.0 < end)
    }

    pub(super) fn apply(&self, bytes: &mut [u8], offset: u64, control: &Control) -> Result<(), String> {
        let end = offset
            .checked_add(bytes.len() as u64)
            .ok_or("Mask offset overflow")?;
        for &(start, stop) in &self.0[self.first(offset)..] {
            control.check()?;
            if start >= end {
                break;
            }
            bytes[(start.max(offset) - offset) as usize..(stop.min(end) - offset) as usize].fill(0);
        }
        Ok(())
    }
}

pub(super) fn valid_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(super) fn checked_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|e| format!("Invalid StreetZim URL: {e}"))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err("StreetZim URLs must use HTTPS without credentials or fragments".into());
    }
    Ok(url)
}

pub(super) fn recipe_url(index: &Url, recipe: &str) -> Result<Url, String> {
    // The protocol resolves recipes/a/b.json against /build/, NOT /build/recipes/.
    let root = index.join("../").map_err(|e| e.to_string())?;
    let url = root.join(recipe).map_err(|e| e.to_string())?;
    checked_url(url.as_str())
}

pub(super) fn source_url(value: &str, proxy: &Url) -> Result<Url, String> {
    checked_url(&value.replace("{proxy}", proxy.as_str().trim_end_matches('/')))
}

impl MapCatalog {
    pub(super) fn validate(&self, index: &Url, control: &Control) -> Result<(), String> {
        if self.format != "streetzim-build-index/1" {
            return Err("Unsupported StreetZim catalog format".into());
        }
        let mut regions = HashSet::new();
        for region in &self.regions {
            control.check()?;
            if region.id.is_empty()
                || region.title.is_empty()
                || region.base.size == 0
                || region.base.size > i64::MAX as u64
                || !regions.insert(region.id.as_str())
            {
                return Err("Invalid or duplicate StreetZim region".into());
            }
            let mut languages = HashSet::new();
            for language in &region.menu {
                control.check()?;
                if language.lang.is_empty()
                    || language.name.is_empty()
                    || language.size.is_some_and(|size| size <= 16 || size > i64::MAX as u64)
                    || !valid_hex(&language.md5, 32)
                    || !languages.insert(language.lang.as_str())
                    || region
                        .base
                        .size
                        .checked_add(language.download_bytes)
                        .is_none_or(|n| n > i64::MAX as u64)
                {
                    return Err("Invalid or duplicate StreetZim language version".into());
                }
                recipe_url(index, &language.recipe)?;
            }
        }
        Ok(())
    }
}

impl Source {
    pub(super) fn validate(&self, control: &Control) -> Result<Intervals, String> {
        if self.file.is_empty()
            || self.size == 0
            || self.size > i64::MAX as u64
            || self.chunk == 0
            || self.chunk > MAX_CHUNK
            || (self.size - 1) / self.chunk + 1 != self.sha256.len() as u64
            || Uuid::parse_str(&self.uuid).is_err()
        {
            return Err("This map download is damaged. Start the download again".into());
        }
        for hash in &self.sha256 {
            control.check()?;
            if !valid_hex(hash, 64) {
                return Err("Invalid source SHA-256".into());
            }
        }
        let mut masks = Vec::new();
        for mask in self.mask.iter().flatten() {
            control.check()?;
            if mask.len() != 2 || mask[0] > self.size || mask[1] > self.size - mask[0] {
                return Err("Invalid source mask bounds".into());
            }
            masks.push((mask[0], mask[0] + mask[1]));
        }
        Intervals::new(masks, control)
    }
}

impl Recipe {
    pub(super) fn fingerprint(&self, base_url: &str, proxy: &Url) -> Result<String, String> {
        let packs: std::collections::BTreeMap<_, _> = self.packs.iter().collect();
        let mut hash = Sha256::new();
        serde_json::to_writer(crate::integrity::Sha256Writer(&mut hash), &(
            &self.format, &self.uuid, &self.name, &self.file_name, self.size,
            &self.md5, &self.file_md5, &self.base, packs, &self.segments, base_url, proxy.as_str(),
        )).map_err(|error| error.to_string())?;
        Ok(hex::encode(hash.finalize()))
    }
    pub(super) fn output_stem(&self) -> &str {
        self.file_name.as_deref().unwrap_or(&self.name)
    }

    pub(super) fn source(&self, key: &str) -> Result<&Source, String> {
        if key == "base" {
            Ok(&self.base)
        } else {
            self.packs
                .get(key.strip_prefix("pack:").ok_or("Invalid source key")?)
                .ok_or_else(|| "Unknown source pack".into())
        }
    }

    pub(super) fn validate(&self, control: &Control) -> Result<HashMap<String, Intervals>, String> {
        let stem = self.output_stem();
        if self.format != "streetzim-recipe/2"
            || Uuid::parse_str(&self.uuid).is_err()
            || !valid_hex(&self.md5, 32)
            || self.file_md5.as_ref().is_some_and(|s| !valid_hex(s, 32))
            || self.size <= 16
            || self.size > i64::MAX as u64
            || self.segments.is_empty()
            || self.segments.last().map(|s| s.src.as_str()) != Some("md5")
            || stem.is_empty()
            || stem.starts_with('.')
            || stem.ends_with(['.', ' '])
            || stem.len() + 4 > 240
            || stem
                .chars()
                .any(|c| c.is_control() || "/\\:<>\"|?*".contains(c))
        {
            return Err("Invalid StreetZim recipe or output filename".into());
        }
        let mut masks = HashMap::new();
        masks.insert("base".into(), self.base.validate(control)?);
        for (key, source) in &self.packs {
            control.check()?;
            masks.insert(format!("pack:{key}"), source.validate(control)?);
        }
        let mut total = 0u64;
        for (index, segment) in self.segments.iter().enumerate() {
            control.check()?;
            let length = match segment.src.as_str() {
                "md5" if index + 1 == self.segments.len() => 16,
                "inline" => {
                    let bytes = STANDARD
                        .decode(segment.data.as_deref().ok_or("Missing inline data")?)
                        .map_err(|_| "Invalid inline base64")?;
                    if segment.len != Some(bytes.len() as u64)
                        || segment.sha256.as_deref()
                            != Some(hex::encode(Sha256::digest(&bytes)).as_str())
                    {
                        return Err("Inline data SHA-256 or length mismatch".into());
                    }
                    bytes.len() as u64
                }
                "base" | "pack" => {
                    let key = segment.source_key()?;
                    let source = self.source(&key)?;
                    let off = segment.off.ok_or("Missing source offset")?;
                    let len = segment.len.ok_or("Missing source length")?;
                    if off > source.size
                        || len > source.size - off
                        || masks
                            .get(&key)
                            .ok_or("Missing source masks")?
                            .overlaps(off, off + len)
                    {
                        return Err("Source segment exceeds source or copies masked bytes".into());
                    }
                    len
                }
                _ => return Err("Unknown segment or non-final MD5 segment".into()),
            };
            if length > self.size - total {
                return Err("Recipe segment size overflow".into());
            }
            total += length;
        }
        if total != self.size {
            return Err("Recipe output size mismatch".into());
        }
        Ok(masks)
    }

    pub(super) fn matches_catalog(&self, region: &MapRegion, version: &MapLanguage) -> bool {
        self.name == version.name
            && version.size.is_none_or(|size| self.size == size)
            && self.md5 == version.md5
            && self.base.size == region.base.size
            && self.base.file == region.base.file
    }

    /// Mirror the composer's ordered chunk reads, single-chunk working set and
    /// bounded FIFO cache. Counting source file sizes or unique chunks alone
    /// underestimates repeated reads after eviction; masked bytes still transfer.
    /// Inline recipe data and the generated MD5 do not transfer as source payload.
    pub(super) fn download_estimate<'a>(&'a self, reusable_base: bool, control: &Control) -> Result<u64, String> {
        // Bound expansion before simulating any chunk reads. Recipe validation
        // has already checked source offsets/lengths; this pass remains O(JSON).
        let mut reads = 1u64; // UUID header
        for segment in &self.segments {
            control.check()?;
            let source = match segment.src.as_str() {
                "base" => &self.base,
                "pack" => self.packs.get(segment.pack.as_deref().ok_or("Missing pack key")?)
                    .ok_or("Unknown source pack")?,
                _ => continue,
            };
            let off = segment.off.ok_or("Missing source offset")?;
            let len = segment.len.ok_or("Missing source length")?;
            if len > 0 {
                reads = reads.checked_add((off + len - 1) / source.chunk - off / source.chunk + 1)
                    .ok_or("Map recipe is too complex to estimate download size safely")?;
                if reads > MAX_ESTIMATE_CHUNK_READS {
                    return Err("Map recipe is too complex to estimate download size safely".into());
                }
            }
        }
        let mut last: Option<(&str, u64)> = None;
        let mut cache = std::collections::VecDeque::<(&str, u64)>::new();
        let mut cached = HashSet::<(&str, u64)>::new();
        let mut cached_bytes = 0u64;
        let mut total = 0u64;
        let mut read = |source: &'a Source, index: u64, base: bool| -> Result<(), String> {
            control.check()?;
            let len = source.chunk.min(source.size - index * source.chunk);
            let hash = source.sha256[index as usize].as_str();
            let chunk = (hash, len);
            if last == Some(chunk) {
                return Ok(());
            }
            last = Some(chunk);
            if cached.contains(&chunk) || (reusable_base && base) {
                return Ok(());
            }
            total = total.checked_add(len).ok_or("Map transfer estimate overflow")?;
            if len <= MAX_CACHE_BYTES {
                while cached_bytes + len > MAX_CACHE_BYTES
                    || cache.len() >= crate::map_chunk_cache::MAX_ENTRIES
                {
                    if let Some(chunk) = cache.pop_front() {
                        cached.remove(&chunk);
                        cached_bytes -= chunk.1;
                    }
                }
                cache.push_back(chunk);
                cached.insert(chunk);
                cached_bytes += len;
            }
            Ok(())
        };
        // UUID verification always reads the base header before composition.
        read(&self.base, 0, true)?;
        for segment in &self.segments {
            control.check()?;
            if !matches!(segment.src.as_str(), "base" | "pack") {
                continue;
            }
            let source = if segment.src == "base" {
                &self.base
            } else {
                self.packs.get(segment.pack.as_deref().ok_or("Missing pack key")?)
                    .ok_or("Unknown source pack")?
            };
            let off = segment.off.ok_or("Missing source offset")?;
            let len = segment.len.ok_or("Missing source length")?;
            if len > 0 {
                for index in off / source.chunk..=(off + len - 1) / source.chunk {
                    read(source, index, segment.src == "base")?;
                }
            }
        }
        Ok(total)
    }
}

impl Segment {
    pub(super) fn source_key(&self) -> Result<String, String> {
        match self.src.as_str() {
            "base" => Ok("base".into()),
            "pack" => Ok(format!(
                "pack:{}",
                self.pack.as_deref().ok_or("Missing pack ID")?
            )),
            _ => Err("Segment is not a source range".into()),
        }
    }
}
