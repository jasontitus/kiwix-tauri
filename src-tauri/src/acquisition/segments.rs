// SPDX-License-Identifier: GPL-3.0-or-later
//! Segmented (multi-range) HTTP transfers with durable per-range progress.
#[allow(unused_imports)]
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SegmentRun {
    Complete,
    Restart { changed: bool },
    Stopped(Outcome),
}

pub(super) struct SegmentCounters {
    pub(super) written: Vec<AtomicU64>,
    pub(super) durable: Vec<AtomicU64>,
    /// Start time of the last completed inode fsync. Range workers share one
    /// sync per interval instead of issuing one each against the same part.
    sync: tokio::sync::Mutex<Option<Instant>>,
    /// Completed `BufWriter` flushes, so a syncing worker can tell whether a
    /// peer flushed while its fsync was running and needs one more pass.
    flushes: AtomicU64,
}

impl SegmentCounters {
    pub(super) fn new(plan: &SegmentPlan) -> Self {
        let mut written = Vec::with_capacity(plan.ranges.len());
        let mut durable = Vec::with_capacity(plan.ranges.len());
        for range in &plan.ranges {
            written.push(AtomicU64::new(range.done));
            durable.push(AtomicU64::new(range.done));
        }
        Self {
            written,
            durable,
            sync: tokio::sync::Mutex::new(None),
            flushes: AtomicU64::new(0),
        }
    }

    pub(super) fn written_total(&self) -> u64 {
        self.written
            .iter()
            .map(|value| value.load(Ordering::Acquire))
            .sum()
    }
}

/// One retained transfer shared by every range worker, including inode proof
/// and counters. Workers clone only this Arc, not the URL/path/job strings.
pub(super) struct SegmentTransfer {
    pub(super) inner: Arc<Inner>,
    pub(super) id: String,
    pub(super) transfer: Url,
    pub(super) part: PathBuf,
    pub(super) identity: fs::Metadata,
    /// The coordinator handle's receipt identity (see [`file_identity`]).
    pub(super) handle_identity: Option<PartIdentity>,
    pub(super) plan: SegmentPlan,
    pub(super) counters: SegmentCounters,
}

impl SegmentTransfer {
    pub(super) fn new(
        inner: Arc<Inner>, id: String, transfer: Url, part: PathBuf, identity: fs::Metadata,
        handle_identity: Option<PartIdentity>, plan: SegmentPlan,
    ) -> Self {
        let counters = SegmentCounters::new(&plan);
        Self { inner, id, transfer, part, identity, handle_identity, plan, counters }
    }
}

pub(super) struct SegmentError {
    pub(super) restart: bool,
    pub(super) changed: bool,
    pub(super) message: String,
}

impl SegmentError {
    pub(super) fn fatal(message: impl Into<String>) -> Self {
        Self {
            restart: false,
            changed: false,
            message: message.into(),
        }
    }

    pub(super) fn restart(changed: bool, message: impl Into<String>) -> Self {
        Self {
            restart: true,
            changed,
            message: message.into(),
        }
    }
}

pub(super) fn response_validator(headers: &reqwest::header::HeaderMap) -> Option<String> {
    header_text(headers, ETAG.as_str())
        .filter(|tag| !tag.starts_with("W/") && tag.starts_with('"') && tag.ends_with('"'))
        .or_else(|| {
            header_text(headers, LAST_MODIFIED.as_str())
                .filter(|value| chrono::DateTime::parse_from_rfc2822(value).is_ok())
        })
}

pub(super) fn accepts_ranges(headers: &reqwest::header::HeaderMap) -> bool {
    headers
        .get(ACCEPT_RANGES)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("bytes"))
        })
}

pub(super) fn segment_delay(
    attempt: usize,
    index: usize,
    headers: Option<&reqwest::header::HeaderMap>,
) -> Duration {
    let retry_after = headers
        .and_then(|headers| header_u64(headers, "retry-after").ok().flatten())
        .filter(|seconds| *seconds > 0)
        .map(|seconds| Duration::from_secs(seconds.min(30)));
    let base = retry_after.unwrap_or_else(|| {
        Duration::from_secs(1u64 << attempt.saturating_sub(1).min(4))
    });
    base + Duration::from_millis((index as u64 * 137) % 1000)
}

