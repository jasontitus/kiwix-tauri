// SPDX-License-Identifier: GPL-3.0-or-later
//! Digest verification, durable publication and the recovery sweep for completed downloads.
#[allow(unused_imports)]
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FinalizeMode {
    /// A live transfer worker: its watch channel carries stop commands.
    Worker,
    /// A startup finalize before the manager is published to any caller.
    Startup,
    /// A recovery-sweep finalize whose row is already claimed as verifying.
    Sweep,
}

/// Expected digest and length are checked against the row, never the part.
pub(super) fn verify_expected_digest(
    snapshot: &StoredJob,
    downloaded: u64,
    digest: &str,
) -> Result<(), String> {
    if snapshot
        .job
        .sha256
        .as_ref()
        .is_some_and(|expected| expected != digest)
    {
        return Err("Downloaded archive failed its advertised SHA-256 checksum; cancel and start a new download".into());
    }
    if let Some(expected) = snapshot.expected_size {
        if downloaded != expected {
            return Err(format!(
                "Final download length {downloaded} differs from expected {expected}"
            ));
        }
    }
    Ok(())
}

/// Persist a successful pass before publication or a stop decision. A pause
/// that lands right after the hash must resume from the receipt, not from byte
/// zero of a multi-gigabyte part.
pub(super) fn record_verification_success(
    inner: &Inner,
    id: &str,
    identity: Option<PartIdentity>,
    digest: &str,
) -> Result<(), String> {
    let Some(receipt) = verification_receipt(
        identity,
        digest,
        VerificationOutcome::Passed,
        None,
    ) else {
        return Ok(());
    };
    inner.update(id, |stored| stored.verification = Some(receipt))
}

/// Persist a failed pass so a restart reuses the verdict instead of hashing the
/// same rejected bytes again. Failures never authorize publication.
pub(super) fn record_verification_failure(
    inner: &Inner,
    id: &str,
    identity: Option<PartIdentity>,
    digest: &str,
    error: &str,
) {
    let Some(receipt) = verification_receipt(
        identity,
        digest,
        VerificationOutcome::Failed,
        Some(error.to_owned()),
    ) else {
        return;
    };
    let _ = inner.update(id, |stored| stored.verification = Some(receipt));
}

/// A live worker may ignore an interrupt (foreground/background churn) but
/// never a pause, cancel, or a privacy/read-only revocation.
pub(super) fn worker_stop_action(
    inner: &Inner,
    signal: &watch::Receiver<Signal>,
) -> Result<Option<Signal>, String> {
    let action = *signal.borrow();
    if action == Signal::Run || (action == Signal::Interrupt && !finalize_forbidden(inner)) {
        Ok(None)
    } else {
        Ok(Some(action))
    }
}

/// Startup has no worker; sweep finalizers use ordinary worker stop ownership.
pub(super) fn local_stop_action(inner: &Inner, id: &str) -> Result<Option<Signal>, String> {
    let state = inner.state()?;
    let action = state.running.get(id).map(|running| *running.command.borrow());
    let status = state.jobs.get(id).map(|stored| stored.job.status.as_str()).unwrap_or_default();
    Ok(match action {
        Some(Signal::Cancel) => Some(Signal::Cancel),
        Some(Signal::Pause) => Some(Signal::Pause),
        _ if status == "cancelled" => Some(Signal::Cancel),
        _ if status == "paused" => Some(Signal::Pause),
        _ if finalize_forbidden(inner) => Some(Signal::Interrupt),
        _ => None,
    })
}

