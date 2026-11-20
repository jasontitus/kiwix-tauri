// SPDX-License-Identifier: GPL-3.0-or-later
//! Loading, validating and quarantining saved download rows and segment plans.
#![forbid(unsafe_code)]
#[allow(unused_imports)]
use super::*;

/// A plan is only usable while its total and validator still match the row and
/// its byte intervals cover the object exactly once.
pub(super) fn valid_segment_plan(stored: &StoredJob) -> Option<&SegmentPlan> {
    let plan = stored.segments.as_ref()?;
    let validator = validator(stored)?;
    if !plan.covers(plan.total, validator)
        || stored.job.total.is_some_and(|total| total != plan.total)
        || stored.expected_size.is_some_and(|total| total != plan.total)
    {
        return None;
    }
    Some(plan)
}

/// The part file may be shorter than the plan (a crash before preallocation),
/// but it must never be longer than the object or shorter than synced bytes.
/// An unstarted range records no synced bytes, so its offset does not have to
/// exist in the part yet: preallocation precedes the first segment write, and a
/// crash between the journal write and `set_len` must not discard the plan.
pub(super) fn plan_fits(plan: &SegmentPlan, part_length: u64) -> bool {
    part_length <= plan.total
        && plan.ranges.iter().all(|range| {
            range.done == 0 || range.start.saturating_add(range.done) <= part_length
        })
}

/// The whole object length once a segmented journal proves every byte is on
/// disk. A preallocated part length is never accepted as progress. A
/// single-response row has no per-range plan, so it keeps its existing
/// resume path instead of being finalized from a byte count alone.
pub(super) fn durable_total(stored: &StoredJob) -> Option<u64> {
    if let Some(native) = &stored.native {
        return native.total.filter(|total| *total > 0 && native.copied == *total);
    }
    if !stored.segmented {
        return None;
    }
    valid_segment_plan(stored)
        .filter(|plan| plan.durable() == plan.total)
        .map(|plan| plan.total)
}

/// A row that may be finished locally without a worker: its bytes are durable,
/// its part exists, and no deletion or publication cleanup has started.
pub(super) fn durable_finalize_candidate(stored: &StoredJob) -> bool {
    if stored.history_dismissed || stored.files_deleted || durable_total(stored).is_none() {
        return false;
    }
    if stored
        .part_ownership
        .as_ref()
        .is_some_and(crate::torrent::CleanupEntry::cleanup_started)
        || stored
            .published_ownership
            .as_ref()
            .is_some_and(crate::torrent::CleanupEntry::cleanup_started)
    {
        return false;
    }
    let Ok((_, part)) = paths(&stored.job) else {
        return false;
    };
    matches!(regular_file(&part), Ok(Some(_)))
}

/// The byte offset a transfer must continue from. A preallocated part's file
/// length is not progress: only the durable plan is.
pub(super) fn resume_offset(stored: &StoredJob, part_length: u64) -> u64 {
    if stored.segmented {
        valid_segment_plan(stored)
            .filter(|plan| plan_fits(plan, part_length))
            .map(SegmentPlan::durable)
            .unwrap_or(0)
    } else {
        part_length
    }
}

