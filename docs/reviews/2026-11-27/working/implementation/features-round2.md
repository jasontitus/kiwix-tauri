# Lane `features` — follow-up report (changes since the first report only)

R1, R4, R5 and R6 are addressed in my owned files. Lane tests pass, and host and Windows-target clippy are clean on the final tree. Nothing was run on macOS, iOS, Android or Windows.

One point needs your confirmation. A later Delete returns `Err` again while a proven record's reclaim is still failing (for example the drive is still unplugged). I read "returns `Ok` once nothing it owns remains" that way; the reviewer's wording was "later calls return `Ok`". It is one `match` in `delete_download` if you want the other reading.

Diff as before: `git diff d98cac2 -- src-tauri/src/maps.rs src-tauri/src/maps src-tauri/src/maps_tests.rs`

---

### FEATURES-R1 — demotion irreversible, Delete reported success
Disposition: fixed (reviewer items 1, 2 and 3 adopted).

Verification of the finding: Confirmed by reading. Nothing reclaimed a recovery record after demotion, and the demoting `delete_download` returned `Ok`.

Change (`maps/manager.rs`, `maps/partials.rs`, `maps.rs`):
- **Retry on explicit Delete only.** New `retry_recovery_records`, called from `delete_download`'s current-job branch and nowhere else.
  - Each record with a journaled inode goes through the same `reclaim_partial` as an owned partial, under `with_acquisition_destinations`, with every step journaled. `Reclaimed` drops the record.
  - Records with no journaled identity are never passed to reclaim.
  - A refusal by the library guard counts as "not attempted".
- **Vanished records are dropped.** New `partials.rs: record_vanished`: the destination opens and none of the record's partial, publication or cleanup-capability names exists. Such a record is dropped without touching anything. An unreachable destination, or any other error, is not "vanished".
- **Delete result.** The call that demotes returns `Err("Kiwix could not remove a partial map download, so it was kept at <path>. <reason>")`. So does a later call whose proven reclaim fails again.
- **Reason kept (R5).** `reclaim_job_partial` now returns `Option<(path, reason)>`. The reason is written into the recovery sentence of `job.error` (`append_recovery_warning_because`), so it disappears together with the warning. For a foreign file at the name the reason is a new string, "The file there is not the one this download wrote, so Kiwix left it alone".
- **Accurate path (R5).** `MapJob::recovery_path` reports `<destination>/<.streetzim-cleanup-uuid>/<name>` when the entry is actually inside its capture directory. `forget_recovery` accepts either spelling. No journal field was added, because `OwnedPartial` has `deny_unknown_fields`.

Still never deleting unproven bytes: the only unlink remains inside `reclaim_owned_file`, after directory identity, inode identity, exclusive capture and re-verification inside the capability.

**`cancel()` decision: keep the demotion and return `Ok`.** Evidence:
- `mobile_policy.rs:472` pushes a `cancel()` error into `internet_errors`. `NETWORK.acknowledge(retirement_revision)` is then skipped, so `retirement_pending()` stays true.
- While it is pending, `mobile_network_check` refuses every Internet transfer with "The previous network connection is still being closed…", and `internet_admission_of` reports `Retiring`, which skips the automatic resume.
- Reconcile is driven only by `network-policy-changed` events and by startup (`mobile.rs:493` and `:574`, the latter with `?`). There is no retry.
- `dispatch.rs:1396` (switch to private or read-only mode) uses `cancel().await?`, so an `Err` there aborts the settings save.
- `mobile_service.rs:2301` only collects the error.
- `cancel()` cannot tell its callers apart, so the user's `map_cancel` also gets `Ok`. The kept file and reason are on the job, and the user's next Delete reclaims it.

Correction to my first report: shutdown does not call `cancel()`; it uses `pause()`.

Tests (`maps_tests.rs`):
- `deleting_a_map_whose_folder_is_unreachable_keeps_it_until_delete_can_finish` — the reviewer's case: `Err` and kept, still `Err` while unreachable, durable across restart, folder renamed back, Delete leaves no file and no warning.
- `deleting_a_map_whose_folder_was_replaced_says_so_once_and_reports_nothing_that_is_not_there`.
- `a_failed_cleanup_capture_does_not_strand_the_partial` — failing capture through `reclaim_partial_with_capture`, then a clean retry.
- `a_partial_stuck_inside_its_cleanup_capability_is_reported_where_it_is`.
- `delete_reclaims_a_proven_partial_that_an_automatic_cancel_had_to_keep`.
- `a_deleted_map_with_kept_files_reports_them_until_each_warning_is_dismissed` — updated: first call `Err`, repeat `Ok`.
- `cancelling_…_demotes_it_instead_of_failing_forever` — unchanged.
- Replaced: `deleting_a_map_whose_partial_cannot_be_reclaimed_keeps_the_bytes_and_unblocks_the_next_map`, by the tests above.
- Two older tests (`map_cleanup_persists_reclaimed_publication_before_reporting_older_recovery_warnings`, `historical_recovery_warnings_remain_visible_when_carried_to_another_map`) pinned that a proven leftover from an earlier job survives Delete. Under the adopted fix it is reclaimed, so they now use a foreign file at the journaled name to keep their intent.

