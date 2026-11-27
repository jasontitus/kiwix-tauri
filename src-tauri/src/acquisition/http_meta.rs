// SPDX-License-Identifier: GPL-3.0-or-later
//! Disk-space checks, response header/range validation and Metalink resolution.
#![forbid(unsafe_code)]
#[allow(unused_imports)]
use super::*;

/// Capacity probes are incremental: at most one per interval of newly written
/// bytes. A full-response restart must reset the checkpoint together with the
/// byte counter, or the discarded partial's length postpones the first probe.
pub(super) fn space_check_due(downloaded: u64, checked_at: u64) -> bool {
    downloaded.saturating_sub(checked_at) >= SPACE_CHECK_INTERVAL
}

/// Bytes the transfer still has to find free. `allocated` is the length the
/// part file was preallocated to (its `set_len` target). Filesystems that
/// allocate on `set_len` (NTFS) have already charged that whole region to this
/// transfer, so an incremental or resume check must not demand it a second
/// time. Filesystems that leave the tail sparse already passed the full
/// admission check before preallocation, so treating the reserved region as
/// free never weakens the first admission.
pub(super) fn space_remaining(total: u64, written: u64, allocated: u64) -> u64 {
    total.saturating_sub(written.max(allocated))
}

pub(super) fn check_space(part: &Path, remaining: u64) -> Result<(), String> {
    let directory = part
        .parent()
        .ok_or("Download has no destination directory")?;
    let free = crate::fs_ext::available_space(directory).map_err(|e| format!("Checking free space: {e}"))?;
    let needed = remaining
        .checked_add(DISK_RESERVE)
        .ok_or("Advertised download size overflows available-space calculation")?;
    if free < needed {
        return Err(format!(
            "Not enough disk space: need {needed} bytes, have {free}"
        ));
    }
    Ok(())
}

pub(super) fn header_text(headers: &reqwest::header::HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .filter(|s| s.len() <= 2048)
        .map(str::to_string)
}

pub(super) fn header_u64(headers: &reqwest::header::HeaderMap, name: &str) -> Result<Option<u64>, String> {
    headers
        .get(name)
        .map(|value| {
            value
                .to_str()
                .ok()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| format!("Invalid HTTP {name}"))
        })
        .transpose()
}

pub(super) fn validator(stored: &StoredJob) -> Option<&str> {
    stored
        .etag
        .as_deref()
        .filter(|s| !s.starts_with("W/") && s.starts_with('"') && s.ends_with('"'))
        .or_else(|| {
            stored
                .last_modified
                .as_deref()
                .filter(|s| crate::timestamps::parse_mail_date(s).is_ok())
        })
}