/// Publish bytes already hashed while retaining the exact inode. Live transfer
/// and sweep workers re-read the latest stop command under the same state lock
/// as publication. Startup has no published manager and uses its durable row.
pub(super) fn publish_verified_download(
    inner: &Inner,
    id: &str,
    file: std::fs::File,
    downloaded: u64,
    digest: &str,
    mode: FinalizeMode,
) -> Result<Outcome, String> {
    let snapshot = inner.snapshot(id)?;
    let (destination, part) = paths(&snapshot.job)?;
    // Keep the exact hashed inode open across persistence and native publication.
    let verified = crate::publication::VerifiedFile::new(file, downloaded)
        .map_err(|e| format!("Retaining verified download: {e}"))?;
    let identity = file_identity(verified.file())
        .map_err(|e| format!("Inspecting verified download: {e}"))?;
    // Publication and stop commands share this lock, giving them a clear ordering.
    let mut state = inner.state()?;
    match mode {
        FinalizeMode::Worker | FinalizeMode::Sweep => {
            let action = state
                .running
                .get(id)
                .map(|r| *r.command.borrow())
                .unwrap_or(Signal::Pause);
            if action != Signal::Run
                && !(action == Signal::Interrupt && !finalize_forbidden(inner))
            {
                return Ok(signal_outcome(action));
            }
        }
        FinalizeMode::Startup => {
            let status = state
                .jobs
                .get(id)
                .map(|stored| stored.job.status.as_str())
                .unwrap_or_default();
            if status == "cancelled" {
                return Ok(Outcome::Cancelled);
            }
            if status == "paused" {
                return Ok(Outcome::Paused);
            }
            if finalize_forbidden(inner) {
                return Ok(Outcome::Interrupted);
            }
        }
    }
    if !verified.matches_path(&part).map_err(|e| e.to_string())? {
        return Err("Download part changed before publication".into());
    }
    let previous = state.jobs.get(id).cloned().ok_or("Unknown download ID")?;
    let stored = state.jobs.get_mut(id).ok_or("Unknown download ID")?;
    stored.job.downloaded = downloaded;
    stored.job.total = Some(downloaded);
    let mut published_digest = [0u8; 32];
    hex::decode_to_slice(digest, &mut published_digest).map_err(|error| error.to_string())?;
    stored.publication = Some(PublicationIntent {
        size: downloaded,
        sha256: digest.to_string(),
    });
    // Job receipts report this digest once publication commits; the public
    // snapshot reads it from the durable intent, never from a duplicate key.
    stored.job.sha256 = Some(digest.to_string());
    // The receipt is durable before the move, so a crash between hash and
    // publication can finish from the same verified inode without rehashing.
    stored.verification = verification_receipt(
        identity,
        digest,
        VerificationOutcome::Passed,
        None,
    );
    stored.published_ownership = stored.part_ownership.as_ref()
        .map(|entry| entry.renamed(&stored.job.file_name).with_publication(downloaded, Some(published_digest)));
    // Persist proof before the move: a crash after this point can recover a
    // missing part only by matching its exact verified bytes, never just its name.
    if let Err(error) = inner.save(&state) {
        state.jobs.insert(id.to_string(), previous);
        return Err(error);
    }
    let mut backup_result = Ok(());
    let publication_url = state.jobs.get(id).ok_or("Unknown download ID")?.job.url.as_str();
    if let Err(error) = inner.store.with_http_acquisition_destinations(&[&destination], publication_url, || {
        crate::publication::publish_verified_noreplace(&part, &destination, &verified)
            .map_err(|error| error.to_string())?;
        backup_result = crate::mobile_backup::archive_policy(verified.file(), &destination);
        Ok(())
    }) {
        // Withdraw this attempt's recovery proof after a rejected live commit.
        // Even byte-identical foreign entries must not turn a known identity
        // rejection into a successful adoption when the failed job is resumed.
        state
            .jobs
            .get_mut(id)
            .ok_or("Unknown download ID")?
            .publication = None;
        let rejected = state.jobs.get_mut(id).ok_or("Unknown download ID")?;
        rejected.job.sha256 = None;
        return Err(format!(
            "Publishing verified archive without overwrite: {error}"
        ));
    }
    backup_result?;
    // Native rename consumed our source. Any newly appearing part is not ours
    // to delete; failed commits likewise preserve potentially substituted files.
    let stored = state.jobs.get_mut(id).ok_or("Unknown download ID")?;
    stored.job.status = "complete".into();
    stored.job.interrupted = false;
    stored.job.speed = 0;
    stored.job.eta_seconds = None;
    stored.job.error = None;
    stored.job.updated_at = now();
    stored.segments = None;
    stored.segmented = false;
    stored.verification = None;
    if let Err(_error) = sync_directory(destination.parent().ok_or("Destination has no parent")?)
        .and_then(|_| inner.save(&state))
    {
        // A committed archive is never removed to roll back a failed state write.
        // The durable publication intent still allows recovery on the next start.
        state
            .jobs
            .get_mut(id)
            .ok_or("Unknown download ID")?
            .job
            .error = Some(
            "Archive added, but Kiwix could not finish saving its download history. Restart the app to finish tidying up".to_owned()
        );
    }
    Ok(Outcome::Complete)
}