/// Validate and repair one saved download. `Err` means the record is dropped
/// and reported; it never aborts the manager or startup.
pub(super) fn validate_loaded_download(id: &str, stored: &mut StoredJob) -> Result<(), String> {
    if id != stored.job.id {
        return Err("Persisted download key/ID mismatch".into());
    }
    if stored.job.url.starts_with("kiwix-library:") {
        let source = Url::parse(&stored.job.url).map_err(|_| "Invalid shared library history URL")?;
        if source.scheme() != "kiwix-library" || source.host_str().is_none()
            || !source.username().is_empty() || source.password().is_some()
            || source.query().is_some() || source.fragment().is_some()
            || stored.native.is_some() || stored.transfer_url.as_deref().is_some_and(|url| url != stored.job.url)
        { return Err("Invalid shared library history URL".into()); }
    } else {
        checked_url(&stored.job.url, true)?;
        if let Some(url) = &stored.transfer_url { checked_url(url, true)?; }
    }
    if let Some(native) = &stored.native {
        Uuid::parse_str(&native.generation).map_err(|_| "Invalid native download generation")?;
        if stored.segmented || stored.segments.is_some()
            || native.total == Some(0)
            || native.total.map_or(native.copied != 0, |total| native.copied > total)
        {
            return Err("Invalid native download adoption checkpoint".into());
        }
    }
    let (destination, part) = paths(&stored.job)?;
    if stored.history_dismissed || stored.files_deleted {
        return Ok(());
    }
    // This launch recovered the durable journal or its publication intent.
    stored.job.history_warning = None;
    stored.job.speed = 0;
    stored.job.eta_seconds = None;
    if !matches!(
        stored.job.status.as_str(),
        "queued" | "downloading" | "verifying" | "paused" | "complete" | "failed" | "cancelled"
    ) {
        return Err("Invalid persisted download status".into());
    }
    if stored.job.status == "complete" {
        stored.job.interrupted = false;
        stored.segments = None;
        stored.segmented = false;
        stored.verification = None;
        match regular_file(&destination) {
            Ok(Some(metadata)) if stored.job.total == Some(metadata.len()) => {
                // The destination is back with its expected length, so an
                // earlier "missing or changed" note is stale. A real cleanup
                // failure below replaces this with its own message.
                stored.job.error = None;
                // Complete history is not import authority: the import worker
                // checks original ownership and digest before registration.
                // Leave legacy/public part names alone; pathname validation
                // cannot authorize a later unlink.
                if regular_file(&part)?.is_some() {
                    stored.job.error = Some("A leftover download part was preserved. Remove it manually only if you no longer need it".into());
                }
            }
            _ => {
                // A completed transfer remains historical even if its
                // archive was subsequently moved, deleted or replaced.
                // Do not turn it into a resumable destination reservation.
                stored.job.error =
                    Some("Completed archive is missing or has changed length".into());
            }
        }
        return Ok(());
    }
    if stored.job.status != "cancelled" {
        match recover_publication(stored, &destination, &part) {
            Ok(true) => {
                stored.job.status = "complete".into();
                stored.job.interrupted = false;
                stored.segments = None;
                stored.segmented = false;
                stored.verification = None;
                stored.job.error = None;
                stored.job.updated_at = now();
                return Ok(());
            }
            Ok(false) => {}
            Err(error) => {
                stored.job.status = "failed".into();
                stored.job.interrupted = false;
                stored.job.error = Some(error);
                return Ok(());
            }
        }
    }
    if stored.job.status == "cancelled" {
        stored.job.interrupted = false;
        stored.segments = None;
        stored.segmented = false;
        stored.verification = None;
        // Cleanup runs after manager construction, when a private capture can
        // be durably journaled before its owned payload is moved or removed.
        return Ok(());
    }
    match regular_file(&part) {
        Ok(Some(metadata)) => {
            if let Some(native) = &mut stored.native {
                if metadata.len() < native.copied {
                    native.copied = 0;
                    native.total = None;
                    native.stage_identity = None;
                    stored.verification = None;
                }
                stored.job.downloaded = native.copied;
            } else if stored.segmented {
                match valid_segment_plan(stored) {
                    Some(plan) if plan_fits(plan, metadata.len()) => {
                        stored.job.downloaded = plan.durable();
                    }
                    _ => {
                        // A preallocated part must never be read as a complete
                        // file. Drop the layout and force a fresh transfer.
                        stored.segments = None;
                        stored.segmented = false;
                        stored.verification = None;
                        stored.job.downloaded = 0;
                        stored.etag = None;
                        stored.last_modified = None;
                    }
                }
            } else {
                stored.job.downloaded = metadata.len();
            }
        }
        Ok(None) => {
            stored.job.downloaded = 0;
            stored.etag = None;
            stored.last_modified = None;
            stored.segments = None;
            stored.segmented = false;
            stored.verification = None;
            if let Some(native) = &mut stored.native {
                native.copied = 0;
                native.total = None;
                native.stage_identity = None;
            }
        }
        Err(error) => {
            stored.job.status = "failed".into();
            stored.job.interrupted = false;
            stored.job.error = Some(error);
            return Ok(());
        }
    }
    if matches!(stored.job.status.as_str(), "downloading" | "queued") {
        stored.job.status = "paused".into();
        stored.job.interrupted = true;
        stored.job.error =
            Some("Download interrupted by application shutdown; resume when ready".into());
        stored.job.updated_at = now();
    }
    if stored.job.status == "verifying" {
        // Only a row whose durable journal is whole may stay in verification;
        // anything else must become resumable instead of a permanent 100% row.
        if durable_total(stored).is_some() {
            stored.job.downloaded = stored.job.total.unwrap_or(stored.job.downloaded);
            stored.job.interrupted = false;
        } else {
            stored.job.status = "paused".into();
            stored.job.interrupted = true;
            stored.job.error =
                Some("Verification was interrupted by application shutdown; resume when ready".into());
            stored.job.updated_at = now();
        }
    }
    if stored.job.status != "paused" {
        stored.job.interrupted = false;
    }
    Ok(())
}