All pass. Observed failing in two neutralization passes: Delete forced to `Ok` and no capture probe (7 failed, including the R4 tests); retry disabled (5 failed).

Not exercised: the Windows sharing-violation trigger; a real unplugged volume.

### FEATURES-R4 — failed sync still led to a checkpoint
Disposition: fixed.

Change:
- `checkpoint_output` returns `CompositionFailure`. A flush or sync failure is `retain: true, checkpoint_current: false`, so the failure path keeps the previous durable checkpoint.
- A journal-save failure keeps the old classification.

Tests:
- `a_checkpoint_whose_sync_fails_keeps_the_last_durable_checkpoint` — end to end through `compose`. The checkpoint stays at 0 and the job then resumes into a byte-identical archive.
- The existing rejected-write test also asserts `retain && !checkpoint_current`.
- Both observed failing with `checkpoint_current: true`.

The first test needed one new `#[cfg(test)]` hook, `Inner.fail_sync_from`; no existing injection point reached the sync. It injects the error and does not exercise a real failing `fsync`.

### FEATURES-R6 — FEAT-8 test could not fail
Disposition: test deleted. A holder count would need an edit in `power_activity.rs`, which I do not own, and the guarded line only exists on macOS. FEAT-8 is **not compiled here, untested**.

---

## Commands run (final result lines)
- `cargo test … --lib -- maps:: kokoro native_pdf` (final tree) → `test result: ok. 132 passed; 0 failed; 3 ignored`
- `cargo test … --lib -- map pdf kokoro voice speech power_activity` → `test result: ok. 182 passed; 0 failed; 3 ignored`
- Neutralization pass 1 → `FAILED. 59 passed; 7 failed`; pass 2 → `FAILED. 61 passed; 5 failed`; files restored and compared byte-for-byte.
- Host clippy → exit 0. The first attempt failed only in `store_tests.rs` (another lane).
- Windows-gnu clippy → exit 0. Earlier attempts failed only in `library_files_tests.rs` and `acquisition/transfer.rs` (other lanes).
- `npx --no-install tsc --noEmit` → exit 0.
- `node scripts/check-unsafe-census.mjs` → `75 matches / 75 blocks / 2 impls in 20 files; every item documented`.

## Files changed since the first report
`src-tauri/src/maps.rs`, `src-tauri/src/maps/manager.rs`, `src-tauri/src/maps/partials.rs`, `src-tauri/src/maps_tests.rs`.

New Rust user-visible strings (for `docs/ERRORS.md` regeneration):
- "Kiwix could not remove a partial map download, so it was kept at {path}. {reason}"
- "The file there is not the one this download wrote, so Kiwix left it alone"

New test-only hooks: `Inner.fail_capture`, `Inner.fail_sync_from`.

## Updated frontend contract for `map_delete_download`
- **`Err(message)`**: a removal this call attempted did not complete.
  - Either the old cases (active job, read-only, library reservation, published file changed, journal not saved), or the new "…so it was kept at <path>. <reason>".
  - In the new case the job stays listed as `cancelled` with `recoveryPaths` and the same reason inside `error`; Get map is not vetoed.
  - Pressing Delete again retries.
- **`Ok` with `map_status` then `null`**: everything is gone, including earlier kept files that could now be proven and removed, or that are no longer there.
- **`Ok` with `map_status` still returning the job**: only files that were never Kiwix's to remove remain (a foreign file at the journaled name, or a reservation that was never proven). `recoveryPaths` is non-empty and `error` holds the warning without a reason. Only Dismiss (`map_forget_recovery`) clears these.
- `recoveryPaths` strings may now point inside a `.streetzim-cleanup-<uuid>/` directory. Pass them back unchanged.
- `map_cancel` returns `Ok` and puts the warning and reason on the job.

## Doubts
- **The repeated-`Err` reading above.** For a folder that is permanently gone, Delete keeps failing until the warning is dismissed, and Dismiss has no UI yet.
- **Delete of one job now removes proven leftovers of earlier jobs.** That is the adopted widening. They are always hidden `.streetzim-*.part` files matching a journaled inode, but it is a behaviour change, and two older tests were adjusted for it.
- **Vanished-record drop ignores directory identity.** After "folder recreated", the second Delete stops reporting bytes the user moved elsewhere, and a dropped record with a publication name gives up its monitored-path claim.
- **Reason text is the raw reclaim error.** OS errors read like "No such file or directory (os error 2)", and the capture-failure text includes a `Debug`-printed identity. This is the same text the base build returned.
- **`status()` now stats the capture path** for each recovery record that has a capture. That state is rare, but it is I/O on a polled path.