/// Finish a row whose journal already records every byte as durable: a crash
/// after the last segment synced, or a worker that died before publication,
/// must not strand the row at 100%. No network is used and no admission is
/// required, so an offline device can still finish the archive. The status
/// becomes `verifying` before the pass starts, and a matching receipt skips
/// the pass entirely.
pub(super) fn finalize_durable_download(
    inner: &Inner,
    id: &str,
    mode: FinalizeMode,
) -> Result<Outcome, String> {
    if finalize_forbidden(inner) {
        // Never leave a claimed row pinned at verifying while policy forbids
        // the pass: it becomes a resumable interruption that the sweep retries.
        let _ = inner.update(id, |stored| {
            if stored.job.status == "verifying" {
                stored.job.status = "paused".into();
                stored.job.interrupted = true;
                stored.job.speed = 0;
                stored.job.eta_seconds = None;
                stored.job.error = None;
            }
        });
        return Ok(Outcome::Interrupted);
    }
    let snapshot = inner.snapshot(id)?;
    let total = durable_total(&snapshot).ok_or("Saved download layout is incomplete")?;
    let (_, part) = paths(&snapshot.job)?;
    let file = open_part(&part)?;
    if let Some(ownership) = &snapshot.part_ownership {
        if !ownership.matches(&part, &file)? {
            return Err(
                "The downloaded file was replaced before it could be finished; resume to download it again"
                    .into(),
            );
        }
    }
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if metadata.len() != total {
        return Err(
            "The downloaded file is not the expected size any more; resume to download it again"
                .into(),
        );
    }
    if total < 80 {
        return Err("Downloaded file is too short to be a ZIM archive".into());
    }
    #[cfg(test)]
    if let Some(gate) = inner.finalize_gate.lock().unwrap().take() {
        let _ = gate.entered.send(());
        let _ = gate.release.recv();
    }
    // A sweep already claimed verifying under the state lock. Never repeat
    // that transition here: it could erase pause/cancel arriving after claim.
    if mode == FinalizeMode::Sweep {
        if let Some(action) = local_stop_action(inner, id)? {
            return Ok(signal_outcome(action));
        }
    } else {
        inner.update(id, |stored| {
            stored.job.status = "verifying".into();
            stored.job.interrupted = false;
            stored.job.downloaded = total;
            stored.job.speed = 0;
            stored.job.eta_seconds = None;
            stored.job.error = None;
        })?;
    }
    let mut reader =
        std::io::BufReader::with_capacity(WRITE_BUDGET, file.try_clone().map_err(|e| e.to_string())?);
    let mut magic = [0u8; 4];
    reader
        .read_exact(&mut magic)
        .map_err(|e| format!("Reading ZIM signature: {e}"))?;
    if magic != *b"ZIM\x04" {
        return Err("Downloaded resource is not a ZIM archive (invalid magic)".into());
    }
    reader
        .seek(std::io::SeekFrom::Start(0))
        .map_err(|e| e.to_string())?;
    let identity = file_identity(&file)?;
    if let Some(receipt) = matching_receipt(&snapshot, identity.as_ref()) {
        if receipt.identity.size != total {
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
                verify_expected_digest(&snapshot, total, &digest)?;
                if let Some(action) = local_stop_action(inner, id)? {
                    return Ok(signal_outcome(action));
                }
                return publish_verified_download(inner, id, file, total, &digest, mode);
            }
        }
    }
    #[cfg(test)]
    inner.hash_passes.fetch_add(1, Ordering::SeqCst);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; WRITE_BUDGET];
    let mut hashed = 0u64;
    // The stop/privacy probe takes the state lock and two store locks. Checking
    // it once per read made a multi-GB verify pay one lock round per MiB, so it
    // runs at most once per this many bytes; the final check below still runs
    // before publication.
    let mut next_stop_check = 0u64;
    loop {
        if hashed >= next_stop_check {
            if let Some(action) = local_stop_action(inner, id)? {
                return Ok(signal_outcome(action));
            }
            next_stop_check = hashed.saturating_add(STOP_CHECK_INTERVAL);
        }
        let count = reader
            .read(&mut buffer)
            .map_err(|e| format!("Reading download checksum: {e}"))?;
        if count == 0 {
            break;
        }
        hashed = hashed
            .checked_add(count as u64)
            .ok_or("Download checksum length overflow")?;
        hasher.update(&buffer[..count]);
        #[cfg(test)]
        {
            let delay = inner.hash_delay.load(Ordering::Acquire);
            if delay > 0 {
                std::thread::sleep(Duration::from_millis(delay));
            }
        }
    }
    if hashed != total {
        return Err("Download changed length during checksum verification".into());
    }
    let after = file_identity(&file)?;
    if after != identity {
        return Err("Download changed identity during checksum verification".into());
    }
    let digest = hex::encode(hasher.finalize());
    if let Err(error) = verify_expected_digest(&snapshot, total, &digest) {
        record_verification_failure(inner, id, after, &digest, &error);
        return Err(error);
    }
    record_verification_success(inner, id, after, &digest)?;
    if let Some(action) = local_stop_action(inner, id)? {
        return Ok(signal_outcome(action));
    }
    publish_verified_download(inner, id, file, total, &digest, mode)
}

