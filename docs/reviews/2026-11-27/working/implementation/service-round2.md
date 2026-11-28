Lane `service`, round 2: all ten items are done. The mobile build compiles again: the Android clippy script exits 0. Host clippy, Windows-gnu clippy and the lane tests (407 passed) are clean. Nothing ran on a device, macOS or iOS. iOS-only lines are marked "not compiled here".

### SERVICE-R1 — mobile did not compile (`has_book_at_path` is desktop/test-only)
Disposition: fixed in service.rs only; store.rs untouched.
Verification of the finding: confirmed. `Store::has_book_at_path` is gated `cfg(any(not(mobile), test))`.
Change: `service.rs: registered_at_path` — the prefilter is now `#[cfg(not(mobile))]`. Mobile goes straight to the exact store lookup; its imports come from its own staging and are few.
Tests: the Android clippy script now exits 0 with no errors in this lane's files. An intermediate run failed only in other lanes' files (store.rs dead code and crash.rs `is_multiple_of`); the final run is `Finished`, exit 0. No further mobile errors were hidden behind R1.

### SERVICE-R2 — a file renamed over a registered path was not detected
Disposition: fixed.
Verification of the finding: confirmed. The old descriptor keeps its own size and mtime, so the probe passes and `is_file` stays true.
Change:
- `registered_at_path`: after `archive_for_book` succeeds, the fast path is taken only if `library_files::archive_content_identity(&book)` equals `archive.content_identity()`. A mismatch or an error falls through to the verified import.
- `library_maintenance`'s availability pass checks each opened cached slot (new `Service::cached_archives()`, at most 16 slots, 4 on mobile) for a book that is still available. If the path's identity differs from the handle's, or the handle can no longer be read, the slot is invalidated and that book's readers are retired. Path stats stay off the `archive_for_book` hot path.
Tests: `a_file_renamed_over_a_registered_path_is_verified_and_its_old_handles_released`. It keeps a reader and a handle, renames a new-UUID file over the path, and checks that `retained_unchanged` still passes on the old handle. Re-importing then verifies and registers the new archive, and after maintenance the old cache slot and listener are gone. It passes, and fails without the identity check (at the "not verified" assertion). The maintenance half's failure without the fix is inferred, because the test stops earlier.

### SERVICE-R3 — a cancelled monitored import was remembered as a damaged file
Disposition: fixed.
Change: `record_watch_result` skips entries whose error is `IMPORT_CANCELLED`.
Tests: `a_cancelled_monitored_import_is_retried_not_remembered_as_damaged` checks that a cancelled file is not skipped later while a corrupt one is. It passes, and fails reverted.

### SERVICE-R4 — the cancellation notice was lost after a privacy switch
Disposition: fixed in the backend, as decided.
Change: in `import_paths`, a path that ends with `IMPORT_CANCELLED` while the app is not stopping calls `note_startup_warning(IMPORT_CANCELLED)`. That only happens when a cutover actually cancelled the import. Quit adds nothing, since no shell is left.
Tests:
- `a_privacy_switch_cancels_a_running_import_and_later_imports_still_work` now asserts that the snapshot's `startupWarnings` contains the sentence. It fails reverted.
- `quit_cancels_…` asserts that Quit adds no warning.

### SERVICE-R5 — readers were retired when a book became available again
Disposition: fixed.
Change: the maintenance list now carries `(id, installation, retire)`.
- A flip to available, or any other change, still invalidates the cache.
- Readers are retired only for flips to `available == false` and for R2 identity mismatches.
Tests: `availability_changes_release_cached_handles_and_readers` now opens a reader after the file comes back and asserts that it survives the flip to available. It fails reverted.

### SERVICE-R6 — background retirement and per-book zoom
Disposition: background part fixed; per-book zoom needed no change.
Change: `mobile_service.rs` begins an `ImportCutover` before `privacy_gate.write()` in two places, and drops it once the writer is held:
- the background retirement branch of `mobile_transition` (Android and iOS);
- `mobile_media_released` (iOS only, not compiled here).

`IMPORT_CANCELLED` now reads: "Import cancelled because Kiwix closed, moved to the background, or switched privacy mode. Add the archive again to import it."

Intake stays pending, checked by reading the code:
- A cancelled verification returns errors from `admit_intake_batch`.
- `mobile_copy_incoming` then returns without `mobile_acknowledge_incoming`, so the request stays in `state.incoming`. The shell re-reads `mobile_lifecycle_state` and offers it again.
- No copy is removed, because duplicate removal runs only after a successful admission.
- Residual: the cancelled attempt's staged copy stays in its own `mobile-imports-v1/<uuid>/` directory, and a retry copies again into a new one. This is the same as any other batch error before this change.

Per-book zoom: no change needed. Per-book zoom is saved with `set_book_zoom` (`src/api.ts:733`), not `save_settings`. `operation` gives that command an ordinary read lease and never a writer, so it cannot queue behind an import. CORE-6's `bookZoom` trigger does not exist.

Tests: `a_cancelled_intake_stays_pending_and_keeps_its_copy`. Under a cutover, the copy is kept, no book is added and the receipt is not consumed; the retry admits it. It passes. It tests the service-side contract and passes even without the mobile wiring; the mobile call sites are type-checked for Android only.

