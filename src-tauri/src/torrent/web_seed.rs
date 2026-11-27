// SPDX-License-Identifier: GPL-3.0-or-later
#![forbid(unsafe_code)]
//! BEP 19 HTTP web seeds for torrents whose swarm is empty.
//!
//! The torrent engine has no web-seed support. This adapter downloads the
//! advertised files into the same confined staging files the engine will hash
//! and verify. It never writes outside `ConfinedFactory`, never trusts the
//! seed's length or contents, and checks every torrent piece itself. Pieces it
//! could not obtain are left for the engine, whose own check decides what the
//! staged files really hold.
use super::*;
use futures_util::StreamExt;
use sha1::{Digest, Sha1};
use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom, Write};
use url::Url;

const MAX_BENCODE_DEPTH: usize = 32;
const MAX_SEED_REDIRECTS: usize = 5;
const SEED_TIMEOUT: Duration = Duration::from_secs(90);
/// Consecutive passes that add nothing before the engine continues with peers.
const SEED_STALLED_PASSES: u32 = 6;
/// Delay before the next pass; it doubles per stalled pass, sixteenfold at most.
#[cfg(not(test))]
const SEED_BACKOFF: Duration = Duration::from_secs(1);
#[cfg(test)]
const SEED_BACKOFF: Duration = Duration::from_millis(10);

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
    /// May clear on its own: a lost connection, a timeout, a busy mirror.
    Transport(String),
    /// Asking again cannot help: local I/O, or an answer no retry changes.
    Fatal(String),
    /// Every mirror delivered its bytes and torrent pieces still fail.
    Verify(String),
}

impl From<String> for SeedError {
    fn from(error: String) -> Self {
        Self::Fatal(error)
    }
}

impl SeedError {
    fn message(self) -> String {
        match self {
            Self::Transport(error) | Self::Fatal(error) | Self::Verify(error) => error,
        }
    }

    fn status(status: reqwest::StatusCode) -> Self {
        let message = format!("Web seed returned HTTP {status}");
        if status == reqwest::StatusCode::REQUEST_TIMEOUT
            || status == reqwest::StatusCode::TOO_MANY_REQUESTS
            || status.is_server_error()
        {
            Self::Transport(message)
        } else {
            Self::Fatal(message)
        }
    }

    /// The failure one pass reports after several mirrors: a transient one
    /// wins, because it is the only kind another pass can outlast.
    fn after(earlier: Option<Self>, error: Self) -> Self {
        match (earlier, error) {
            (Some(Self::Transport(kept)), Self::Fatal(_)) => Self::Transport(kept),
            (_, error) => error,
        }
    }
}

/// What one `download` call has established about the staged files. It is
/// carried across passes so a retry neither hashes nor fetches again what an
/// earlier pass already settled.
struct SeedProgress {
    /// Pieces that failed the full hash pass and have not verified since.
    /// `None` until every file has its full length and that pass has run.
    missing: Option<Vec<usize>>,
    /// Highest front-to-back byte count reached while files were still short.
    sequential: u64,
    /// Whether the current pass raised `sequential` or shrank `missing`.
    advanced: bool,
}

/// Download every advertised file into the confined staging factory and verify
/// all torrent pieces. The engine still hashes the same bytes before it can
/// publish, so this is an acquisition shortcut, not a verification substitute.
///
/// Nothing a pass finds on disk is discarded for failing verification. A file
/// already at its full length was sized by the engine or by partial reuse and
/// can hold valid pieces among unwritten ones, so only the byte ranges of the
/// pieces that fail are fetched again, in place. On failure the files are left
/// as they are for the engine's own check.
pub(super) async fn download(
    token: &CancellationToken,
    factory: &ConfinedFactory,
    seed: &WebSeedTorrent,
    progress: &(dyn Fn(u64, u64, f64) + Sync),
) -> Result<(), String> {
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
    let mut state = SeedProgress { missing: None, sequential: 0, advanced: false };
    let mut stalled = 0u32;
    loop {
        state.advanced = false;
        let error = match download_once(&client, token, factory, seed, progress, &mut state).await {
            Ok(()) => return Ok(()),
            Err(_) if token.is_cancelled() => return Ok(()),
            Err(SeedError::Transport(error)) => error,
            Err(error) => return Err(error.message()),
        };
        // Only a pass that added nothing counts against the limit, and every
        // other pass raised a high-water mark that the torrent size bounds. A
        // long transfer over an unreliable link is therefore neither abandoned
        // after a few drops nor able to loop without end.
        stalled = if state.advanced { 0 } else { stalled + 1 };
        if stalled >= SEED_STALLED_PASSES {
            return Err(error);
        }
        let delay = SEED_BACKOFF * (1 << stalled.saturating_sub(1).min(4));
        tokio::select! {
            _ = token.cancelled() => return Ok(()),
            _ = tokio::time::sleep(delay) => {}
        }
    }
}