pub(super) fn validators_match(stored: &StoredJob, headers: &reqwest::header::HeaderMap) -> bool {
    if let Some(etag) = stored.etag.as_deref().filter(|s| !s.starts_with("W/")) {
        return header_text(headers, ETAG.as_str()).as_deref() == Some(etag);
    }
    if let Some(modified) = &stored.last_modified {
        return header_text(headers, LAST_MODIFIED.as_str()).as_ref() == Some(modified);
    }
    false
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct ByteRange {
    pub(super) start: u64,
    pub(super) end: u64,
    pub(super) total: u64,
}

pub(super) fn parse_range(value: &str) -> Option<ByteRange> {
    let rest = value.strip_prefix("bytes ")?;
    let (bounds, total) = rest.split_once('/')?;
    let (start, end) = bounds.split_once('-')?;
    let start = start.parse::<u64>().ok()?;
    let end = end.parse::<u64>().ok()?;
    let total = total.parse::<u64>().ok()?;
    if start > end || end >= total {
        return None;
    }
    Some(ByteRange { start, end, total })
}

pub(super) fn unsatisfied_range(value: &str) -> Option<u64> {
    value.strip_prefix("bytes */")?.parse().ok()
}

pub(super) struct Metalink {
    pub(super) url: String,
    pub(super) size: u64,
    pub(super) sha256: Option<String>,
}

/// `after` is the mirror an earlier attempt used without keeping any bytes;
/// see [`parse_metalink`].
pub(super) async fn resolve_metalink(
    inner: &Inner,
    source: &Url,
    signal: &mut watch::Receiver<Signal>,
    after: Option<&str>,
) -> Result<Metalink, String> {
    crate::transfer_policy::require_internet()?;
    let response = inner
        .client()
        .await?
        .get(source.clone())
        .header(ACCEPT_ENCODING, "identity")
        .send()
        .await
        .map_err(|e| format!("Fetching Metalink: {e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Metalink server returned HTTP {}",
            response.status()
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > METALINK_LIMIT as u64)
    {
        return Err("Metalink response exceeds size limit".into());
    }
    let base = response.url().clone();
    let response = TransferResponse::Http(response);
    let stream = response
        .bytes_stream()
        .map(|result| result.map_err(std::io::Error::other));
    futures_util::pin_mut!(stream);
    let mut reader = StreamReader::new(stream);
    let mut bytes = Vec::new();
    let mut buffer = vec![0u8; READ_BUDGET];
    loop {
        let Some(permit) = bandwidth_permit(inner, signal).await? else {
            return Err("Metalink resolution stopped".into());
        };
        let count = reader
            .read(&mut buffer[..permit.bytes])
            .await
            .map_err(|e| format!("Reading Metalink: {e}"))?;
        refund_bandwidth(inner, &permit, count)?;
        if count == 0 {
            break;
        }
        if count > METALINK_LIMIT.saturating_sub(bytes.len()) {
            return Err("Metalink response exceeds size limit".into());
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    let xml = std::str::from_utf8(&bytes).map_err(|e| format!("Metalink is not UTF-8: {e}"))?;
    parse_metalink(xml, &base, after)
}

/// Chooses the eligible mirror with the best priority. When `after` is one of
/// the eligible mirrors, the one listed after it is chosen instead, wrapping
/// around, so repeated attempts work through the whole list.
pub(super) fn parse_metalink(xml: &str, base: &Url, after: Option<&str>) -> Result<Metalink, String> {
    const NS: &str = "urn:ietf:params:xml:ns:metalink";
    let doc =
        roxmltree::Document::parse(xml).map_err(|e| format!("Malformed Metalink XML: {e}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "metalink" || root.tag_name().namespace() != Some(NS) {
        return Err("Expected an IETF Metalink document".into());
    }
    let mut files = root.children().filter(|n| {
        n.is_element() && n.tag_name().namespace() == Some(NS) && n.tag_name().name() == "file"
    });
    let file = files.next().ok_or("Metalink has no file")?;
    if files.next().is_some() {
        return Err("Multi-file Metalinks are not supported".into());
    }
    safe_name(file.attribute("name").ok_or("Metalink file has no name")?)?;
    let child = |name: &str| {
        file.children().find(|n| {
            n.is_element() && n.tag_name().namespace() == Some(NS) && n.tag_name().name() == name
        })
    };
    let size = child("size")
        .and_then(|n| n.text())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|n| *n >= 80)
        .ok_or("Metalink has no valid file size")?;
    let sha256 = file
        .children()
        .find(|n| {
            n.is_element()
                && n.tag_name().namespace() == Some(NS)
                && n.tag_name().name() == "hash"
                && n.attribute("type")
                    .is_some_and(|v| v.eq_ignore_ascii_case("sha-256"))
        })
        .map(|n| {
            let hash = n.text().unwrap_or("").trim().to_ascii_lowercase();
            if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("Invalid Metalink SHA-256".to_string());
            }
            Ok(hash)
        })
        .transpose()?;
    let mut mirrors = Vec::new();
    for node in file.children().filter(|n| {
        n.is_element() && n.tag_name().namespace() == Some(NS) && n.tag_name().name() == "url"
    }) {
        let Some(href) = node.text().map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        let url = base
            .join(href)
            .map_err(|e| format!("Malformed Metalink mirror URL: {e}"))?;
        // Public Metalinks frequently advertise HTTP mirrors. They are not eligible.
        if checked_url(url.as_str(), true).is_err()
            || (base.scheme() == "https" && url.scheme() != "https")
        {
            continue;
        }
        let priority = node
            .attribute("priority")
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(u32::MAX);
        mirrors.push((priority, url.to_string()));
    }
    mirrors.sort_by_key(|(priority, _)| *priority);
    let choice = after
        .and_then(|after| mirrors.iter().position(|(_, url)| url == after))
        .map_or(0, |used| (used + 1) % mirrors.len());
    let url = mirrors
        .into_iter()
        .nth(choice)
        .map(|(_, url)| url)
        .ok_or("Metalink has no eligible HTTPS mirror (loopback HTTP is fixture-only)")?;
    Ok(Metalink { url, size, sha256 })
}
