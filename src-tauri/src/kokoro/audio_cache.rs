// SPDX-License-Identifier: GPL-3.0-or-later
//! An owned receipt binds one article/rate/voice key to a private PCM file.
//! Existing files without that receipt are never served, overwritten or evicted.
use super::*;
use crate::integrity::ContentIdentity;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    key: String,
    parent: Id,
    file: String,
    bytes: u64,
    sha256: String,
    stamp: ContentIdentity,
}

fn key(name: &OsStr) -> Result<&str> {
    let key = name
        .to_str()
        .and_then(|n| n.strip_suffix(".wav"))
        .ok_or_else(changed)?;
    if key.len() != 64
        || !key
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(changed());
    }
    Ok(key)
}

fn receipt_name(key: &str) -> String {
    format!("{key}.receipt")
}

impl Receipt {
    fn read(dir: &Dir, key: &str) -> Result<(Self, File)> {
        let marker = file(dir, receipt_name(key).as_ref(), false)?;
        let mut bytes = Vec::new();
        (&marker).take(2049).read_to_end(&mut bytes)?;
        if bytes.len() > 2048 {
            return Err(changed());
        }
        let row: Self = serde_json::from_slice(&bytes)?;
        let uuid = row.file.strip_suffix(".wav").ok_or_else(changed)?;
        if uuid::Uuid::parse_str(uuid)
            .map_err(|_| changed())?
            .to_string()
            != uuid
            || row.key != key
            || row.parent != id(&dir.dir_metadata()?)
            || row.bytes < 44
            || row.bytes > u32::MAX as u64
            || row.bytes % 2 != 0
        {
            return Err(changed());
        }
        Ok((row, marker))
    }

    fn opened(&self, dir: &Dir) -> Result<File> {
        let mut audio = file(dir, self.file.as_ref(), false)?;
        let stamp =
            |f: &File| ContentIdentity::from_handles(std::iter::once(f)).map_err(io::Error::other);
        if stamp(&audio)? != self.stamp || audio.metadata()?.len() != self.bytes {
            return Err(changed());
        }
        let mut hash = chksum_hash_sha2_256::new();
        let mut remaining = self.bytes;
        let mut buffer = vec![0; 64 * 1024];
        while remaining != 0 {
            let n = remaining.min(buffer.len() as u64) as usize;
            audio.read_exact(&mut buffer[..n])?;
            hash.update(&buffer[..n]);
            remaining -= n as u64;
        }
        if hash.digest().to_hex_lowercase() != self.sha256 || stamp(&audio)? != self.stamp {
            return Err(changed());
        }
        audio.rewind()?;
        Ok(audio)
    }
}

