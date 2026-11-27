// SPDX-License-Identifier: GPL-3.0-or-later
#![forbid(unsafe_code)]
//! BEP 19 HTTP web seeds for torrents whose swarm is empty.
//!
//! The torrent engine has no web-seed support. This adapter downloads the
//! advertised files into the same confined staging files the engine will hash
//! and verify. It never writes outside `ConfinedFactory`, never trusts the
//! seed's length or contents, and verifies every torrent piece before handing
//! the job to the engine.
use super::*;
use futures_util::StreamExt;
use sha1::{Digest, Sha1};
use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom, Write};
use url::Url;

const MAX_BENCODE_DEPTH: usize = 32;
const MAX_SEED_REDIRECTS: usize = 5;
const SEED_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Debug, Clone)]
pub(super) struct WebSeedTorrent {
    pub(super) name: String,
    pub(super) piece_length: u64,
    pub(super) pieces: Vec<[u8; 20]>,
    pub(super) files: Vec<WebSeedFile>,
    pub(super) total: u64,
    pub(super) single_file: bool,
    pub(super) urls: Vec<String>,
}

#[derive(Debug, Clone)]
pub(super) struct WebSeedFile {
    pub(super) path: PathBuf,
    pub(super) length: u64,
    pub(super) padding: bool,
}

#[derive(Debug, Clone)]
enum BValue {
    Int(i64),
    Bytes(Vec<u8>),
    List(Vec<BValue>),
    Dict(BTreeMap<Vec<u8>, BValue>),
}

struct Bencode<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Bencode<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, at: 0 }
    }

    fn value(&mut self, depth: usize) -> Result<BValue, String> {
        if depth > MAX_BENCODE_DEPTH {
            return Err("Torrent metadata is nested too deeply".into());
        }
        match self.data.get(self.at).copied() {
            Some(b'i') => self.integer(),
            Some(b'l') => self.list(depth),
            Some(b'd') => self.dict(depth),
            Some(b'0'..=b'9') => self.bytes().map(BValue::Bytes),
            Some(_) => Err("Torrent metadata is not valid bencode".into()),
            None => Err("Torrent metadata ended early".into()),
        }
    }

    fn integer(&mut self) -> Result<BValue, String> {
        self.at += 1;
        let start = self.at;
        while self.data.get(self.at).is_some_and(|byte| *byte != b'e') {
            self.at += 1;
        }
        if self.data.get(self.at) != Some(&b'e') || self.at == start {
            return Err("Torrent metadata has an invalid integer".into());
        }
        let value = std::str::from_utf8(&self.data[start..self.at])
            .ok()
            .and_then(|value| value.parse::<i64>().ok())
            .ok_or("Torrent metadata has an invalid integer")?;
        self.at += 1;
        Ok(BValue::Int(value))
    }

    fn bytes(&mut self) -> Result<Vec<u8>, String> {
        let start = self.at;
        while self.data.get(self.at).is_some_and(u8::is_ascii_digit) {
            self.at += 1;
        }
        if self.data.get(self.at) != Some(&b':') || self.at == start {
            return Err("Torrent metadata has an invalid byte string".into());
        }
        let length = std::str::from_utf8(&self.data[start..self.at])
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .ok_or("Torrent metadata has an invalid byte string")?;
        self.at += 1;
        let end = self
            .at
            .checked_add(length)
            .ok_or("Torrent metadata length overflow")?;
        if end > self.data.len() {
            return Err("Torrent metadata ended inside a byte string".into());
        }
        let bytes = self.data[self.at..end].to_vec();
        self.at = end;
        Ok(bytes)
    }

    fn list(&mut self, depth: usize) -> Result<BValue, String> {
        self.at += 1;
        let mut values = Vec::new();
        loop {
            match self.data.get(self.at) {
                Some(b'e') => {
                    self.at += 1;
                    return Ok(BValue::List(values));
                }
                Some(_) => values.push(self.value(depth + 1)?),
                None => return Err("Torrent metadata ended inside a list".into()),
            }
        }
    }

    fn dict(&mut self, depth: usize) -> Result<BValue, String> {
        self.at += 1;
        let mut values = BTreeMap::new();
        loop {
            match self.data.get(self.at) {
                Some(b'e') => {
                    self.at += 1;
                    return Ok(BValue::Dict(values));
                }
                Some(_) => {
                    let key = self.bytes()?;
                    let value = self.value(depth + 1)?;
                    if values.insert(key, value).is_some() {
                        return Err("Torrent metadata has duplicate dictionary keys".into());
                    }
                }
                None => return Err("Torrent metadata ended inside a dictionary".into()),
            }
        }
    }
}