### SERVICE-R7 — files written under an unmounted mount point bypassed the device guard
Disposition: fixed.
Change: `watched_device_unchanged` is replaced by `watched_folder_evidence(dir, device, live, missing) -> (record, retire)`.
- `retire` requires the device recorded in this run to equal the current one.
- A device is recorded only on a live observation (archives present or admitted), and only when it equals the recorded one or no known book in the folder is missing.
- With no device recorded in this run, nothing is retired. This is the accepted residual.
- Windows: `(false, true)`, i.e. retirement as before.
- The `SettledFile` path computes `missing` from the store and records the device only on that same evidence.
Tests:
- `an_archive_written_under_an_unmounted_mount_point_does_not_retire_the_volumes_books` (unix): the volume's book is kept across two reconciles, and the device beneath is not adopted. It fails reverted.
- `an_empty_monitored_mount_point…` and `monitored_folder_watch_imports_and_retires_live` still pass.

### SERVICE-R8 — the spelling key assertion could not fail
Disposition: fixed.
Change: after the restart and before any relink, maintenance runs. The test asserts that the sweep ran and that the `.words` file survived. Only after that does it relink and reset the throttle.
Tests: a scratch mutation of `Identity::from_content` makes the test fail at "a live dictionary was collected after a restart".

### SERVICE-R9 — iOS kept no trace of raw native error text
Disposition: fixed.
Change: `mobile.rs: error()` calls `crate::diagnostics::record(&service.store, operation, Some(&message))` before emitting.
- Redaction: `record` never stores the message. It writes only a coarse category plus `safe_operation(operation)`, which maps names outside its allow-list (all of these mobile operations) to "application_operation".
- It is skipped in private and read-only mode, and when diagnostics are off.
- So no document text or paths are logged, and the diagnostic value is low.

### SERVICE-R10 — intake comment
Disposition: fixed. The comment now says the batch is not all-or-nothing: a relink committed before a later failure stays, and the retry resolves that copy as installed. No copy is removed on failure.

### mobile_policy.rs:566
Disposition: left alone; the file is not mine. Its test list still names the removed `localswarm_hotspot_start`, which is harmless.

**Commands run** (all with `SHERPA_ONNX_ARCHIVE_DIR` set)
- `cargo test --lib -- service:: server:: spelling:: search:: archive:: imports:: integrity:: watch:: native_pdf::` → `test result: ok. 407 passed; 0 failed; 6 ignored`. That is 403 plus 4 new tests.
- Scratch copy with the round-2 fixes reverted and the R8 mutation applied (`CARGO_TARGET_DIR` in scratch; deleted afterwards) → `test result: FAILED. 1 passed; 6 failed`. The six failures are the R2, R3, R4, R5, R7 and R8 tests, each at its intended assertion. The one that passed is the R6 intake test, which by design does not depend on the mobile call sites.
- Host clippy `--lib --tests --features custom-protocol -D warnings` → `Finished`, clean on the final tree. It was blocked for a while by other lanes' store.rs and crash.rs lints, which have since been fixed.
- Windows-gnu clippy → `Finished`, exit 0.
- Android clippy script → `Finished`, exit 0.
- `node scripts/check-unsafe-census.mjs` → `75 matches / 75 blocks / 2 impls in 20 files; every item documented`.

**Files changed this round:** `service.rs`, `service/dispatch.rs`, `service_tests.rs`, `imports.rs`, `mobile.rs`, `mobile_service.rs`.

**User-visible string:** `IMPORT_CANCELLED` now reads "Import cancelled because Kiwix closed, moved to the background, or switched privacy mode. Add the archive again to import it." `docs/ERRORS.md` needs regenerating.

**Handoffs (unchanged from round 1):**
- FEAT-9 needs the `DownloadControl::cancelled()` accessor in kokoro.rs.
- `docs/STATE_MACHINES.md` §1.1 and §1.2 should list import cancellation, now also on mobile background retirement.

**Doubts**
- The R2 maintenance check runs `archive_content_identity`, a retain with a shared flock, for each opened cached slot on every pass. That is at most 16 opens per 15 s on desktop and at most 4 per pass on mobile.
- A path that cannot be read at all during that check is skipped; the availability flip handles it.
- The R4 notice persists in `startupWarnings` for the rest of the process, until dismissed.
- The R6 background cutover also cancels a manual import running on Android when the app goes to the background, not only an intake. The user must start it again.
- The iOS `mobile_media_released` edit is not compiled here.

---
Follow-up note:
The features lane's `finish_voice_job` edit is in place and I am making no further edits. `service.rs:3782` reads `"state": if success { "installed" } else if control.cancelled() { "cancelled" } else { "error" }`, and `kokoro.rs:191` has the `DownloadControl::cancelled()` accessor it calls.

That also lands the FEAT-9 handoff from my earlier reports, so it no longer needs to be listed as open.

My round-2 report stands as sent. I did not re-run the build, tests or clippy after that edit appeared. Earlier in this round I edited `service.rs` with whole-file Python rewrites; the coordinator confirmed at the start of the round that those dropped no other lane's edit.