pub(in super::super) fn cached_audio(path: &Path, name: &OsStr) -> Result<Option<File>> {
    let dir = match open_root(path, false) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        r => r?,
    };
    let key = key(name)?;
    match Receipt::read(&dir, key).and_then(|(row, _)| row.opened(&dir)) {
        Ok(file) => Ok(Some(file)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

pub(in super::super) fn publish_audio(path: &Path, name: &Path, wav: &[u8]) -> Result<File> {
    let lease = Lease::open(path, true)?;
    let dir = &lease.parent;
    let key = key(name.as_os_str())?;
    match Receipt::read(dir, key) {
        Ok((row, marker)) => match row.opened(dir) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                lease.unchanged()?;
                remove_owned_file(
                    dir,
                    receipt_name(key).as_ref(),
                    id(&Metadata::from_file(&marker)?),
                )?;
            }
            result => return result,
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    // The file has a fresh name and is invisible to lookup until its owned
    // receipt has been published. Death before the receipt can leave an orphan.
    let name = format!("{}.wav", uuid::Uuid::new_v4());
    let mut audio = file(dir, name.as_ref(), true)?;
    let identity = id(&Metadata::from_file(&audio)?);
    let result = (|| {
        audio.write_all(wav)?;
        audio.sync_data()?;
        let row = Receipt {
            key: key.to_owned(),
            parent: id(&dir.dir_metadata()?),
            file: name.clone(),
            bytes: wav.len() as u64,
            sha256: chksum_hash_sha2_256::hash(wav).to_hex_lowercase(),
            stamp: ContentIdentity::from_handles(std::iter::once(&audio))
                .map_err(io::Error::other)?,
        };
        lease.unchanged()?;
        publish_bytes(dir, receipt_name(key).as_ref(), &serde_json::to_vec(&row)?)?;
        audio.rewind()?;
        audio.try_clone()
    })();
    drop(audio);
    if result.is_err() && !dir.try_exists(receipt_name(key))? {
        remove_owned_file(dir, name.as_ref(), identity)?;
    }
    result
}

pub(in super::super) fn clean_cache(path: &Path, remove_all: bool) -> Result<()> {
    let lease = match Lease::open(path, false) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        r => r?,
    };
    let dir = &lease.parent;
    let mut known = Vec::new();
    let mut total = 0u64;
    for entry in dir.entries()? {
        let name = entry?.file_name();
        let Some(key) = name.to_str().and_then(|n| n.strip_suffix(".receipt")) else {
            continue;
        };
        if key.len() != 64 {
            continue;
        }
        let (row, marker) = match Receipt::read(dir, key) {
            Ok(pair) => pair,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::InvalidData | io::ErrorKind::NotFound
                ) =>
            {
                continue
            }
            Err(e) => return Err(e),
        };
        let m = Metadata::from_file(&marker)?;
        total = total.saturating_add(row.bytes);
        known.push((m.modified()?, id(&m), row));
    }
    if !remove_all && total <= super::super::CACHE_BUDGET {
        return Ok(());
    }
    known.sort_unstable_by_key(|(t, _, _)| *t);
    for (_, marker_id, row) in known {
        if !remove_all && total <= super::super::CACHE_BUDGET / 2 {
            break;
        }
        let audio = match row.opened(dir) {
            Ok(audio) => audio,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                lease.unchanged()?;
                remove_owned_file(dir, receipt_name(&row.key).as_ref(), marker_id)?;
                total = total.saturating_sub(row.bytes);
                continue;
            }
            Err(e) if e.kind() == io::ErrorKind::InvalidData => continue,
            Err(e) => return Err(e),
        };
        let identity = id(&Metadata::from_file(&audio)?);
        drop(audio);
        lease.unchanged()?;
        remove_owned_file(dir, row.file.as_ref(), identity)?;
        remove_owned_file(dir, receipt_name(&row.key).as_ref(), marker_id)?;
        total = total.saturating_sub(row.bytes);
    }
    if remove_all
        && dir.entries()?.try_fold(false, |present, e| {
            Ok::<_, io::Error>(present || e?.file_name() != LOCK)
        })?
    {
        let parent = path.parent().ok_or_else(changed)?;
        mark(&open_root(parent, false)?, "voice-audio-preserved-v1")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::super::fixtures;
    use std::fs;

    #[test]
    fn missing_owned_audio_can_be_regenerated() {
        let profile = fixtures::profile().unwrap();
        let models = profile.path().canonicalize().unwrap().join("models");
        crate::fs_ext::create_private_dir_all(&models).unwrap();
        let cache = models.join("voice-audio-v2");
        let key = "f".repeat(64);
        let name = format!("{key}.wav");
        let bytes = super::super::super::wav_bytes(&[0.25], 24000);
        drop(publish_audio(&cache, name.as_ref(), &bytes).unwrap());
        let dir = open_root(&cache, false).unwrap();
        let (row, _) = Receipt::read(&dir, &key).unwrap();
        fs::remove_file(cache.join(row.file)).unwrap();
        fs::write(cache.join("other"), b"keep").unwrap();
        assert!(cached_audio(&cache, name.as_ref()).unwrap().is_none());
        drop(publish_audio(&cache, name.as_ref(), &bytes).unwrap());
        assert!(cached_audio(&cache, name.as_ref()).unwrap().is_some());
        assert_eq!(fs::read(cache.join("other")).unwrap(), b"keep");
    }

    #[test]
    fn retained_audio_and_receipts_preserve_even_a_valid_replacement_wav() {
        let profile = fixtures::profile().unwrap();
        let models = profile.path().canonicalize().unwrap().join("models");
        crate::fs_ext::create_private_dir_all(&models).unwrap();
        let cache = models.join("voice-audio-v2");
        let key = "a".repeat(64);
        let name = format!("{key}.wav");
        let expected = super::super::super::wav_bytes(&[0.25, 0.5], 24000);
        drop(publish_audio(&cache, name.as_ref(), &expected).unwrap());
        let mut retained = cached_audio(&cache, name.as_ref()).unwrap().unwrap();
        let dir = open_root(&cache, false).unwrap();
        let (row, _) = Receipt::read(&dir, &key).unwrap();
        let source = cache.join(&row.file);
        fs::rename(&source, cache.join("saved.wav")).unwrap();
        let other = super::super::super::wav_bytes(&[0.5, 0.75], 24000);
        fixtures::write_private(&source, &other).unwrap();
        assert!(cached_audio(&cache, name.as_ref()).is_err());
        let mut bytes = Vec::new();
        retained.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, expected);
        clean_cache(&cache, true).unwrap();
        assert!(preserved_legacy(&models).1);
        assert_eq!(fs::read(&source).unwrap(), other);
        assert!(cache.join(receipt_name(&key)).is_file());
        assert!(cache.join("saved.wav").is_file());
    }
}
