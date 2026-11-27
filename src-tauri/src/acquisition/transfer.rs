// SPDX-License-Identifier: GPL-3.0-or-later
//! The single-stream HTTP transfer worker and its stop/resume outcomes.
#![forbid(unsafe_code)]
#[allow(unused_imports)]
use super::*;

const BODY_RETRIES: usize = 3;

pub(super) fn settle_worker_result(result: Result<Outcome, String>, action: Signal) -> Result<Outcome, String> {
    // Publication is already committed at Complete. Every earlier outcome,
    // including an error racing a local hash stop, must retain explicit intent.
    if matches!(&result, Ok(Outcome::Complete)) {
        return result;
    }
    if action != Signal::Run {
        return Ok(signal_outcome(action));
    }
    match result {
        Err(error) if error == "MOBILE_NETWORK_BLOCKED" => Ok(Outcome::Interrupted),
        other => other,
    }
}

pub(super) async fn execute_transfer(
    inner: &Arc<Inner>,
    id: &str,
    signal: &mut watch::Receiver<Signal>,
) -> Result<Outcome, String> {
    require_download_network(inner, id, &inner.snapshot(id)?.job.url)?;
    let network_denied = download_denied(inner, id);
    tokio::pin!(network_denied);
    let slots = Arc::clone(&inner.slots);
    let _permit = tokio::select! {
        biased;
        _ = &mut network_denied => return Ok(Outcome::Interrupted),
        _ = signal.changed() => {
            return Ok(signal_outcome(*signal.borrow()));
        }
        permit = slots.acquire_owned() => permit.map_err(|_| "Download scheduler closed")?,
    };
    if *signal.borrow() != Signal::Run {
        return Ok(signal_outcome(*signal.borrow()));
    }
    inner.update(id, |stored| {
        if stored.job.status == "queued" {
            stored.job.status = "downloading".into();
            stored.job.error = None;
        }
    })?;
    let snapshot = inner.snapshot(id)?;
    let (destination, part) = paths(&snapshot.job)?;
    inner
        .store
        .with_http_acquisition_destinations(&[&destination], &snapshot.job.url, || Ok(()))?;
    let directory = part
        .parent()
        .ok_or("Download has no destination directory")?;
    let file = open_part(&part)?;
    if let Some(ownership) = &snapshot.part_ownership {
        if !ownership.matches(&part, &file)? {
            return Err("Owned HTTP part was replaced; refusing to resume writes".into());
        }
    }
    let mut file = tokio::fs::File::from_std(file);
    crate::mobile_backup::exclude_payload(&file, &part)?;
    let part_length = file.metadata().await.map_err(|e| e.to_string())?.len();
    let mut downloaded = resume_offset(&snapshot, part_length);
    let source = download_url(inner, id, &snapshot.job.url)?;
    if snapshot.transfer_url.is_none() {
        let mut resolution_signal = signal.clone();
        let resolution = async {
            if source.scheme() != "kiwix-library" && (source.path().ends_with(".meta4") || source.path().ends_with(".metalink")) {
                let resolved = resolve_metalink(inner, &source, &mut resolution_signal).await?;
                inner.update(id, |stored| {
                    stored.transfer_url = Some(resolved.url);
                    stored.expected_size = Some(resolved.size);
                    stored.job.total = Some(resolved.size);
                    stored.job.sha256 = resolved.sha256;
                })
            } else {
                inner.update(id, |stored| stored.transfer_url = Some(source.to_string()))
            }
        };
        tokio::select! {
            biased;
            _ = &mut network_denied => return stop_file(inner, id, &mut file, downloaded, Signal::Interrupt).await,
            _ = signal.changed() => {
                let action = *signal.borrow();
                return stop_file(inner, id, &mut file, downloaded, action).await;
            }
            result = resolution => match result {
                Err(error) if error == "MOBILE_NETWORK_BLOCKED" => {
                    return stop_file(inner, id, &mut file, downloaded, Signal::Interrupt).await;
                }
                other => other?,
            },
        }
    }
    let mut snapshot = inner.snapshot(id)?;
    downloaded = resume_offset(&snapshot, part_length);
    // Resolution is complete before any payload write or resume truncation.
    // The job destination is authoritative (Metalink names never rename it).
    let (destination, _) = paths(&snapshot.job)?;
    inner
        .store
        .with_http_acquisition_destinations(&[&destination], &snapshot.job.url, || Ok(()))?;
    crate::storage::check_file_size(
        directory,
        snapshot
            .expected_size
            .or(snapshot.job.total)
            .unwrap_or(downloaded),
    )?;
    let known_total = snapshot.expected_size.or(snapshot.job.total);
    // A part left at its preallocated length already owns those bytes on
    // filesystems that allocate on `set_len`; only growth past it is new.
    let known_remaining =
        known_total.map(|size| space_remaining(size, downloaded, part_length));
    if let Some(remaining) = known_remaining { check_space(&part, remaining)?; }
    let outstanding = inner.outstanding_space(directory, id)?;
    crate::disk_warning::check_with_outstanding(
        directory, known_remaining, outstanding, &http_space_identity(&snapshot.job),
        snapshot.space_acknowledgement.as_deref(),
    )?;
    // A byte count is not proof of object identity. Never append without If-Range.
    if downloaded > 0 && validator(&snapshot).is_none() {
        // Request a full response, but retain the old bytes until its declared
        // size passes preflight. The full-response path truncates before writing.
        downloaded = 0;
        if snapshot.segmented {
            inner.update(id, |stored| {
                stored.segments = None;
                stored.segmented = false;
                stored.verification = None;
                stored.job.downloaded = 0;
            })?;
        }
    }
    file.seek(std::io::SeekFrom::Start(downloaded))
        .await
        .map_err(|e| e.to_string())?;
    let mut checkpoint = Instant::now();
    let mut admitted_total = None;
    let mut speed_since = Instant::now();
    let mut speed_start = downloaded;
    let mut last_speed = 0u64;
    let mut ranges_rejected = false;
    let mut segment_restarts = 0u32;
    // A 206 may legally end before the object, but an unbounded continuation
    // loop lets a short-range server issue request after request forever.
    let mut short_body_continues = 0u32;
    let mut body_retries = 0;
    let mut retry_delay = None;
    'transfer: {
    // A durable layout resumes each segment at its own synced offset without a
    // probe request. Every segment still sends If-Range, so a changed object
    // restarts the transfer instead of mixing representations.
    if let Some(plan) = valid_segment_plan(&snapshot)
        .filter(|plan| plan_fits(plan, part_length))
        .cloned()
    {
        if part_length < plan.total {
            file.set_len(plan.total)
                .await
                .map_err(|e| format!("Preallocating segmented download: {e}"))?;
            file.sync_all()
                .await
                .map_err(|e| format!("Synchronizing segmented download: {e}"))?;
        }
        let identity = file.metadata().await.map_err(|e| e.to_string())?;
        let handle_identity = tokio_file_identity(&file).await?;
        let total = plan.total;
        let transfer = download_url(inner, id, snapshot.transfer_url.as_deref().ok_or("Download has no transfer URL")?)?;
        match run_segments(
            Arc::new(SegmentTransfer::new(
                Arc::clone(inner), id.to_owned(), transfer, part.clone(), identity, handle_identity, plan,
            )),
            signal,
            download_denied(inner, id),
        )
        .await?
        {
            SegmentRun::Complete => {
                downloaded = total;
                break 'transfer;
            }
            SegmentRun::Restart { changed } => {
                file.set_len(0)
                    .await
                    .map_err(|e| format!("Restarting segmented download: {e}"))?;
                file.sync_all().await.map_err(|e| e.to_string())?;
                file.seek(std::io::SeekFrom::Start(0))
                    .await
                    .map_err(|e| e.to_string())?;
                downloaded = 0;
                inner.update(id, |stored| {
                    stored.segments = None;
                    stored.segmented = false;
                    stored.verification = None;
                    stored.job.downloaded = 0;
                    stored.job.speed = 0;
                    stored.job.eta_seconds = None;
                    stored.etag = None;
                    stored.last_modified = None;
                })?;
                ranges_rejected = !changed;
                segment_restarts += 1;
                if segment_restarts >= 2 {
                    // A server that keeps changing or misreporting the object
                    // must not trap the transfer in a segmentation loop.
                    ranges_rejected = true;
                }
                checkpoint = Instant::now();
                speed_start = 0;
                speed_since = Instant::now();
            }
            SegmentRun::Stopped(outcome) => return Ok(outcome),
        }
    }
    loop {
        if require_download_network(inner, id, &snapshot.job.url).is_err() {
            return stop_file(inner, id, &mut file, downloaded, Signal::Interrupt).await;
        }
        if *signal.borrow() != Signal::Run {
            let action = *signal.borrow();
            return stop_file(inner, id, &mut file, downloaded, action).await;
        }
        if let Some(delay) = retry_delay.take() {
            tokio::select! {
                biased;
                _ = &mut network_denied => return stop_file(inner, id, &mut file, downloaded, Signal::Interrupt).await,
                _ = signal.changed() => {
                    let action = *signal.borrow();
                    return stop_file(inner, id, &mut file, downloaded, action).await;
                }
                _ = tokio::time::sleep(delay) => {}
            }
        }
        snapshot = inner.snapshot(id)?;
        let transfer = download_url(inner, id, snapshot.transfer_url.as_deref().ok_or("Download has no transfer URL")?)?;
        let mut headers = http::HeaderMap::new();
        headers.insert(ACCEPT_ENCODING, http::HeaderValue::from_static("identity"));
        let resumed = downloaded > 0;
        if resumed {
            let valid =
                validator(&snapshot).ok_or("Partial transfer lacks a safe resume validator")?;
            headers.insert(RANGE, http::HeaderValue::from_str(&format!("bytes={downloaded}-")).map_err(|_| "Invalid download range")?);
            headers.insert(IF_RANGE, http::HeaderValue::from_str(valid).map_err(|_| "Invalid download validator")?);
        }
        let response = tokio::select! {
            biased;
            _ = &mut network_denied => return stop_file(inner, id, &mut file, downloaded, Signal::Interrupt).await,
            _ = signal.changed() => {
                let action = *signal.borrow();
                return stop_file(inner, id, &mut file, downloaded, action).await;
            }
            result = download_request(inner, id, transfer.clone(), headers) => match result {
                Ok(response) => response,
                Err(error)
                    if error.starts_with("Requesting download:")
                        && !error.contains("Unsafe HTTP redirect")
                        && !error.contains("Too many HTTP redirects")
                        && body_retries < BODY_RETRIES =>
                {
                    body_retries += 1;
                    retry_delay = Some(segment_delay(body_retries, 0, None));
                    continue;
                }
                Err(error) => return Err(error),
            },
        };
        let status = response.status();
        if (status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error())
            && body_retries < BODY_RETRIES
        {
            body_retries += 1;
            retry_delay = Some(segment_delay(body_retries, 0, Some(response.headers())));
            continue;
        }
        if response
            .headers()
            .get(CONTENT_ENCODING)
            .is_some_and(|h| h.as_bytes() != b"identity")
        {
            return Err("Encoded HTTP download cannot be resumed safely".into());
        }
        if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
            let total = response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|h| h.to_str().ok())
                .and_then(unsatisfied_range)
                .ok_or("416 response has no valid bytes */length Content-Range")?;
            if !resumed
                || total != downloaded
                || snapshot.expected_size.is_some_and(|n| n != total)
                || !validators_match(&snapshot, response.headers())
            {
                return Err("416 response does not prove that the partial file is complete".into());
            }
            inner.update(id, |stored| {
                stored.job.total = Some(total);
                stored.job.downloaded = downloaded;
            })?;
            break 'transfer;
        }
        if status != reqwest::StatusCode::OK && status != reqwest::StatusCode::PARTIAL_CONTENT {
            return Err(format!("Download server returned HTTP {status}"));
        }
        let content_length = header_u64(response.headers(), CONTENT_LENGTH.as_str())?;
        let (total, response_bytes, append) = if status == reqwest::StatusCode::PARTIAL_CONTENT {
            if !resumed {
                return Err("Server sent an unsolicited partial response".into());
            }
            let range = response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|h| h.to_str().ok())
                .and_then(parse_range)
                .ok_or("Invalid 206 Content-Range")?;
            if range.start != downloaded || !validators_match(&snapshot, response.headers()) {
                return Err(
                    "206 range start or object validator does not match the partial file".into(),
                );
            }
            let length = range.end - range.start + 1;
            if content_length.is_some_and(|n| n != length) {
                return Err("206 Content-Length does not match Content-Range".into());
            }
            if snapshot.job.total.is_some_and(|n| n != range.total) {
                return Err("Resumed object total changed".into());
            }
            (Some(range.total), Some(length), true)
        } else {
            (
                // A 200 means the full representation is being sent. When the
                // server omits Content-Length (chunked), a previously learned
                // total still proves the body length; without this fallback a
                // short chunked body would be adopted as a complete archive.
                content_length.or(snapshot.expected_size).or(snapshot.job.total),
                content_length,
                false,
            )
        };
        if let (Some(expected), Some(actual)) = (snapshot.expected_size, total) {
            if expected != actual {
                return Err(format!(
                    "Download size differs from expected size: {actual} versus {expected}"
                ));
            }
        }
        // Reject a known per-file filesystem limit before discarding a resumable part.
        crate::storage::check_file_size(directory, total.unwrap_or(downloaded))?;
        let mut large_file_checked =
            total.unwrap_or(downloaded) > crate::storage::FAT_MAX_FILE_SIZE;
        if admitted_total != Some(total) {
            // Retain discovered length even when admission fails, so explicit
            // Resume can request confirmation before another payload request.
            inner.update(id, |stored| stored.job.total = total)?;
            let retained = file.metadata().await.map_err(|e| e.to_string())?.len();
            if let Some(total) = total { check_space(&part, total.saturating_sub(retained))?; }
            let outstanding = inner.outstanding_space(directory, id)?;
            crate::disk_warning::check_with_outstanding(
                directory,
                total.map(|size| size.saturating_sub(retained)),
                outstanding,
                &http_space_identity(&snapshot.job),
                snapshot.space_acknowledgement.as_deref(),
            )?;
            admitted_total = Some(total);
        }
        // The checkpoint must track the bytes actually written in this
        // response, so a 200 restart resets it together with the counter.
        let mut checked_space_at = downloaded;
        let restarted_full = !append;
        if !append {
            // A 200 response to If-Range means the representation changed or ranges are
            // unsupported. Truncate before writing; never append the complete object.
            file.set_len(0)
                .await
                .map_err(|e| format!("Restarting full response: {e}"))?;
            file.sync_all()
                .await
                .map_err(|e| format!("Restarting full response: {e}"))?;
            file.seek(std::io::SeekFrom::Start(0))
                .await
                .map_err(|e| e.to_string())?;
            downloaded = 0;
            checked_space_at = 0;
            speed_start = 0;
            speed_since = Instant::now();
            checkpoint = Instant::now();
            last_speed = 0;
        }
        // A full response deliberately discarded the old partial above, so the
        // capacity check observes the space it reclaimed instead of double-counting.
        check_space(
            &part,
            total
                .map(|n| n.saturating_sub(downloaded))
                .unwrap_or(DISK_RESERVE),
        )?;
        let etag = header_text(response.headers(), ETAG.as_str())
            .filter(|tag| !tag.starts_with("W/") && tag.starts_with('"') && tag.ends_with('"'));
        let modified = header_text(response.headers(), LAST_MODIFIED.as_str())
            .filter(|value| crate::timestamps::parse_mail_date(value).is_ok());
        // Parallel ranges need a stable object, a known total and either a
        // server that advertises ranges or a 206 that already proved it honors
        // them. The layout is durable before the part is preallocated or any
        // segment byte is written, so a crash can never look complete.
        let response_validator = response_validator(response.headers());
        if !ranges_rejected {
            if let (Some(total_now), Some(validator_now)) = (total, response_validator) {
                if downloaded < total_now
                    && total_now >= SEGMENT_MIN_TOTAL
                    && (status == reqwest::StatusCode::PARTIAL_CONTENT
                        || accepts_ranges(response.headers()))
                {
                    if let Some(plan) = SegmentPlan::build(total_now, validator_now, downloaded) {
                        file.flush().await.map_err(|e| e.to_string())?;
                        inner.update(id, |stored| {
                            stored.job.downloaded = plan.durable();
                            stored.job.total = Some(total_now);
                            stored.etag = etag.clone();
                            stored.last_modified = modified.clone();
                            stored.segments = Some(plan.clone());
                            stored.segmented = true;
                        })?;
                        file.set_len(total_now)
                            .await
                            .map_err(|e| format!("Preallocating segmented download: {e}"))?;
                        file.sync_all()
                            .await
                            .map_err(|e| format!("Synchronizing segmented download: {e}"))?;
                        let identity = file.metadata().await.map_err(|e| e.to_string())?;
                        let handle_identity = tokio_file_identity(&file).await?;
                        drop(response);
                        match run_segments(
                            Arc::new(SegmentTransfer::new(
                                Arc::clone(inner), id.to_owned(), transfer.clone(), part.clone(), identity,
                                handle_identity, plan,
                            )),
                            signal,
                            download_denied(inner, id),
                        )
                        .await?
                        {
                            SegmentRun::Complete => {
                                downloaded = total_now;
                                break 'transfer;
                            }
                            SegmentRun::Restart { changed } => {
                                file.set_len(0).await.map_err(|e| {
                                    format!("Restarting segmented download: {e}")
                                })?;
                                file.sync_all().await.map_err(|e| e.to_string())?;
                                file.seek(std::io::SeekFrom::Start(0))
                                    .await
                                    .map_err(|e| e.to_string())?;
                                downloaded = 0;
                                inner.update(id, |stored| {
                                    stored.segments = None;
                                    stored.segmented = false;
                                    stored.verification = None;
                                    stored.job.downloaded = 0;
                                    stored.job.speed = 0;
                                    stored.job.eta_seconds = None;
                                    stored.etag = None;
                                    stored.last_modified = None;
                                })?;
                                ranges_rejected = !changed;
                                segment_restarts += 1;
                                if segment_restarts >= 2 {
                                    ranges_rejected = true;
                                }
                                checkpoint = Instant::now();
                                speed_start = 0;
                                speed_since = Instant::now();
                                continue;
                            }
                            SegmentRun::Stopped(outcome) => return Ok(outcome),
                        }
                    }
                }
            }
        }
        let retryable_body = matches!(&response, TransferResponse::Http(_))
            && (etag.is_some() || modified.is_some());
        // Validators are durable before the first byte associated with them is written.
        file.flush().await.map_err(|e| e.to_string())?;
        file.sync_data().await.map_err(|e| e.to_string())?;
        inner.update(id, |stored| {
            stored.job.downloaded = downloaded;
            stored.job.total = total;
            if !append || etag.is_some() {
                stored.etag = etag;
            }
            if !append || modified.is_some() {
                stored.last_modified = modified;
            }
            if restarted_full {
                stored.segments = None;
                stored.segmented = false;
                stored.verification = None;
            }
        })?;
        let mut received = 0u64;
        let stream = response
            .bytes_stream()
            .map(|result| result.map_err(std::io::Error::other));
        futures_util::pin_mut!(stream);
        let mut reader = StreamReader::new(stream);
        let mut buffer = vec![0u8; READ_BUDGET];
        let mut writer = tokio::io::BufWriter::with_capacity(WRITE_BUDGET, file);
        let mut speed_tick = Instant::now();
        let body_error = loop {
            let Some(permit) = bandwidth_permit(inner, signal).await? else {
                let action = *signal.borrow();
                return stop_buffered(inner, id, &mut writer, downloaded, action).await;
            };
            let result = tokio::select! {
                biased;
                _ = &mut network_denied => return stop_buffered(inner, id, &mut writer, downloaded, Signal::Interrupt).await,
                _ = signal.changed() => {
                    let action = *signal.borrow();
                    return stop_buffered(inner, id, &mut writer, downloaded, action).await;
                }
                result = reader.read(&mut buffer[..permit.bytes]) => result,
            };
            let count = match result {
                Ok(count) => count,
                Err(error) => {
                    refund_bandwidth(inner, &permit, 0)?;
                    break Some(error);
                }
            };
            refund_bandwidth(inner, &permit, count)?;
            if count == 0 {
                break None;
            }
            // Payload bytes are flowing; reset request-phase retry count so
            // transient connect/5xx retries do not starve mid-stream recovery.
            body_retries = 0;
            let chunk = &buffer[..count];
            let next_received = received
                .checked_add(chunk.len() as u64)
                .ok_or("Download response length overflow")?;
            let next_downloaded = downloaded
                .checked_add(chunk.len() as u64)
                .ok_or("Download length overflow")?;
            if response_bytes.is_some_and(|n| next_received > n)
                || total.is_some_and(|n| next_downloaded > n)
            {
                return Err("Download body exceeds its declared length".into());
            }
            // Unknown-length bodies need one native probe at the FAT boundary, not
            // a filesystem syscall for each received buffer on large-file volumes.
            if !large_file_checked && next_downloaded > crate::storage::FAT_MAX_FILE_SIZE {
                crate::storage::check_file_size(directory, next_downloaded)?;
                large_file_checked = true;
            }
            if space_check_due(next_downloaded, checked_space_at) {
                check_space(
                    &part,
                    total
                        .map(|n| n.saturating_sub(downloaded))
                        .unwrap_or(chunk.len() as u64),
                )?;
                checked_space_at = downloaded;
            }
            writer
                .write_all(chunk)
                .await
                .map_err(|e| format!("Writing download (possibly out of disk space): {e}"))?;
            received = next_received;
            downloaded = next_downloaded;
            if speed_tick.elapsed() >= SPEED_INTERVAL {
                let elapsed = speed_since.elapsed().as_secs_f64().max(0.001);
                last_speed = ((downloaded.saturating_sub(speed_start)) as f64 / elapsed) as u64;
                inner.update_memory(id, |stored| {
                    stored.job.downloaded = downloaded;
                    stored.job.speed = last_speed;
                    stored.job.eta_seconds = total.and_then(|t| {
                        if last_speed > 0 {
                            Some(t.saturating_sub(downloaded).div_ceil(last_speed))
                        } else {
                            None
                        }
                    });
                })?;
                speed_tick = Instant::now();
                speed_start = downloaded;
                speed_since = Instant::now();
            }
            if checkpoint.elapsed() >= SYNC_INTERVAL {
                writer.flush().await.map_err(|e| e.to_string())?;
                writer
                    .get_ref()
                    .sync_data()
                    .await
                    .map_err(|e| format!("Synchronizing download progress: {e}"))?;
                inner.update(id, |stored| {
                    stored.job.downloaded = downloaded;
                    stored.job.speed = last_speed;
                    stored.job.eta_seconds = total.and_then(|t| {
                        if last_speed > 0 {
                            Some(t.saturating_sub(downloaded).div_ceil(last_speed))
                        } else {
                            None
                        }
                    });
                })?;
                checkpoint = Instant::now();
            }
        };
        if let Some(error) = body_error {
            // Retry at the exact checkpoint on the same retained inode. The
            // next response must pass the normal If-Range/Content-Range and
            // validator checks; QUIC and unvalidated representations are not
            // retried through this path.
            if !retryable_body || downloaded == 0 || body_retries >= BODY_RETRIES {
                return Err(format!("Receiving download: {error}"));
            }
            writer.flush().await.map_err(|error| format!("Flushing download: {error}"))?;
            file = writer.into_inner();
            file.sync_data().await.map_err(|error| format!("Synchronizing download progress: {error}"))?;
            inner.update(id, |stored| {
                stored.job.downloaded = downloaded;
                stored.job.speed = 0;
                stored.job.eta_seconds = None;
            })?;
            body_retries += 1;
            retry_delay = Some(segment_delay(body_retries, 0, None));
            checkpoint = Instant::now();
            speed_since = checkpoint;
            speed_start = downloaded;
            last_speed = 0;
            continue;
        }
        if response_bytes.is_some_and(|n| n != received) {
            return Err("Download response ended before its declared length".into());
        }
        writer.flush().await.map_err(|e| e.to_string())?;
        writer
            .get_ref()
            .sync_all()
            .await
            .map_err(|e| format!("Synchronizing completed response: {e}"))?;
        file = writer.into_inner();
        inner.update(id, |stored| {
            stored.job.downloaded = downloaded;
            if total.is_none() {
                stored.job.total = Some(downloaded);
            }
        })?;
        if total.is_some_and(|n| downloaded < n) {
            if status == reqwest::StatusCode::PARTIAL_CONTENT {
                // Every continuation must add bytes and the loop stays bounded:
                // a server that keeps ending a 206 early fails instead of
                // spinning request after request.
                short_body_continues += 1;
                if received == 0 || short_body_continues > SHORT_BODY_CONTINUES {
                    return Err(
                        "Download server kept ending partial responses before the expected object size"
                            .into(),
                    );
                }
                continue;
            }
            return Err("Download ended before the expected object size".into());
        }
        break 'transfer;
    }
    }
    file.flush().await.map_err(|e| e.to_string())?;
    file.sync_all().await.map_err(|e| e.to_string())?;
    // Verification is streamed and pause/cancel remains responsive while hashing.
    file.seek(std::io::SeekFrom::Start(0))
        .await
        .map_err(|e| e.to_string())?;
    if downloaded < 80 {
        return Err("Downloaded file is too short to be a ZIM archive".into());
    }
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)
        .await
        .map_err(|e| format!("Reading ZIM signature: {e}"))?;
    if magic != *b"ZIM\x04" {
        return Err("Downloaded resource is not a ZIM archive (invalid magic)".into());
    }
    let snapshot = inner.snapshot(id)?;
    file.seek(std::io::SeekFrom::Start(0))
        .await
        .map_err(|e| e.to_string())?;
    // A durable receipt for this exact inode skips a repeat pass after a
    // restart or a foreground cycle. Any identity change forces a fresh hash.
    let identity = tokio_file_identity(&file).await?;
    if let Some(receipt) = matching_receipt(&snapshot, identity.as_ref()) {
        if receipt.identity.size != downloaded {
            return Err("Download receipt does not match the durable byte count".into());
        }
        match receipt.outcome {
            VerificationOutcome::Failed => {
                return Err(receipt.error.clone().unwrap_or_else(|| {
                    "Downloaded archive failed its advertised SHA-256 checksum; cancel and start a new download".into()
                }))
            }
            VerificationOutcome::Passed => {
                let digest = receipt.sha256.clone();
                verify_expected_digest(&snapshot, downloaded, &digest)?;
                if let Some(action) = worker_stop_action(inner, signal)? {
                    return stop_file(inner, id, &mut file, downloaded, action).await;
                }
                return publish_verified_download(
                    inner,
                    id,
                    file.into_std().await,
                    downloaded,
                    &digest,
                    identity,
                    FinalizeMode::Worker,
                );
            }
        }
    }
    #[cfg(test)]
    pass_verify_gate(inner, 0).await;
    // The row reads as a local finalize, never as a stalled download. Decide
    // under the state lock `stop` uses: a pause/cancel it already persisted
    // must never be overwritten by `verifying`. A crash after such an
    // overwrite would restart with a durable `verifying` row, and startup
    // finalize would then publish an archive the user had cancelled.
    let stop = {
        let mut state = inner.state()?;
        let action = state.running.get(id).map(|running| *running.command.borrow());
        if let Some(action @ (Signal::Pause | Signal::Cancel)) = action {
            Some(action)
        } else {
            let stored = state.jobs.get_mut(id).ok_or("Unknown download ID")?;
            stored.job.status = "verifying".into();
            stored.job.interrupted = false;
            stored.job.downloaded = downloaded;
            stored.job.speed = 0;
            stored.job.eta_seconds = None;
            stored.job.error = None;
            stored.job.updated_at = now();
            let journal = inner.stage(&state)?;
            drop(state);
            inner.publish(journal)?;
            None
        }
    };
    if let Some(action) = stop {
        return stop_file(inner, id, &mut file, downloaded, action).await;
    }
    #[cfg(test)]
    pass_verify_gate(inner, 1).await;
    #[cfg(test)]
    inner.hash_passes.fetch_add(1, Ordering::SeqCst);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; WRITE_BUDGET];
    let mut hashed = 0u64;
    loop {
        // Privacy or read-only revocation is a hard stop even without a new
        // signal, so a policy change can never leave hashing running.
        if finalize_forbidden(inner) {
            return stop_file(inner, id, &mut file, downloaded, Signal::Interrupt).await;
        }
        // Probe at most one byte past the admitted length so appends cannot extend the pass.
        let remaining = downloaded - hashed;
        let limit = remaining.saturating_add(1).min(buffer.len() as u64) as usize;
        let count = tokio::select! {
            biased;
            _ = signal.changed() => {
                let action = *signal.borrow();
                // A foreground/background cycle must not restart a local pass
                // that needs no network. Explicit pause/cancel still wins.
                if worker_stop_action(inner, signal)?.is_none() {
                    continue;
                }
                return stop_file(inner, id, &mut file, downloaded, action).await;
            }
            result = file.read(&mut buffer[..limit]) => result.map_err(|e| format!("Reading download checksum: {e}"))?,
        };
        if count == 0 {
            break;
        }
        if count as u64 > remaining {
            #[cfg(test)]
            inner.checksum_offset_on_growth.store(file.stream_position().await.unwrap(), Ordering::SeqCst);
            return Err(CHECKSUM_LENGTH_CHANGED.into());
        }
        hashed += count as u64;
        // Run CPU-bound SHA-256 compression off the Tokio async worker thread.
        let chunk = std::mem::take(&mut buffer);
        let (next_hasher, next_buffer) = tokio::task::spawn_blocking(move || {
            hasher.update(&chunk[..count]);
            (hasher, chunk)
        })
        .await
        .map_err(|error| format!("Checksum worker failed: {error}"))?;
        hasher = next_hasher;
        buffer = next_buffer;
        #[cfg(test)]
        {
            let delay = inner.hash_delay.load(Ordering::Acquire);
            if delay > 0 {
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }
        }
    }
    if hashed != downloaded {
        return Err(CHECKSUM_LENGTH_CHANGED.into());
    }
    let after = tokio_file_identity(&file).await?;
    if after != identity {
        return Err("Download changed identity during checksum verification".into());
    }
    let digest = hex::encode(hasher.finalize());
    if let Err(error) = verify_expected_digest(&snapshot, downloaded, &digest) {
        record_verification_failure(inner, id, after, &digest, &error);
        return Err(error);
    }
    record_verification_success(inner, id, after.clone(), &digest)?;
    if let Some(action) = worker_stop_action(inner, signal)? {
        return stop_file(inner, id, &mut file, downloaded, action).await;
    }
    publish_verified_download(
        inner,
        id,
        file.into_std().await,
        downloaded,
        &digest,
        after,
        FinalizeMode::Worker,
    )
}