pub(super) async fn segment_sleep(
    stop: &mut watch::Receiver<bool>,
    signal: &mut watch::Receiver<Signal>,
    delay: Duration,
) -> Result<bool, SegmentError> {
    if *stop.borrow() {
        return Ok(false);
    }
    if *signal.borrow() != Signal::Run {
        return Ok(false);
    }
    tokio::select! {
        biased;
        changed = stop.changed() => Ok(changed.is_ok() && !*stop.borrow()),
        changed = signal.changed() => Ok(changed.is_ok() && *signal.borrow() == Signal::Run),
        _ = tokio::time::sleep(delay) => Ok(true),
    }
}

pub(super) async fn flush_segment(
    writer: &mut BufWriter<tokio::fs::File>,
    counters: &SegmentCounters,
    index: usize,
    done: u64,
) -> Result<(), SegmentError> {
    writer
        .flush()
        .await
        .map_err(|error| SegmentError::fatal(format!("Writing download: {error}")))?;
    counters.flushes.fetch_add(1, Ordering::Release);
    // Every worker's flushed bytes live in the same inode's page cache, so one
    // fsync that starts after a flush makes that flush durable. Only the first
    // worker of an interval syncs; peers that flushed while that fsync ran are
    // covered by one more pass instead of an fsync each.
    let written_at = Instant::now();
    let mut last_sync = counters.sync.lock().await;
    let covered = matches!(*last_sync, Some(started) if started > written_at);
    if !covered {
        for _ in 0..2 {
            let flushes = counters.flushes.load(Ordering::Acquire);
            let started = Instant::now();
            writer
                .get_ref()
                .sync_data()
                .await
                .map_err(|error| SegmentError::fatal(format!("Synchronizing download progress: {error}")))?;
            // Record only a completed sync: a failed one must not let another
            // range publish bytes the disk never received.
            *last_sync = Some(started);
            if counters.flushes.load(Ordering::Acquire) == flushes {
                break;
            }
        }
    }
    drop(last_sync);
    counters.durable[index].store(done, Ordering::Release);
    Ok(())
}