fn dict_get<'a>(dict: &'a BTreeMap<Vec<u8>, BValue>, key: &[u8]) -> Option<&'a BValue> {
    dict.get(key)
}

fn as_dict(value: Option<&BValue>) -> Result<&BTreeMap<Vec<u8>, BValue>, String> {
    match value {
        Some(BValue::Dict(dict)) => Ok(dict),
        _ => Err("Torrent metadata is missing a dictionary".into()),
    }
}

fn as_bytes(value: Option<&BValue>) -> Result<&[u8], String> {
    match value {
        Some(BValue::Bytes(bytes)) => Ok(bytes),
        _ => Err("Torrent metadata is missing a byte string".into()),
    }
}

fn as_int(value: Option<&BValue>) -> Result<i64, String> {
    match value {
        Some(BValue::Int(value)) => Ok(*value),
        _ => Err("Torrent metadata is missing an integer".into()),
    }
}

fn as_string(value: Option<&BValue>, label: &str) -> Result<String, String> {
    String::from_utf8(as_bytes(value)?.to_vec())
        .map_err(|_| format!("Torrent {label} is not valid UTF-8"))
}

fn as_urls(value: Option<&BValue>) -> Result<Vec<String>, String> {
    let values = match value {
        Some(BValue::Bytes(bytes)) => vec![bytes.clone()],
        Some(BValue::List(items)) => {
            let mut result = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    BValue::Bytes(bytes) => result.push(bytes.clone()),
                    _ => return Err("Torrent url-list contains a non-string".into()),
                }
            }
            result
        }
        _ => return Err("Torrent url-list has an invalid shape".into()),
    };
    values
        .into_iter()
        .map(|value| {
            String::from_utf8(value).map_err(|_| "Torrent web seed URL is not valid UTF-8".into())
        })
        .collect()
}

fn web_file_path(parts: &[BValue]) -> Result<PathBuf, String> {
    if parts.is_empty() {
        return Err("Torrent file has an empty path".into());
    }
    let mut path = PathBuf::new();
    for part in parts {
        let part = match part {
            BValue::Bytes(bytes) => String::from_utf8(bytes.clone())
                .map_err(|_| "Torrent file path is not valid UTF-8")?,
            _ => return Err("Torrent file path contains a non-string".into()),
        };
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.contains('/')
            || part.contains('\\')
        {
            return Err("Torrent file path is not a safe relative path".into());
        }
        path.push(part);
    }
    safe_relative(&path).map_err(display)?;
    Ok(path)
}