/// Re-check a live manager for rows whose bytes are durable but whose worker
/// is gone. A bounded ticker covers a worker lost without unwinding, while a
/// foreground resume calls the same sweep directly.
pub(super) fn start_recovery_sweep(inner: &Arc<Inner>) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let weak = Arc::downgrade(inner);
    handle.spawn(async move {
        let mut ticker = tokio::time::interval(RECOVERY_SWEEP_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // The first tick is immediate; startup already performed its own pass.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let Some(inner) = weak.upgrade() else {
                break;
            };
            // The timer is independent of command dispatch. Foreground sweeps
            // inherit their caller's lease and must not acquire a nested read.
            let _admission = inner.admission.admit().await;
            if let Err(error) = sweep_durable_finalize(Arc::clone(&inner)).await {
                crate::diagnostics::record(
                    &inner.store,
                    "downloads",
                    Some(&format!("Download recovery sweep: {error}")),
                );
            }
        }
    });
}

pub(super) async fn sweep_durable_finalize(inner: Arc<Inner>) -> Result<(), String> {
    // Private/read-only policy forbids the finalize pass. The manager (and
    // this ticker) outlive the mode switch, so claiming a durable row here
    // would only bounce it verifying → paused+interrupted, rewriting the
    // journal three times every tick for as long as the mode lasts.
    if finalize_forbidden(&inner) {
        return Ok(());
    }
    let candidates: Vec<String> = {
        let state = inner.state()?;
        state.jobs.iter().filter_map(|(id, stored)| {
            let worker_gone = state.running.get(id)
                .is_none_or(|running| running.finished.has_changed().is_err());
            let eligible = matches!(stored.job.status.as_str(), "downloading" | "queued" | "verifying")
                || (stored.job.status == "paused" && stored.job.interrupted);
            (worker_gone && eligible && durable_finalize_candidate(stored)).then(|| id.clone())
        }).collect()
    };
    for id in candidates {
        // A mode switch can land while an earlier candidate was finalizing.
        if finalize_forbidden(&inner) {
            return Ok(());
        }
        let running = {
            let mut state = inner.state()?;
            let Some(stored) = state.jobs.get(&id) else { continue };
            let worker_gone = state.running.get(&id)
                .is_none_or(|running| running.finished.has_changed().is_err());
            let eligible = matches!(stored.job.status.as_str(), "downloading" | "queued" | "verifying")
                || (stored.job.status == "paused" && stored.job.interrupted);
            if !worker_gone || !eligible || !durable_finalize_candidate(stored) {
                continue;
            }
            let previous = stored.clone();
            let stored = state.jobs.get_mut(&id).ok_or("Unknown download ID")?;
            stored.job.status = "verifying".into();
            stored.job.interrupted = false;
            stored.job.speed = 0;
            stored.job.eta_seconds = None;
            stored.job.error = None;
            stored.job.updated_at = now();
            if let Err(error) = inner.save(&state) {
                state.jobs.insert(id.clone(), previous);
                return Err(error);
            }
            // Claim, spawn and retain completion before releasing the same
            // lock pause/resume uses. Dropping this sweep cannot detach file
            // ownership from its completion signal or result settlement.
            spawn_download_worker(&inner, &mut state, &id, WorkerMode::DurableFinalize);
            state.running.get(&id).cloned().ok_or("Download finalizer did not start")?
        };
        let mut finished = running.finished;
        while !*finished.borrow() {
            finished.changed().await
                .map_err(|_| "Download finalizer exited without reporting its result")?;
        }
    }
    Ok(())
}