/// One parallel range. Progress is only published to the durable counter after
/// `sync_data`, so a crash can lose at most one sync interval of re-download.
pub(super) async fn segment_worker(
    shared: Arc<SegmentTransfer>,
    index: usize,
    mut stop: watch::Receiver<bool>,
    mut signal: watch::Receiver<Signal>,
) -> Result<(), SegmentError> {
    let SegmentTransfer { inner, id, transfer, part, identity, handle_identity, plan, counters } = shared.as_ref();
    let range = &plan.ranges[index];
    let length = range.end - range.start + 1;
    let mut done = range.done;
    if done >= length {
        return Ok(());
    }
    let file = open_segment_part(part, identity, handle_identity.as_ref()).map_err(SegmentError::fatal)?;
    let mut file = tokio::fs::File::from_std(file);
    file.seek(std::io::SeekFrom::Start(range.start + done))
        .await
        .map_err(|error| SegmentError::fatal(format!("Seeking download segment: {error}")))?;
    let mut writer = BufWriter::with_capacity(WRITE_BUDGET, file);
    let mut buffer = vec![0u8; READ_BUDGET];
    let mut retries = 0usize;
    let mut since_sync = Instant::now();
    loop {
        if *stop.borrow() || *signal.borrow() != Signal::Run {
            flush_segment(&mut writer, counters, index, done).await?;
            return Ok(());
        }
        if done >= length {
            flush_segment(&mut writer, counters, index, done).await?;
            return Ok(());
        }
        let request_start = range.start + done;
        let expected = range.end - request_start + 1;
        let mut headers = http::HeaderMap::new();
        headers.insert(ACCEPT_ENCODING, http::HeaderValue::from_static("identity"));
        headers.insert(RANGE, http::HeaderValue::from_str(&format!("bytes={request_start}-{}", range.end)).map_err(|_| SegmentError::fatal("Invalid segment range"))?);
        headers.insert(IF_RANGE, http::HeaderValue::from_str(&plan.validator).map_err(|_| SegmentError::fatal("Invalid segment validator"))?);
        let request = download_request(inner, id, transfer.clone(), headers);
        let response = tokio::select! {
            biased;
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    flush_segment(&mut writer, counters, index, done).await?;
                    return Ok(());
                }
                continue;
            }
            changed = signal.changed() => {
                if changed.is_err() || *signal.borrow() != Signal::Run {
                    flush_segment(&mut writer, counters, index, done).await?;
                    return Ok(());
                }
                continue;
            }
            result = request => match result {
                Ok(response) => response,
                Err(error) => {
                    retries += 1;
                    if retries > SEGMENT_RETRIES {
                        return Err(SegmentError::fatal(format!(
                            "Requesting download segment: {error}"
                        )));
                    }
                    if !segment_sleep(&mut stop, &mut signal, segment_delay(retries, index, None)).await? {
                        flush_segment(&mut writer, counters, index, done).await?;
                        return Ok(());
                    }
                    continue;
                }
            },
        };
        if response
            .headers()
            .get(CONTENT_ENCODING)
            .is_some_and(|value| value.as_bytes() != b"identity")
        {
            return Err(SegmentError::fatal(
                "Encoded HTTP download cannot be resumed safely",
            ));
        }
        let status = response.status();
        match status {
            reqwest::StatusCode::PARTIAL_CONTENT => {}
            reqwest::StatusCode::OK => {
                let changed =
                    response_validator(response.headers()).as_deref() != Some(plan.validator.as_str());
                return Err(SegmentError::restart(
                    changed,
                    "Server ignored a range request during a segmented transfer",
                ));
            }
            reqwest::StatusCode::RANGE_NOT_SATISFIABLE => {
                return Err(SegmentError::restart(
                    true,
                    "416 response ended a segmented transfer",
                ));
            }
            reqwest::StatusCode::TOO_MANY_REQUESTS | reqwest::StatusCode::SERVICE_UNAVAILABLE => {
                retries += 1;
                if retries > SEGMENT_RETRIES {
                    return Err(SegmentError::fatal(format!(
                        "Download server returned HTTP {status} for a segment"
                    )));
                }
                let delay = segment_delay(retries, index, Some(response.headers()));
                if !segment_sleep(&mut stop, &mut signal, delay).await? {
                    flush_segment(&mut writer, counters, index, done).await?;
                    return Ok(());
                }
                continue;
            }
            other if other.is_server_error() => {
                retries += 1;
                if retries > SEGMENT_RETRIES {
                    return Err(SegmentError::fatal(format!(
                        "Download server returned HTTP {other}"
                    )));
                }
                if !segment_sleep(&mut stop, &mut signal, segment_delay(retries, index, None)).await? {
                    flush_segment(&mut writer, counters, index, done).await?;
                    return Ok(());
                }
                continue;
            }
            other => {
                return Err(SegmentError::fatal(format!(
                    "Download server returned HTTP {other}"
                )));
            }
        }
        let Some(header_range) = response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .and_then(parse_range)
        else {
            return Err(SegmentError::fatal("Invalid 206 Content-Range"));
        };
        if header_range.start != request_start
            || header_range.end != range.end
            || header_range.total != plan.total
        {
            return Err(SegmentError::restart(
                true,
                "206 range or object total changed during a segmented transfer",
            ));
        }
        let content_length = header_u64(response.headers(), CONTENT_LENGTH.as_str())
            .map_err(SegmentError::fatal)?;
        if content_length.is_some_and(|value| value != expected) {
            return Err(SegmentError::fatal(
                "206 Content-Length does not match Content-Range",
            ));
        }
        if response_validator(response.headers()).as_deref() != Some(plan.validator.as_str()) {
            return Err(SegmentError::restart(
                true,
                "Object validator changed during a segmented transfer",
            ));
        }
        let before = done;
        let mut received = 0u64;
        let stream = response
            .bytes_stream()
            .map(|result| result.map_err(std::io::Error::other));
        futures_util::pin_mut!(stream);
        let mut reader = StreamReader::new(stream);
        loop {
            if received >= expected {
                break;
            }
            let remaining = (expected - received).min(READ_BUDGET as u64) as usize;
            let permit = tokio::select! {
                biased;
                changed = stop.changed() => {
                    if changed.is_err() || *stop.borrow() {
                        flush_segment(&mut writer, counters, index, done).await?;
                        return Ok(());
                    }
                    continue;
                }
                result = bandwidth_permit(inner, &mut signal) => {
                    result.map_err(SegmentError::fatal)?
                }
            };
            let Some(permit) = permit else {
                flush_segment(&mut writer, counters, index, done).await?;
                return Ok(());
            };
            let count = tokio::select! {
                biased;
                changed = stop.changed() => {
                    if changed.is_err() || *stop.borrow() {
                        flush_segment(&mut writer, counters, index, done).await?;
                        return Ok(());
                    }
                    continue;
                }
                changed = signal.changed() => {
                    if changed.is_err() || *signal.borrow() != Signal::Run {
                        flush_segment(&mut writer, counters, index, done).await?;
                        return Ok(());
                    }
                    continue;
                }
                result = reader.read(&mut buffer[..permit.bytes.min(remaining)]) => match result {
                    Ok(count) => count,
                    Err(error) => {
                        flush_segment(&mut writer, counters, index, done).await?;
                        return Err(SegmentError::fatal(format!(
                            "Receiving download segment: {error}"
                        )));
                    }
                },
            };
            refund_bandwidth(inner, &permit, count).map_err(SegmentError::fatal)?;
            if count == 0 {
                break;
            }
            if count as u64 > expected - received {
                return Err(SegmentError::fatal(
                    "Download segment exceeds its declared length",
                ));
            }
            writer
                .write_all(&buffer[..count])
                .await
                .map_err(|error| {
                    SegmentError::fatal(format!(
                        "Writing download (possibly out of disk space): {error}"
                    ))
                })?;
            received += count as u64;
            done += count as u64;
            counters.written[index].store(done, Ordering::Release);
            if since_sync.elapsed() >= SYNC_INTERVAL {
                flush_segment(&mut writer, counters, index, done).await?;
                since_sync = Instant::now();
            }
        }
        flush_segment(&mut writer, counters, index, done).await?;
        if done > before {
            retries = 0;
        }
        if received < expected {
            retries += 1;
            if retries > SEGMENT_RETRIES {
                return Err(SegmentError::fatal(
                    "Download segment ended before its declared length",
                ));
            }
            if !segment_sleep(&mut stop, &mut signal, segment_delay(retries, index, None)).await? {
                return Ok(());
            }
            continue;
        }
        if done == length {
            return Ok(());
        }
    }
}