async fn download_once(
    client: &reqwest::Client,
    token: &CancellationToken,
    factory: &ConfinedFactory,
    seed: &WebSeedTorrent,
    progress: &(dyn Fn(u64, u64, f64) + Sync),
    state: &mut SeedProgress,
) -> Result<(), SeedError> {
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
        if current > file.length {
            handle.set_len(0).map_err(display)?;
            current = 0;
        }
        if current < file.length {
            // A file shorter than the torrent says was only ever written front
            // to back by this adapter: the engine and partial reuse both give a
            // file its full length before writing into it. Its length is
            // therefore the offset to continue from.
            state.sequential = state.sequential.max(completed.saturating_add(current));
            let mut failure = None;
            for base in &seed.urls {
                if token.is_cancelled() {
                    return Ok(());
                }
                let url = match seed_url(base, seed, file) {
                    Ok(url) => url,
                    Err(error) => {
                        failure = Some(SeedError::after(failure, SeedError::Fatal(error)));
                        continue;
                    }
                };
                let result = download_file(
                    client,
                    token,
                    &url,
                    &handle,
                    current,
                    file.length,
                    completed,
                    seed.total,
                    progress,
                )
                .await;
                // Whatever this mirror did, the next request continues from
                // the bytes now on disk instead of repeating them.
                current = handle.metadata().map_err(display)?.len();
                let reached = completed.saturating_add(current.min(file.length));
                if reached > state.sequential {
                    state.sequential = reached;
                    state.advanced = true;
                }
                let error = match result {
                    Ok(()) if current >= file.length => {
                        failure = None;
                        break;
                    }
                    Ok(()) => SeedError::Transport("Web seed returned a short file".into()),
                    Err(error) => error,
                };
                failure = Some(SeedError::after(failure, error));
            }
            if current < file.length {
                return Err(failure.unwrap_or_else(|| {
                    SeedError::Fatal("Web seed did not provide the complete file".into())
                }));
            }
            handle.set_len(file.length).map_err(display)?;
            handle.sync_all().map_err(display)?;
            progress(completed.saturating_add(file.length), seed.total, 0.0);
        }
        completed = completed.saturating_add(file.length);
        files.push(Some(handle));
    }
    if token.is_cancelled() {
        return Ok(());
    }
    let mut missing = match state.missing.take() {
        Some(missing) => missing,
        None => match unverified_pieces(seed, &files, None, token).await {
            Ok(missing) => missing,
            Err(_) if token.is_cancelled() => return Ok(()),
            Err(error) => return Err(SeedError::Fatal(error)),
        },
    };
    let result = repair(client, token, seed, &files, &mut missing, &mut state.advanced, progress).await;
    state.missing = Some(missing);
    result?;
    if token.is_cancelled() {
        return Ok(());
    }
    progress(seed.total, seed.total, 0.0);
    Ok(())
}