/// Parse only the fields needed for a BEP 19 HTTP web seed. A torrent without
/// `url-list` returns `Ok(None)` and follows the normal peer path.
pub(super) fn parse(torrent_bytes: &[u8]) -> Result<Option<WebSeedTorrent>, String> {
    let mut parser = Bencode::new(torrent_bytes);
    let root = parser.value(0)?;
    if parser.at != torrent_bytes.len() {
        return Err("Torrent metadata has trailing bytes".into());
    }
    let root = as_dict(Some(&root))?;
    let Some(url_list) = dict_get(root, b"url-list") else {
        return Ok(None);
    };
    let urls = as_urls(Some(url_list))?;
    let info = as_dict(dict_get(root, b"info"))?;
    let name = as_string(dict_get(info, b"name"), "name")?;
    let piece_length = as_int(dict_get(info, b"piece length"))?;
    if piece_length <= 0 || piece_length > 64 * 1024 * 1024 {
        return Err("Torrent piece length is out of range".into());
    }
    let pieces = as_bytes(dict_get(info, b"pieces"))?;
    if pieces.is_empty() || !pieces.len().is_multiple_of(20) {
        return Err("Torrent piece hashes are invalid".into());
    }
    let pieces: Vec<[u8; 20]> = pieces.as_chunks::<20>().0.to_vec();

    let mut files = Vec::new();
    let mut total = 0u64;
    let single_file = dict_get(info, b"length").is_some();
    if let Some(length) = dict_get(info, b"length") {
        let length = as_int(Some(length))?;
        if length < 0 {
            return Err("Torrent file length is negative".into());
        }
        let length = length as u64;
        let path = PathBuf::from(&name);
        safe_relative(&path).map_err(display)?;
        files.push(WebSeedFile {
            path,
            length,
            padding: false,
        });
        total = length;
    } else {
        let entries = match dict_get(info, b"files") {
            Some(BValue::List(entries)) => entries,
            _ => return Err("Torrent metadata is missing its file list".into()),
        };
        for entry in entries {
            let entry = as_dict(Some(entry))?;
            let length = as_int(dict_get(entry, b"length"))?;
            if length < 0 {
                return Err("Torrent file length is negative".into());
            }
            let length = length as u64;
            let path = match dict_get(entry, b"path") {
                Some(BValue::List(parts)) => web_file_path(parts)?,
                _ => return Err("Torrent file is missing its path".into()),
            };
            let padding = dict_get(entry, b"attr")
                .and_then(|value| value_as_bytes(value).ok())
                .is_some_and(|attrs| attrs.contains(&b'p'));
            files.push(WebSeedFile {
                path,
                length,
                padding,
            });
            total = total.checked_add(length).ok_or("Torrent size overflow")?;
        }
    }
    if files.is_empty() {
        return Err("Torrent has no files".into());
    }
    let piece_count = total.saturating_add(piece_length as u64 - 1) / piece_length as u64;
    if piece_count as usize != pieces.len() {
        return Err("Torrent piece count does not match its size".into());
    }
    Ok(Some(WebSeedTorrent {
        name,
        piece_length: piece_length as u64,
        pieces,
        files,
        total,
        single_file,
        urls,
    }))
}

fn value_as_bytes(value: &BValue) -> Result<&[u8], String> {
    match value {
        BValue::Bytes(bytes) => Ok(bytes),
        _ => Err("Torrent attribute is not a byte string".into()),
    }
}

fn validate_seed_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(display)?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err("Web seed URLs cannot contain credentials".into());
    }
    match url.scheme() {
        "https" if url.host().is_some() => Ok(url),
        "http"
            if url
                .host_str()
                .and_then(|host| {
                    host.trim_matches(['[', ']'])
                        .parse::<std::net::IpAddr>()
                        .ok()
                })
                .is_some_and(|ip| ip.is_loopback()) =>
        {
            Ok(url)
        }
        _ => Err(
            "Web seed URLs require HTTPS (literal loopback HTTP is allowed for local testing)"
                .into(),
        ),
    }
}

fn seed_url(base: &str, seed: &WebSeedTorrent, file: &WebSeedFile) -> Result<Url, String> {
    let mut url = validate_seed_url(base)?;
    if seed.single_file && !url.path().ends_with('/') {
        return Ok(url);
    }
    {
        let trailing = url.path().ends_with('/');
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| "Web seed URL cannot be a base")?;
        if trailing {
            segments.pop_if_empty();
        }
        if !seed.single_file {
            segments.push(&seed.name);
        }
        for component in file.path.components() {
            match component {
                std::path::Component::Normal(part) => {
                    segments.push(part.to_str().ok_or("Web seed file path is not UTF-8")?);
                }
                _ => return Err("Web seed file path is not relative".into()),
            }
        }
    }
    Ok(url)
}

#[derive(Debug)]
enum SeedError {
    Transport(String),
    Verify(String),
}

impl From<String> for SeedError {
    fn from(error: String) -> Self {
        Self::Transport(error)
    }
}

impl SeedError {
    fn message(self) -> String {
        match self {
            Self::Transport(error) | Self::Verify(error) => error,
        }
    }
}