pub(super) fn persist_segment_progress(
    inner: &Inner,
    id: &str,
    plan: &SegmentPlan,
    counters: &SegmentCounters,
    speed: u64,
) -> Result<(), String> {
    let mut state = inner.state()?;
    let stored = state.jobs.get_mut(id).ok_or("Unknown download ID")?;
    let (downloaded, total) = {
        let current = stored
            .segments
            .as_mut()
            .ok_or("Segmented download layout is missing")?;
        if current.total != plan.total || current.ranges.len() != counters.durable.len() {
            return Err("Segmented download layout changed while it was running".into());
        }
        for (range, durable) in current.ranges.iter_mut().zip(counters.durable.iter()) {
            let length = range.end - range.start + 1;
            range.done = durable.load(Ordering::Acquire).min(length);
        }
        (current.durable(), current.total)
    };
    stored.job.downloaded = downloaded;
    stored.job.total = Some(total);
    stored.job.speed = speed;
    stored.job.eta_seconds = if speed > 0 {
        Some(total.saturating_sub(downloaded).div_ceil(speed))
    } else {
        None
    };
    stored.job.updated_at = now();
    let journal = inner.stage(&state)?;
    drop(state);
    inner.publish(journal)
}

pub(super) async fn run_segments(
    shared: Arc<SegmentTransfer>,
    signal: &mut watch::Receiver<Signal>,
    denial: impl Future<Output = ()>,
) -> Result<SegmentRun, String> {
    let SegmentTransfer { inner, id, part, identity, plan, counters, .. } = shared.as_ref();
    // The retained inode length is the region the filesystem may already have
    // charged to this transfer, including a resumed preallocation.
    let preallocated = identity.len();
    let (stop_tx, stop_rx) = watch::channel(false);
    let mut tasks = JoinSet::new();
    for (index, range) in plan.ranges.iter().enumerate() {
        if range.done > range.end - range.start {
            continue;
        }
        tasks.spawn(segment_worker(
            Arc::clone(&shared),
            index,
            stop_rx.clone(),
            signal.clone(),
        ));
    }
    let mut stop_outcome = None;
    let mut failure: Option<SegmentError> = None;
    let mut last_persist = Instant::now();
    let mut speed_at = Instant::now();
    let mut speed_bytes = counters.written_total();
    let mut last_speed;
    let mut checked_space_at = plan.durable();
    let mut tick = tokio::time::interval(SPEED_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    tick.tick().await;
    let network_denied = denial;
    tokio::pin!(network_denied);
    while !tasks.is_empty() {
        tokio::select! {
            biased;
            // The denial future is an async fn: polling it after completion
            // panics. The guard disables the arm as soon as it has produced a
            // stop, even though the workers still have to drain.
            _ = &mut network_denied, if stop_outcome.is_none() => {
                stop_outcome = Some(Outcome::Interrupted);
                stop_tx.send_replace(true);
            }
            changed = signal.changed() => {
                if changed.is_err() {
                    if stop_outcome.is_none() {
                        stop_outcome = Some(Outcome::Paused);
                    }
                    stop_tx.send_replace(true);
                } else {
                    let action = *signal.borrow();
                    if action != Signal::Run {
                        if stop_outcome.is_none() {
                            stop_outcome = Some(signal_outcome(action));
                        }
                        stop_tx.send_replace(true);
                    }
                }
            }
            joined = tasks.join_next() => match joined {
                Some(Ok(Ok(()))) => {}
                Some(Ok(Err(error))) => {
                    if failure.is_none() {
                        failure = Some(error);
                    }
                    stop_tx.send_replace(true);
                }
                Some(Err(error)) => {
                    if failure.is_none() {
                        failure = Some(SegmentError::fatal(format!(
                            "Download segment task failed: {error}"
                        )));
                    }
                    stop_tx.send_replace(true);
                }
                None => break,
            },
            _ = tick.tick() => {
                let written = counters.written_total().min(plan.total);
                let elapsed = speed_at.elapsed().as_secs_f64().max(0.001);
                last_speed = ((written.saturating_sub(speed_bytes)) as f64 / elapsed) as u64;
                speed_at = Instant::now();
                speed_bytes = written;
                inner.update_memory(id, |stored| {
                    stored.job.downloaded = written;
                    stored.job.speed = last_speed;
                    stored.job.eta_seconds = if last_speed > 0 {
                        Some(plan.total.saturating_sub(written).div_ceil(last_speed))
                    } else {
                        None
                    };
                })?;
                if space_check_due(written, checked_space_at) {
                    let remaining = space_remaining(plan.total, written, preallocated);
                    if let Err(error) = check_space(part, remaining) {
                        if failure.is_none() {
                            failure = Some(SegmentError::fatal(error));
                        }
                        stop_tx.send_replace(true);
                    } else {
                        checked_space_at = written;
                    }
                }
                if last_persist.elapsed() >= SYNC_INTERVAL {
                    persist_segment_progress(inner, id, plan, counters, last_speed)?;
                    last_persist = Instant::now();
                }
            }
        }
    }
    stop_tx.send_replace(true);
    if let Some(outcome) = stop_outcome {
        persist_segment_progress(inner, id, plan, counters, 0)?;
        return Ok(SegmentRun::Stopped(outcome));
    }
    if let Some(failure) = failure {
        persist_segment_progress(inner, id, plan, counters, 0)?;
        return if failure.restart {
            Ok(SegmentRun::Restart {
                changed: failure.changed,
            })
        } else {
            Err(failure.message)
        };
    }
    for (index, range) in plan.ranges.iter().enumerate() {
        let length = range.end - range.start + 1;
        if counters.durable[index].load(Ordering::Acquire) != length {
            return Err("Segmented download ended with an incomplete range".into());
        }
    }
    persist_segment_progress(inner, id, plan, counters, 0)?;
    Ok(SegmentRun::Complete)
}