/// Fetch the byte ranges of `missing` pieces in place until every piece
/// verifies. Bytes outside those ranges are never written, so a piece that
/// already verified cannot be damaged by a mirror serving something else.
async fn repair(
    client: &reqwest::Client,
    token: &CancellationToken,
    seed: &WebSeedTorrent,
    files: &[Option<Arc<File>>],
    missing: &mut Vec<usize>,
    advanced: &mut bool,
    progress: &(dyn Fn(u64, u64, f64) + Sync),
) -> Result<(), SeedError> {
    if missing.is_empty() {
        return Ok(());
    }
    let mut failure = None;
    for base in &seed.urls {
        if token.is_cancelled() {
            return Ok(());
        }
        let verified = seed.total.saturating_sub(piece_bytes(seed, missing));
        let mut written = 0u64;
        let mut outcome = Ok(());
        for (index, start, end) in missing_ranges(seed, missing) {
            let handle = files
                .get(index)
                .and_then(Option::as_ref)
                .ok_or_else(|| "Web seed file handle is missing".to_string())?;
            let file = &seed.files[index];
            outcome = match seed_url(base, seed, file) {
                Ok(url) => {
                    fetch_range(
                        client,
                        token,
                        &url,
                        handle,
                        start..end,
                        file.length,
                        verified,
                        seed.total,
                        progress,
                        &mut written,
                    )
                    .await
                }
                Err(error) => Err(SeedError::Fatal(error)),
            };
            if outcome.is_err() || token.is_cancelled() {
                break;
            }
        }
        if token.is_cancelled() {
            return Ok(());
        }
        if written > 0 {
            // Trust the hashes, not the mirror: only pieces that verify now
            // leave the list, including after a response that failed part way.
            let before = missing.len();
            *missing = match unverified_pieces(seed, files, Some(missing.clone()), token).await {
                Ok(missing) => missing,
                Err(_) if token.is_cancelled() => return Ok(()),
                Err(error) => return Err(SeedError::Fatal(error)),
            };
            *advanced |= missing.len() < before;
            progress(seed.total.saturating_sub(piece_bytes(seed, missing)), seed.total, 0.0);
        }
        if missing.is_empty() {
            for handle in files.iter().flatten() {
                handle.sync_all().map_err(display)?;
            }
            return Ok(());
        }
        if let Err(error) = outcome {
            failure = Some(SeedError::after(failure, error));
        }
    }
    Err(failure.unwrap_or_else(|| {
        SeedError::Verify(format!(
            "Web seed content failed verification at piece {}",
            missing[0]
        ))
    }))
}

/// Cumulative end offset of every file within the torrent's piece space.
fn file_ends(seed: &WebSeedTorrent) -> Vec<u64> {
    let mut end = 0u64;
    seed.files
        .iter()
        .map(|file| {
            end = end.saturating_add(file.length);
            end
        })
        .collect()
}

fn piece_span(seed: &WebSeedTorrent, piece: usize) -> (u64, u64) {
    let start = (piece as u64).saturating_mul(seed.piece_length);
    (start.min(seed.total), start.saturating_add(seed.piece_length).min(seed.total))
}

fn piece_bytes(seed: &WebSeedTorrent, pieces: &[usize]) -> u64 {
    pieces
        .iter()
        .map(|piece| {
            let (start, end) = piece_span(seed, *piece);
            end - start
        })
        .sum()
}

/// The parts of `[start, end)` in torrent offsets that lie in real files, as
/// `(file index, start, end)` in that file's own offsets.
fn file_ranges(seed: &WebSeedTorrent, ends: &[u64], start: u64, end: u64) -> Vec<(usize, u64, u64)> {
    let mut ranges = Vec::new();
    let mut index = ends.partition_point(|file_end| *file_end <= start);
    while index < seed.files.len() {
        let file = &seed.files[index];
        let file_start = ends[index] - file.length;
        if file_start >= end {
            break;
        }
        let from = start.max(file_start);
        let to = end.min(ends[index]);
        if from < to && !file.padding {
            ranges.push((index, from - file_start, to - file_start));
        }
        index += 1;
    }
    ranges
}