/// Download every advertised file into the confined staging factory and verify
/// all torrent pieces. The engine still hashes the same bytes before it can
/// publish, so this is an acquisition shortcut, not a verification substitute.
pub(super) async fn download(
    token: &CancellationToken,
    factory: &ConfinedFactory,
    seed: &WebSeedTorrent,
    progress: &(dyn Fn(u64, u64, f64) + Sync),
) -> Result<(), String> {
    let mut force_redownload = false;
    for attempt in 0..3 {
        match download_once(token, factory, seed, progress, force_redownload).await {
            Ok(()) => return Ok(()),
            Err(_) if token.is_cancelled() => return Ok(()),
            Err(SeedError::Verify(_)) if attempt < 2 && !force_redownload => {
                // The bytes looked complete but failed piece verification.
                // Discard them and acquire once from scratch.
                reset_files(factory, seed)?;
                progress(0, seed.total, 0.0);
                force_redownload = true;
            }
            Err(SeedError::Transport(_)) if attempt < 2 => {
                // Keep partial files so the next attempt can use Range
                // resume instead of re-downloading from byte zero.
                force_redownload = false;
            }
            Err(error) => return Err(error.message()),
        }
    }
    unreachable!("web seed download attempts are bounded")
}

fn reset_files(factory: &ConfinedFactory, seed: &WebSeedTorrent) -> Result<(), String> {
    for file in &seed.files {
        if file.padding {
            continue;
        }
        let handle = factory
            .open_file(&file.path, file.length)
            .map_err(display)?;
        handle.set_len(0).map_err(display)?;
        handle.sync_all().map_err(display)?;
    }
    Ok(())
}

async fn download_once(
    token: &CancellationToken,
    factory: &ConfinedFactory,
    seed: &WebSeedTorrent,
    progress: &(dyn Fn(u64, u64, f64) + Sync),
    force_redownload: bool,
) -> Result<(), SeedError> {
    let client = reqwest::Client::builder()
        .connect_timeout(SEED_TIMEOUT)
        .read_timeout(SEED_TIMEOUT)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= MAX_SEED_REDIRECTS {
                return attempt.error("Too many web seed redirects");
            }
            if validate_seed_url(attempt.url().as_str()).is_ok() {
                attempt.follow()
            } else {
                attempt.error("Insecure web seed redirect rejected")
            }
        }))
        .build()
        .map_err(display)?;
    let mut files: Vec<Option<Arc<File>>> = Vec::with_capacity(seed.files.len());
    let mut completed = 0u64;
    for file in &seed.files {
        if token.is_cancelled() {
            return Ok(());
        }
        if file.padding {
            files.push(None);
            completed = completed.saturating_add(file.length);
            continue;
        }
        let handle = factory
            .open_file(&file.path, file.length)
            .map_err(display)?;
        let mut current = handle.metadata().map_err(display)?.len();
        if current > file.length || (force_redownload && current == file.length) {
            handle.set_len(0).map_err(display)?;
            current = 0;
        }
        if current < file.length {
            let mut last_error = None;
            for base in &seed.urls {
                if token.is_cancelled() {
                    return Ok(());
                }
                let url = match seed_url(base, seed, file) {
                    Ok(url) => url,
                    Err(error) => {
                        last_error = Some(error);
                        continue;
                    }
                };
                match download_file(
                    &client,
                    token,
                    &url,
                    &handle,
                    current,
                    file.length,
                    completed,
                    seed.total,
                    progress,
                )
                .await
                {
                    Ok(_) => {
                        current = handle.metadata().map_err(display)?.len();
                        if current >= file.length {
                            last_error = None;
                            break;
                        }
                        last_error = Some("Web seed returned a short file".into());
                    }
                    Err(error) => last_error = Some(error),
                }
            }
            if current < file.length {
                return Err(SeedError::Transport(last_error.unwrap_or_else(|| {
                    "Web seed did not provide the complete file".into()
                })));
            }
        }
        handle.set_len(file.length).map_err(display)?;
        handle.sync_all().map_err(display)?;
        completed = completed.saturating_add(file.length);
        progress(completed, seed.total, 0.0);
        files.push(Some(handle));
    }
    if token.is_cancelled() {
        return Ok(());
    }
    let verify_seed = seed.clone();
    let verify_files = files.clone();
    let verify_token = token.clone();
    let result = tokio::task::spawn_blocking(move || {
        verify_pieces(&verify_seed, &verify_files, &verify_token)
    })
    .await
    .map_err(display)?;
    if token.is_cancelled() {
        return Ok(());
    }
    result.map_err(SeedError::Verify)?;
    progress(seed.total, seed.total, 0.0);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn download_file(
    client: &reqwest::Client,
    token: &CancellationToken,
    url: &Url,
    file: &File,
    current: u64,
    length: u64,
    completed: u64,
    total: u64,
    progress: &(dyn Fn(u64, u64, f64) + Sync),
) -> Result<(), String> {
    let mut request = client
        .get(url.clone())
        .header(reqwest::header::ACCEPT_ENCODING, "identity");
    if current > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={current}-"));
    }
    let response = tokio::select! {
        _ = token.cancelled() => return Ok(()),
        result = request.send() => result.map_err(display)?,
    };
    let status = response.status();
    let mut write_offset = current;
    if status == reqwest::StatusCode::PARTIAL_CONTENT {
        let start = response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("bytes "))
            .and_then(|value| value.split('-').next())
            .and_then(|value| value.parse::<u64>().ok());
        if start != Some(current) {
            return Err("Web seed returned an unexpected byte range".into());
        }
    } else if status == reqwest::StatusCode::OK {
        if current > 0 {
            file.set_len(0).map_err(display)?;
            write_offset = 0;
        }
    } else if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE && current == length {
        return Ok(());
    } else {
        return Err(format!("Web seed returned HTTP {status}"));
    }
    let mut writer = file;
    writer
        .seek(SeekFrom::Start(write_offset))
        .map_err(display)?;
    if let Some(content_length) = response.content_length() {
        if content_length > length.saturating_sub(write_offset) {
            return Err("Web seed response is larger than the torrent file".into());
        }
    }
    let mut stream = response.bytes_stream();
    let mut offset = write_offset;
    let mut sample_offset = offset;
    let mut last_progress = Instant::now();
    progress(completed.saturating_add(offset), total, 0.0);
    loop {
        let chunk = tokio::select! {
            _ = token.cancelled() => return Ok(()),
            chunk = stream.next() => chunk,
        };
        let Some(chunk) = chunk else {
            break;
        };
        let chunk = chunk.map_err(display)?;
        if offset.saturating_add(chunk.len() as u64) > length {
            return Err("Web seed response exceeded the torrent file length".into());
        }
        writer.write_all(&chunk).map_err(display)?;
        offset = offset.saturating_add(chunk.len() as u64);
        let now = Instant::now();
        let elapsed = now.duration_since(last_progress);
        if elapsed >= Duration::from_millis(250) {
            let speed = (offset - sample_offset) as f64 / elapsed.as_secs_f64();
            progress(completed.saturating_add(offset), total, speed);
            sample_offset = offset;
            last_progress = now;
        }
    }
    Ok(())
}