/// Read one durable downloads root. `Err` means the record as a whole is
/// unreadable; per-record failures stay warnings so one bad row cannot hide
/// the rest of the history.
pub(super) fn load_saved_downloads(
    file: &File,
    size: u64,
    store: &Store,
) -> Result<(BTreeMap<String, StoredJob>, Vec<String>), String> {
    #[cfg(not(desktop))]
    let _ = store;
    if size > STATE_LIMIT {
        return Err("Download state exceeds size limit".into());
    }
    let mut bytes = Vec::with_capacity(size.min(STATE_LIMIT) as usize);
    file.take(STATE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Reading download state: {e}"))?;
    if bytes.len() as u64 > STATE_LIMIT {
        return Err("Download state exceeds size limit".into());
    }
    #[cfg(desktop)]
    if store.portable_mode() {
        // The rebase rewrites paths inside the document, so portable mode keeps
        // the owned DOM and hands each row to serde without cloning it again.
        let mut value: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|e| format!("Invalid persistent download state: {e}"))?;
        store.portable_rebase_saved_json(
            crate::profile_paths::RecordKind::Downloads,
            &mut value,
        )?;
        let entries = value
            .as_object_mut()
            .map(std::mem::take)
            .ok_or("Invalid persistent download state: expected an object")?;
        return decode_saved_downloads(entries);
    }
    // The common record is decoded straight into `StoredJob`s; only a row that
    // fails falls back to the DOM so one bad entry stays a warning instead of
    // discarding the rest of the history.
    match serde_json::from_slice::<BTreeMap<String, StoredJob>>(&bytes) {
        Ok(jobs) => {
            if jobs.len() > MAX_JOBS {
                return Err("Too many persisted download jobs".into());
            }
            Ok((jobs, Vec::new()))
        }
        Err(_) => {
            let mut value: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|e| format!("Invalid persistent download state: {e}"))?;
            let entries = value
                .as_object_mut()
                .map(std::mem::take)
                .ok_or("Invalid persistent download state: expected an object")?;
            decode_saved_downloads(entries)
        }
    }
}

/// Deserialize already-owned rows one at a time: a row that cannot be read is
/// a warning, never a lost history.
fn decode_saved_downloads(
    entries: serde_json::Map<String, serde_json::Value>,
) -> Result<(BTreeMap<String, StoredJob>, Vec<String>), String> {
    let mut jobs = BTreeMap::new();
    let mut warnings = Vec::new();
    for (id, row) in entries {
        match serde_json::from_value::<StoredJob>(row) {
            Ok(stored) => {
                jobs.insert(id, stored);
            }
            Err(_) => warnings.push("A saved download could not be read and was skipped.".into()),
        }
    }
    if jobs.len() > MAX_JOBS {
        return Err("Too many persisted download jobs".into());
    }
    Ok((jobs, warnings))
}

/// Open one journal handle for decoding and any subsequent recovery. Missing
/// files are a first launch; a non-regular entry cannot start an empty history.
pub(crate) fn open_history(path: &Path) -> Result<Option<File>, String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000);
    }
    let file = match options.open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Opening transfer history: {e}")),
    };
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("The transfer history must be a regular file".into());
    }
    Ok(Some(file))
}

/// Preserve exactly the journal that failed decoding. A competing backup
/// name never grants overwrite permission. The profile manager owns its lock
/// throughout recovery and publishes fresh state only after a durable backup.
pub(crate) fn preserve_history(path: &Path, file: File) -> Result<PathBuf, String> {
    use crate::publication::VerifiedFile;
    let folder = path.parent().ok_or("Missing transfer history folder")?;
    let stem = path.file_stem().and_then(|name| name.to_str()).ok_or("Invalid transfer history filename")?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    let source = VerifiedFile::new(file, size).map_err(|e| e.to_string())?;
    let target = folder.join(format!("{stem}.unreadable-{}.json", uuid::Uuid::new_v4()));
    crate::publication::publish_verified_noreplace(path, &target, &source)
        .map_err(|error| format!("Cannot safely move the damaged transfer history aside. Check the folder permissions and retry: {error}"))?;
    sync_directory(folder)?;
    Ok(target)
}

/// A restored row normally uses its job ID as the map key. A row that failed
/// key/ID validation still carries a job ID the Downloads list can show, so
/// removal actions must find it by that identity too.
pub(super) fn state_key(state: &State, id: &str) -> Option<String> {
    if state.jobs.contains_key(id) {
        return Some(id.to_owned());
    }
    state
        .jobs
        .iter()
        .find(|(_, stored)| stored.job.id == id)
        .map(|(key, _)| key.clone())
}