/// The file byte ranges that ascending `missing` pieces cover, with adjacent
/// pieces merged into one range per file.
fn missing_ranges(seed: &WebSeedTorrent, missing: &[usize]) -> Vec<(usize, u64, u64)> {
    let ends = file_ends(seed);
    let mut ranges = Vec::new();
    let mut pieces = missing.iter().copied().peekable();
    while let Some(first) = pieces.next() {
        let mut last = first;
        while pieces.peek() == Some(&(last + 1)) {
            last += 1;
            pieces.next();
        }
        let (start, _) = piece_span(seed, first);
        let (_, end) = piece_span(seed, last);
        ranges.extend(file_ranges(seed, &ends, start, end));
    }
    ranges
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
) -> Result<(), SeedError> {
    let mut request = client
        .get(url.clone())
        .header(reqwest::header::ACCEPT_ENCODING, "identity");
    if current > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={current}-"));
    }
    let response = tokio::select! {
        _ = token.cancelled() => return Ok(()),
        result = request.send() => result.map_err(|error| SeedError::Transport(display(error)))?,
    };
    let status = response.status();
    let mut write_offset = current;
    if status == reqwest::StatusCode::PARTIAL_CONTENT {
        if content_range_start(&response) != Some(current) {
            return Err(SeedError::Fatal("Web seed returned an unexpected byte range".into()));
        }
    } else if status == reqwest::StatusCode::OK {
        if current > 0 {
            file.set_len(0).map_err(display)?;
            write_offset = 0;
        }
    } else if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE && current == length {
        return Ok(());
    } else {
        return Err(SeedError::status(status));
    }
    let mut writer = file;
    writer
        .seek(SeekFrom::Start(write_offset))
        .map_err(display)?;
    if let Some(content_length) = response.content_length() {
        if content_length > length.saturating_sub(write_offset) {
            return Err(SeedError::Fatal("Web seed response is larger than the torrent file".into()));
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
        let chunk = chunk.map_err(|error| SeedError::Transport(display(error)))?;
        if offset.saturating_add(chunk.len() as u64) > length {
            return Err(SeedError::Fatal("Web seed response exceeded the torrent file length".into()));
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

fn content_range_start(response: &reqwest::Response) -> Option<u64> {
    response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("bytes "))
        .and_then(|value| value.split('-').next())
        .and_then(|value| value.parse::<u64>().ok())
}

/// Write exactly `range` of one staged file from a mirror, in place. A mirror
/// without range support answers with the whole file; only the requested
/// bytes are taken from it, so nothing outside `range` is ever overwritten.
#[allow(clippy::too_many_arguments)]
async fn fetch_range(
    client: &reqwest::Client,
    token: &CancellationToken,
    url: &Url,
    file: &File,
    range: std::ops::Range<u64>,
    length: u64,
    verified: u64,
    total: u64,
    progress: &(dyn Fn(u64, u64, f64) + Sync),
    written: &mut u64,
) -> Result<(), SeedError> {
    let request = client
        .get(url.clone())
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .header(reqwest::header::RANGE, format!("bytes={}-{}", range.start, range.end - 1));
    let response = tokio::select! {
        _ = token.cancelled() => return Ok(()),
        result = request.send() => result.map_err(|error| SeedError::Transport(display(error)))?,
    };
    let status = response.status();
    // File offset of the next response byte.
    let mut at = if status == reqwest::StatusCode::PARTIAL_CONTENT {
        if content_range_start(&response) != Some(range.start) {
            return Err(SeedError::Fatal("Web seed returned an unexpected byte range".into()));
        }
        range.start
    } else if status == reqwest::StatusCode::OK {
        if response.content_length().is_some_and(|content_length| content_length != length) {
            return Err(SeedError::Fatal("Web seed file is not the size the torrent describes".into()));
        }
        0
    } else {
        return Err(SeedError::status(status));
    };
    let mut writer = file;
    writer
        .seek(SeekFrom::Start(range.start))
        .map_err(display)?;
    let mut stream = response.bytes_stream();
    // File offset of the next byte this call still has to write.
    let mut next = range.start;
    let mut sample = *written;
    let mut last_progress = Instant::now();
    progress(verified.saturating_add(*written).min(total), total, 0.0);
    while next < range.end {
        let chunk = tokio::select! {
            _ = token.cancelled() => return Ok(()),
            chunk = stream.next() => chunk,
        };
        let Some(chunk) = chunk else {
            return Err(SeedError::Transport("Web seed returned a short file".into()));
        };
        let chunk = chunk.map_err(|error| SeedError::Transport(display(error)))?;
        let chunk_end = at.saturating_add(chunk.len() as u64);
        if chunk_end > next {
            let skip = (next - at) as usize;
            let take = (chunk_end.min(range.end) - next) as usize;
            writer.write_all(&chunk[skip..skip + take]).map_err(display)?;
            next += take as u64;
            *written += take as u64;
        }
        at = chunk_end;
        let now = Instant::now();
        let elapsed = now.duration_since(last_progress);
        if elapsed >= Duration::from_millis(250) {
            let speed = (*written - sample) as f64 / elapsed.as_secs_f64();
            progress(verified.saturating_add(*written).min(total), total, speed);
            sample = *written;
            last_progress = now;
        }
    }
    Ok(())
}

/// Hash `candidates` (every piece when `None`) off the async runtime and
/// return, in ascending order, those that do not match the torrent.
async fn unverified_pieces(
    seed: &WebSeedTorrent,
    files: &[Option<Arc<File>>],
    candidates: Option<Vec<usize>>,
    token: &CancellationToken,
) -> Result<Vec<usize>, String> {
    let seed = seed.clone();
    let files = files.to_vec();
    let token = token.clone();
    tokio::task::spawn_blocking(move || match candidates {
        Some(candidates) => failed_pieces(&seed, &files, candidates.into_iter(), &token),
        None => failed_pieces(&seed, &files, 0..seed.pieces.len(), &token),
    })
    .await
    .map_err(display)?
}

fn failed_pieces(
    seed: &WebSeedTorrent,
    files: &[Option<Arc<File>>],
    candidates: impl Iterator<Item = usize>,
    token: &CancellationToken,
) -> Result<Vec<usize>, String> {
    for (index, file) in seed.files.iter().enumerate() {
        if file.padding {
            continue;
        }
        let handle = files
            .get(index)
            .and_then(Option::as_ref)
            .ok_or("Web seed file handle is missing")?;
        let actual = handle.metadata().map_err(display)?.len();
        if actual != file.length {
            return Err(format!(
                "Web seed file length mismatch: expected {}, got {actual}",
                file.length
            ));
        }
    }
    let ends = file_ends(seed);
    let mut buffer = vec![0u8; seed.piece_length.min(seed.total) as usize];
    let mut failed = Vec::new();
    for piece in candidates {
        if token.is_cancelled() {
            return Err("cancelled".into());
        }
        let expected = seed
            .pieces
            .get(piece)
            .ok_or("Torrent piece count is inconsistent")?;
        let (start, end) = piece_span(seed, piece);
        let bytes = &mut buffer[..(end - start) as usize];
        // Padding files hold no bytes on disk and count as zeros.
        bytes.fill(0);
        for (index, from, to) in file_ranges(seed, &ends, start, end) {
            let handle = files
                .get(index)
                .and_then(Option::as_ref)
                .ok_or("Web seed file handle is missing")?;
            let file_start = ends[index] - seed.files[index].length;
            let mut reader = &**handle;
            reader.seek(SeekFrom::Start(from)).map_err(display)?;
            reader
                .read_exact(&mut bytes[(file_start + from - start) as usize..(file_start + to - start) as usize])
                .map_err(|error| format!("Reading web seed file for verification failed: {error}"))?;
        }
        if Sha1::digest(&*bytes)[..] != expected[..] {
            failed.push(piece);
        }
    }
    Ok(failed)
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
        // verified first, then have every failing piece acquired again.
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

    const BODY: &[u8] = b"0123456789abcdef";

    /// A single-file torrent of `BODY` in five-byte pieces with these mirrors.
    fn body_seed(ports: &[u16]) -> WebSeedTorrent {
        let hashes: Vec<u8> = BODY.chunks(5).flat_map(|chunk| Sha1::digest(chunk).to_vec()).collect();
        let info = dict(&[
            (b"length", bint(BODY.len() as u64)),
            (b"name", bstr(b"map.zim")),
            (b"piece length", bint(5)),
            (b"pieces", bstr(&hashes)),
        ]);
        let mut urls = b"l".to_vec();
        for port in ports {
            urls.extend(bstr(format!("http://127.0.0.1:{port}/map.zim").as_bytes()));
        }
        urls.push(b'e');
        parse(&dict(&[(b"info", info), (b"url-list", urls)])).unwrap().unwrap()
    }

    /// A staging directory for `body_seed`, holding `staged` when it is given.
    fn body_staging(staged: Option<&[u8]>) -> (PathBuf, ConfinedFactory) {
        let temp = std::env::temp_dir().join(format!("kiwix-webseed-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&temp).unwrap();
        if let Some(staged) = staged {
            std::fs::write(temp.join("map.zim"), staged).unwrap();
        }
        let root = cap_std::fs::Dir::open_ambient_dir(&temp, cap_std::ambient_authority()).unwrap();
        let factory = ConfinedFactory {
            root: Arc::new(root),
            path: temp.clone(),
            expected: Arc::new(HashMap::from([(PathBuf::from("map.zim"), BODY.len() as u64)])),
            sources: Arc::new(Mutex::new(HashMap::new())),
            existing: staged.is_some(),
            stop: CancellationToken::new(),
            gate: Arc::new(std::sync::RwLock::new(())),
            dirs: Arc::new(Mutex::new(HashSet::new())),
            owner: None,
        };
        (temp, factory)
    }

    /// A loopback mirror answering connection `index` with `respond(index,
    /// request head in lower case)`. Returns its port and the heads it saw.
    fn mirror(respond: impl Fn(usize, &str) -> Vec<u8> + Send + 'static) -> (u16, Arc<Mutex<Vec<String>>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();
        std::thread::spawn(move || {
            for (index, stream) in listener.incoming().enumerate() {
                let Ok(mut stream) = stream else { return };
                let mut request = Vec::new();
                let mut byte = [0u8; 1];
                while !request.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                    request.push(byte[0]);
                }
                let request = String::from_utf8_lossy(&request).to_ascii_lowercase();
                seen.lock().unwrap().push(request.clone());
                let _ = stream.write_all(&respond(index, &request));
            }
        });
        (port, requests)
    }

    fn requested_range(request: &str) -> Option<String> {
        request.lines().find_map(|line| line.strip_prefix("range: bytes=")).map(str::to_owned)
    }

    /// An honest answer for `content`: 206 for a range request, 200 otherwise.
    /// Only `sent` body bytes follow the full-length header when it is given,
    /// which the client sees as a connection lost part way.
    fn answer(content: &[u8], request: &str, sent: Option<usize>) -> Vec<u8> {
        let (status, start, end) = match requested_range(request) {
            Some(range) => {
                let (start, end) = range.split_once('-').unwrap();
                let start: usize = start.parse().unwrap();
                let end = end.parse().map_or(content.len(), |end: usize| end + 1);
                (format!("206 Partial Content\r\nContent-Range: bytes {start}-{}/{}", end - 1, content.len()), start, end)
            }
            None => ("200 OK".to_owned(), 0, content.len()),
        };
        let payload = &content[start..end];
        let mut response = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", payload.len()).into_bytes();
        response.extend_from_slice(&payload[..sent.unwrap_or(payload.len()).min(payload.len())]);
        response
    }

    /// Runs `download` and returns its result with every reported byte count.
    async fn run_seed(token: &CancellationToken, factory: &ConfinedFactory, seed: &WebSeedTorrent) -> (Result<(), String>, Vec<u64>) {
        let reported = Mutex::new(Vec::new());
        let result = download(token, factory, seed, &|done, total, _| {
            assert_eq!(total, BODY.len() as u64);
            reported.lock().unwrap().push(done);
        }).await;
        (result, reported.into_inner().unwrap())
    }

    #[tokio::test]
    async fn full_length_file_keeps_its_valid_pieces_and_fetches_only_the_others() {
        // The shape the engine leaves behind: full length, with the second and
        // the last piece never written.
        let mut staged = BODY.to_vec();
        staged[5..10].fill(0);
        staged[15..].fill(0);
        let (port, requests) = mirror(|_, request| answer(BODY, request, None));
        let (temp, factory) = body_staging(Some(&staged));
        let (result, reported) = run_seed(&CancellationToken::new(), &factory, &body_seed(&[port])).await;
        result.unwrap();
        assert_eq!(std::fs::read(temp.join("map.zim")).unwrap(), BODY);
        let ranges: Vec<_> = requests.lock().unwrap().iter().map(|request| requested_range(request)).collect();
        assert_eq!(ranges, [Some("5-9".to_owned()), Some("15-15".to_owned())]);
        // Progress starts from the ten bytes that were already valid.
        assert_eq!(reported.first(), Some(&10));
        assert!(reported.is_sorted(), "{reported:?}");
        assert_eq!(reported.last(), Some(&16));
        let _ = std::fs::remove_dir_all(temp);
    }

    #[tokio::test]
    async fn failed_web_seed_leaves_a_full_length_file_as_it_was() {
        let mut staged = BODY.to_vec();
        staged[10..].fill(0);
        let (port, requests) = mirror(|_, _| b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec());
        let (temp, factory) = body_staging(Some(&staged));
        let (result, _) = run_seed(&CancellationToken::new(), &factory, &body_seed(&[port])).await;
        assert!(result.unwrap_err().contains("503"));
        // The valid leading pieces stay for the engine's own check.
        assert_eq!(std::fs::read(temp.join("map.zim")).unwrap(), staged);
        // A busy mirror is retried, but only a bounded number of times.
        assert_eq!(requests.lock().unwrap().len(), SEED_STALLED_PASSES as usize);
        let _ = std::fs::remove_dir_all(temp);
    }

    #[tokio::test]
    async fn repair_never_overwrites_a_piece_that_already_verified() {
        // This mirror ignores ranges and its copy differs in the first piece,
        // which the staged file already holds correctly.
        let mut served = BODY.to_vec();
        served[..5].fill(b'X');
        let mut staged = BODY.to_vec();
        staged[5..].fill(0);
        let (port, requests) = mirror(move |_, _| answer(&served, "", None));
        let (temp, factory) = body_staging(Some(&staged));
        let (result, _) = run_seed(&CancellationToken::new(), &factory, &body_seed(&[port])).await;
        result.unwrap();
        assert_eq!(std::fs::read(temp.join("map.zim")).unwrap(), BODY);
        assert_eq!(requests.lock().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(temp);
    }

    #[tokio::test]
    async fn the_next_mirror_continues_from_the_bytes_a_dropped_one_wrote() {
        let (dropped, _) = mirror(|_, request| answer(BODY, request, Some(6)));
        let (port, requests) = mirror(|_, request| answer(BODY, request, None));
        let (temp, factory) = body_staging(None);
        let (result, reported) = run_seed(&CancellationToken::new(), &factory, &body_seed(&[dropped, port])).await;
        result.unwrap();
        assert_eq!(std::fs::read(temp.join("map.zim")).unwrap(), BODY);
        let ranges: Vec<_> = requests.lock().unwrap().iter().map(|request| requested_range(request)).collect();
        assert_eq!(ranges, [Some("6-".to_owned())]);
        assert!(reported.is_sorted(), "{reported:?}");
        let _ = std::fs::remove_dir_all(temp);
    }

    #[tokio::test]
    async fn drops_that_still_add_bytes_do_not_use_up_the_retry_limit() {
        // More drops than the stall limit allows, each after three new bytes.
        let drops = SEED_STALLED_PASSES as usize - 1;
        assert!(drops * 3 < BODY.len());
        let (port, requests) = mirror(move |index, request| answer(BODY, request, (index < drops).then_some(3)));
        let (temp, factory) = body_staging(None);
        let (result, reported) = run_seed(&CancellationToken::new(), &factory, &body_seed(&[port])).await;
        result.unwrap();
        assert_eq!(std::fs::read(temp.join("map.zim")).unwrap(), BODY);
        let ranges: Vec<_> = requests.lock().unwrap().iter().map(|request| requested_range(request)).collect();
        let expected: Vec<_> = (0..=drops).map(|index| (index > 0).then(|| format!("{}-", index * 3))).collect();
        assert_eq!(ranges, expected);
        assert!(reported.is_sorted(), "{reported:?}");
        let _ = std::fs::remove_dir_all(temp);
    }

    #[tokio::test]
    async fn an_unreachable_web_seed_fails_after_its_bounded_retries_and_stops_when_cancelled() {
        // A port nothing listens on: every pass fails without adding a byte.
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let seed = body_seed(&[port]);
        let (temp, factory) = body_staging(None);
        let (result, reported) = run_seed(&CancellationToken::new(), &factory, &seed).await;
        assert!(result.is_err());
        assert!(reported.is_empty(), "{reported:?}");
        // A pause or cancel during the wait between passes ends the stage at
        // once, as a stop rather than as a web-seed failure.
        let token = CancellationToken::new();
        let stop = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(SEED_BACKOFF / 2).await;
            stop.cancel();
        });
        let (result, _) = run_seed(&token, &factory, &seed).await;
        assert_eq!(result, Ok(()));
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