fn verify_pieces(
    seed: &WebSeedTorrent,
    files: &[Option<Arc<File>>],
    token: &CancellationToken,
) -> Result<(), String> {
    let mut buffer = Vec::with_capacity(seed.piece_length as usize);
    let mut piece = 0usize;
    let feed = |bytes: &[u8], buffer: &mut Vec<u8>, piece: &mut usize| -> Result<(), String> {
        if token.is_cancelled() {
            return Err("cancelled".into());
        }
        let mut at = 0usize;
        while at < bytes.len() {
            let take = (seed.piece_length as usize - buffer.len()).min(bytes.len() - at);
            buffer.extend_from_slice(&bytes[at..at + take]);
            at += take;
            if buffer.len() == seed.piece_length as usize {
                if &Sha1::digest(&buffer)[..]
                    != seed
                        .pieces
                        .get(*piece)
                        .ok_or("Torrent piece count is inconsistent")?
                {
                    return Err(format!(
                        "Web seed content failed verification at piece {piece}"
                    ));
                }
                *piece += 1;
                buffer.clear();
            }
        }
        Ok(())
    };
    for (index, file) in seed.files.iter().enumerate() {
        if token.is_cancelled() {
            return Err("cancelled".into());
        }
        let handle = files.get(index).and_then(Option::as_ref);
        if file.padding {
            let zeros = vec![0u8; 64 * 1024];
            let mut remaining = file.length;
            while remaining > 0 {
                let take = remaining.min(zeros.len() as u64) as usize;
                feed(&zeros[..take], &mut buffer, &mut piece)?;
                remaining -= take as u64;
            }
            continue;
        }
        let handle = handle.ok_or("Web seed file handle is missing")?;
        let actual = handle.metadata().map_err(display)?.len();
        if actual != file.length {
            return Err(format!(
                "Web seed file length mismatch: expected {}, got {actual}",
                file.length
            ));
        }
        let mut remaining = file.length;
        let mut chunk = vec![0u8; 64 * 1024];
        while remaining > 0 {
            let take = remaining.min(chunk.len() as u64) as usize;
            let mut reader = &**handle;
            reader
                .seek(SeekFrom::Start(file.length - remaining))
                .map_err(display)?;
            reader.read_exact(&mut chunk[..take]).map_err(|error| {
                format!("Reading web seed file for verification failed: {error}")
            })?;
            feed(&chunk[..take], &mut buffer, &mut piece)?;
            remaining -= take as u64;
        }
    }
    if !buffer.is_empty() {
        if &Sha1::digest(&buffer)[..]
            != seed
                .pieces
                .get(piece)
                .ok_or("Torrent piece count is inconsistent")?
        {
            return Err(format!(
                "Web seed content failed verification at piece {piece}"
            ));
        }
        piece += 1;
    }
    if piece != seed.pieces.len() {
        return Err("Web seed content did not cover every torrent piece".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bstr(value: &[u8]) -> Vec<u8> {
        let mut out = format!("{}:", value.len()).into_bytes();
        out.extend_from_slice(value);
        out
    }

    fn bint(value: u64) -> Vec<u8> {
        format!("i{value}e").into_bytes()
    }

    fn dict(entries: &[(&[u8], Vec<u8>)]) -> Vec<u8> {
        let mut out = b"d".to_vec();
        for (key, value) in entries {
            out.extend(bstr(key));
            out.extend(value);
        }
        out.push(b'e');
        out
    }

    fn single_torrent(url: Vec<u8>) -> Vec<u8> {
        let mut info = dict(&[
            (b"length", bint(5)),
            (b"name", bstr(b"map.zim")),
            (b"piece length", bint(4)),
            (b"pieces", bstr(&[0u8; 40])),
        ]);
        info.truncate(info.len() - 1);
        info.push(b'e');
        let mut root = dict(&[(b"info", info), (b"url-list", url)]);
        root.truncate(root.len() - 1);
        root.push(b'e');
        root
    }

    #[test]
    fn parses_single_file_web_seed() {
        let torrent = single_torrent(bstr(b"https://example.test/map.zim"));
        let seed = parse(&torrent).unwrap().unwrap();
        assert_eq!(seed.name, "map.zim");
        assert_eq!(seed.total, 5);
        assert_eq!(seed.files.len(), 1);
        assert!(seed.single_file);
        assert_eq!(seed.urls, vec!["https://example.test/map.zim"]);
    }

    #[test]
    fn rejects_unsafe_and_incomplete_metadata() {
        let torrent = single_torrent(bstr(b"http://example.test/map.zim"));
        let seed = parse(&torrent).unwrap().unwrap();
        assert!(seed_url(&seed.urls[0], &seed, &seed.files[0]).is_err());
        let mut bad = torrent;
        bad.pop();
        assert!(parse(&bad).is_err());
    }

    #[tokio::test]
    async fn downloads_single_file_and_verifies_pieces() {
        let body = b"0123456789abcdef".to_vec();
        let piece_length = 5usize;
        let hashes: Vec<u8> = body
            .chunks(piece_length)
            .flat_map(|chunk| Sha1::digest(chunk).to_vec())
            .collect();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let expected_body = body.clone();
        let server = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 2048];
            let _ = stream.read(&mut request);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                expected_body.len()
            )
            .unwrap();
            stream.write_all(&expected_body).unwrap();
        });
        let info = dict(&[
            (b"length", bint(body.len() as u64)),
            (b"name", bstr(b"map.zim")),
            (b"piece length", bint(piece_length as u64)),
            (b"pieces", bstr(&hashes)),
        ]);
        let root = dict(&[
            (b"info", info),
            (b"url-list", {
                let mut out = b"l".to_vec();
                out.extend(bstr(format!("http://127.0.0.1:{port}/map.zim").as_bytes()));
                out.push(b'e');
                out
            }),
        ]);
        let seed = parse(&root).unwrap().unwrap();
        let temp = std::env::temp_dir().join(format!("kiwix-webseed-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&temp).unwrap();
        let root_dir =
            cap_std::fs::Dir::open_ambient_dir(&temp, cap_std::ambient_authority()).unwrap();
        // A preallocated full-length but corrupt staging file must be
        // verified before it is discarded, then re-acquired on failure.
        std::fs::write(temp.join("map.zim"), vec![0u8; body.len()]).unwrap();
        let factory = ConfinedFactory {
            root: Arc::new(root_dir),
            path: temp.clone(),
            expected: Arc::new(HashMap::from([(
                PathBuf::from("map.zim"),
                body.len() as u64,
            )])),
            sources: Arc::new(Mutex::new(HashMap::new())),
            existing: true,
            stop: CancellationToken::new(),
            gate: Arc::new(std::sync::RwLock::new(())),
            dirs: Arc::new(Mutex::new(HashSet::new())),
            owner: None,
        };
        download(&CancellationToken::new(), &factory, &seed, &|_, _, _| {})
            .await
            .unwrap();
        assert_eq!(std::fs::read(temp.join("map.zim")).unwrap(), body);
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(temp);
    }

    #[tokio::test]
    async fn web_seed_resume_reports_retained_bytes_and_only_new_bytes_in_rate() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let body = b"0123456789";
        for honors_range in [true, false] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = Url::parse(&format!("http://{}/map.zim", listener.local_addr().unwrap())).unwrap();
            let (release, reported) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    request.push(stream.read_u8().await.unwrap());
                }
                assert!(String::from_utf8(request).unwrap().to_ascii_lowercase().contains("\r\nrange: bytes=4-\r\n"));
                let (header, offset): (&[u8], _) = if honors_range {
                    (b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 4-9/10\r\nContent-Length: 6\r\n\r\n", 4)
                } else {
                    (b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\n", 0)
                };
                stream.write_all(header).await.unwrap();
                // Ensure the measured body interval starts after the baseline.
                reported.await.unwrap();
                tokio::time::sleep(Duration::from_millis(300)).await;
                stream.write_all(&body[offset..]).await.unwrap();
            });
            let path = std::env::temp_dir().join(format!("kiwix-webseed-resume-{}", Uuid::new_v4()));
            let mut file = std::fs::OpenOptions::new().read(true).write(true).create_new(true).open(&path).unwrap();
            file.write_all(&body[..4]).unwrap();
            let samples = Mutex::new(Vec::new());
            let release = Mutex::new(Some(release));
            let client = reqwest::Client::builder().timeout(Duration::from_secs(5)).build().unwrap();
            download_file(&client, &CancellationToken::new(), &url, &file, 4, 10, 5, 15, &|done, total, rate| {
                samples.lock().unwrap().push((done, total, rate));
                if let Some(release) = release.lock().unwrap().take() { release.send(()).unwrap(); }
            }).await.unwrap();
            server.await.unwrap();
            let samples = samples.into_inner().unwrap();
            assert_eq!(samples[0], (if honors_range { 9 } else { 5 }, 15, 0.0));
            let &(done, total, rate) = samples.last().unwrap();
            assert_eq!((done, total), (15, 15));
            let new_bytes = if honors_range { 6.0 } else { 10.0 };
            assert!(rate > 0.0 && rate <= new_bytes / 0.250, "impossible rate for new bytes: {rate}");
            assert_eq!(std::fs::read(&path).unwrap().as_slice(), body);
            drop(file);
            std::fs::remove_file(path).unwrap();
        }
    }

    #[tokio::test]
    async fn downloads_multi_file_torrent_from_web_seed() {
        let first = b"abc".to_vec();
        let second = b"defg".to_vec();
        let mut pieces = Vec::new();
        pieces.extend(Sha1::digest(b"abcd").to_vec());
        pieces.extend(Sha1::digest(b"efg").to_vec());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            use std::io::{Read, Write};
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 2048];
                let read = stream.read(&mut request).unwrap_or(0);
                let request = String::from_utf8_lossy(&request[..read]);
                let body = if request.contains("/base/maps/a.bin") {
                    b"abc".to_vec()
                } else if request.contains("/base/maps/b.bin") {
                    b"defg".to_vec()
                } else {
                    Vec::new()
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        let files = BValue::List(vec![
            BValue::Dict(BTreeMap::from([
                (b"length".to_vec(), BValue::Int(3)),
                (
                    b"path".to_vec(),
                    BValue::List(vec![BValue::Bytes(b"a.bin".to_vec())]),
                ),
            ])),
            BValue::Dict(BTreeMap::from([
                (b"length".to_vec(), BValue::Int(4)),
                (
                    b"path".to_vec(),
                    BValue::List(vec![BValue::Bytes(b"b.bin".to_vec())]),
                ),
            ])),
        ]);
        let mut info = BTreeMap::new();
        info.insert(b"files".to_vec(), files);
        info.insert(b"name".to_vec(), BValue::Bytes(b"maps".to_vec()));
        info.insert(b"piece length".to_vec(), BValue::Int(4));
        info.insert(b"pieces".to_vec(), BValue::Bytes(pieces));
        let root = BTreeMap::from([
            (b"info".to_vec(), BValue::Dict(info)),
            (
                b"url-list".to_vec(),
                BValue::Bytes(format!("http://127.0.0.1:{port}/base/").into_bytes()),
            ),
        ]);
        let mut encoded = b"d".to_vec();
        for (key, value) in root {
            encoded.extend(bstr(&key));
            encoded.extend(encode_test_value(&value));
        }
        encoded.push(b'e');
        let seed = parse(&encoded).unwrap().unwrap();
        let temp = std::env::temp_dir().join(format!("kiwix-webseed-multi-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&temp).unwrap();
        let root_dir =
            cap_std::fs::Dir::open_ambient_dir(&temp, cap_std::ambient_authority()).unwrap();
        let expected = seed
            .files
            .iter()
            .map(|file| (file.path.clone(), file.length))
            .collect();
        let factory = ConfinedFactory {
            root: Arc::new(root_dir),
            path: temp.clone(),
            expected: Arc::new(expected),
            sources: Arc::new(Mutex::new(HashMap::new())),
            existing: false,
            stop: CancellationToken::new(),
            gate: Arc::new(std::sync::RwLock::new(())),
            dirs: Arc::new(Mutex::new(HashSet::new())),
            owner: None,
        };
        download(&CancellationToken::new(), &factory, &seed, &|_, _, _| {})
            .await
            .unwrap();
        assert_eq!(std::fs::read(temp.join("a.bin")).unwrap(), first);
        assert_eq!(std::fs::read(temp.join("b.bin")).unwrap(), second);
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(temp);
    }

    #[test]
    fn multi_file_url_contains_torrent_root() {
        let files = BValue::List(vec![BValue::Dict(BTreeMap::from([
            (b"length".to_vec(), BValue::Int(3)),
            (
                b"path".to_vec(),
                BValue::List(vec![BValue::Bytes(b"a.zim".to_vec())]),
            ),
        ]))]);
        let mut info = BTreeMap::new();
        info.insert(b"files".to_vec(), files);
        info.insert(b"name".to_vec(), BValue::Bytes(b"maps".to_vec()));
        info.insert(b"piece length".to_vec(), BValue::Int(4));
        info.insert(b"pieces".to_vec(), BValue::Bytes(vec![0; 20]));
        let root = BTreeMap::from([
            (b"info".to_vec(), BValue::Dict(info)),
            (
                b"url-list".to_vec(),
                BValue::Bytes(b"https://example.test/base/".to_vec()),
            ),
        ]);
        let mut encoded = b"d".to_vec();
        for (key, value) in root {
            encoded.extend(bstr(&key));
            encoded.extend(encode_test_value(&value));
        }
        encoded.push(b'e');
        let seed = parse(&encoded).unwrap().unwrap();
        let url = seed_url(&seed.urls[0], &seed, &seed.files[0]).unwrap();
        assert_eq!(url.as_str(), "https://example.test/base/maps/a.zim");
        let url = seed_url("https://example.test/base", &seed, &seed.files[0]).unwrap();
        assert_eq!(url.as_str(), "https://example.test/base/maps/a.zim");
    }

    fn encode_test_value(value: &BValue) -> Vec<u8> {
        match value {
            BValue::Int(value) => bint(*value as u64),
            BValue::Bytes(bytes) => bstr(bytes),
            BValue::List(values) => {
                let mut out = b"l".to_vec();
                for value in values {
                    out.extend(encode_test_value(value));
                }
                out.push(b'e');
                out
            }
            BValue::Dict(values) => {
                let mut out = b"d".to_vec();
                for (key, value) in values {
                    out.extend(bstr(key));
                    out.extend(encode_test_value(value));
                }
                out.push(b'e');
                out
            }
        }
    }
}
