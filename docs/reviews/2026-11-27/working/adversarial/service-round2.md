## Verdict for lane service (round 2): only non-blocking findings

The lane is not closed. R1, R3, R5, R8, R9 and R10 are closed and R2 and R7 are closed with follow-ups. R4 and R6 are only partly closed: R6's change creates a new storage leak on mobile (SERVICE-R2-3), and R4's notice can stay hidden or go stale (SERVICE-R2-2). Three Should-fix items and two Nits are below.

Android type-check: `Finished`, exit 0. Lane tests: green on a re-run, after two unrelated flaky failures (details under "What I ran").

I reviewed `git diff 6b69d87` for `service.rs`, `dispatch.rs`, `imports.rs`, `mobile.rs`, `mobile_service.rs` and `service_tests.rs`; the working tree matched HEAD `0b669c7`. The features lane's FEAT-9 lines in `finish_voice_job` and the `store.rs` notice edits are other lanes' work, and I did not review them.

### SERVICE-R2-1 — The new maintenance identity check also re-acquires provider-backed archives every pass
Severity: Should-fix
Fix under review: SERVICE-R2
Evidence class: inference (whether provider descriptors keep the same identity between opens cannot be checked here); the code path is source-traced
Location: `service/dispatch.rs:1835-1854` (the `stale` list), `provider_reference.rs:169-193` (`retain`)
Trigger: on Android or iOS, a cached archive opened through a document provider while the app is in the foreground, at every due `library_maintenance` pass.
Causal path:
- `cached_archives()` does not exclude `book.source_reference.is_some()`.
- For each such slot, `archive_content_identity(book)` calls `archive_source::retain`, which makes a synchronous native `acquireExternalArchive` call (lease plus duplicated descriptors) on every pass, up to 4 per pass.
- The identity of a provider descriptor is never pinned: `retain` compares nothing.
- A provider that hands out a proxy or cache descriptor (for example an appfuse proxy or a cloud provider's fresh copy) gives a new dev/inode on each open. Then `held != current` on every pass, so the slot is invalidated and `retire_readers` closes the open tab each time.
- Renaming over a path is a filesystem case; providers have no path to rename over.
Smallest correct fix: in that `filter_map`, skip books where `source_reference.is_some()`. `registered_at_path` is already limited to filesystem books.

### SERVICE-R2-2 — The R4 cancellation notice can stay hidden, or stay after it is stale
Severity: Should-fix (part of it hands off to the frontend lane)
Fix under review: SERVICE-R4
Evidence class: source-traced
Location: `service.rs:1847-1852`, `service.rs:973` (`note_startup_warning`), `src/App.tsx:503`, `:3720-3729`
Trigger 1: earlier in this shell session the user dismissed any startup notice (PDF cleanup, notifications, profile). A privacy switch then cancels an import.
- `startupNoticeDismissed` is one sticky boolean per mounted shell, so the new sentence is never shown. That is the exact silent case R4 was meant to fix.
Trigger 2: the notice was shown, and the user then added the archive again successfully.
- The entry stays in `startup_warnings` for the rest of the process.
- It reappears on every reload or new window, telling them to "Add the archive again", which they already did.
- It also appears under the heading "Some saved items need attention.", which is the wrong context.
The lifetime (deduplicated, at most 32 entries, until restart) matches what the implementer described; the problem is the two cases above.
Smallest correct fix:
- Frontend: reset `startupNoticeDismissed` when the content of `startupWarnings` changes.
- Backend: remove the `IMPORT_CANCELLED` entry when a later `import_paths` call succeeds.

### SERVICE-R2-3 — Cancelling on background now routinely orphans a full staged copy on mobile
Severity: Should-fix (needs an owner decision)
Fix under review: SERVICE-R6, LIB-8
Evidence class: source-traced. That a screen timeout or lock triggers the background retirement branch is an inference from the lifecycle wiring; nothing ran on a device.
Location:
- `mobile_service.rs:1881-1886` (background cutover), `:1302-1317` (`mobile_copy_incoming`)
- `library_files.rs:2434` (`mobile_directory` creates a fresh `<uuid>` directory on each attempt)
- No orphan cleanup exists for `mobile-imports-v1`: the only one is `remove_empty_import_directory`, which runs when a book is deleted.
Trigger:
- Share a multi-GB ZIM into Kiwix (or use "Import a managed copy"), then switch apps or let the screen auto-lock during the minutes-long verification.
- Imports hold no keep-awake: `updateTransferScreen` is used only for transfers.
Causal path:
- The cutover cancels the verification, and the `IMPORT_CANCELLED` errors leave the incoming request pending. That part is correct: nothing is admitted, the receipt is not consumed, and no copy is deleted.
- The retry calls `mobile_staging()` again, which makes a new `<uuid>` directory, and the native side copies the whole archive again.
- The cancelled attempt's copy is never referenced or removed. It is invisible, unrecoverable from the UI, and as large as the archive. A second copy is also needed for the retry, which may then fail for lack of space.
- A cancelled picker-copy import leaves its staged copy behind in the same way, and the user has to pick the file again.
- Before R6, backgrounding waited (or was frozen) and the import then finished, with no leak.
On "is cancelling a manual Android import on backgrounding acceptable": only if the copy is not orphaned and the user is told. Today the user is told only through the R4 notice (see SERVICE-R2-2), and the copy leaks.
Smallest correct fix:
- On a verification that ends in `IMPORT_CANCELLED`, either record the staged paths on the pending incoming request and re-verify them on retry instead of copying again, or remove this attempt's own `<uuid>` staging directory, which the app owns exactly.
- Do the same for picker copies that are cancelled.
- If the owner prefers that imports finish rather than cancel, hold a keep-awake or foreground lease while a verification runs.

### SERVICE-R2-4 — Two cutover sources can overlap
Severity: Nit
Fix under review: SERVICE-R6
Evidence class: source-traced
Location: `service.rs` (`ImportCutover::drop`), `mobile_service.rs:1881`, `service.rs:3151`
Trigger:
1. A mobile privacy `save_settings` (cutover A) and the background retirement (cutover B) both cancel T1.
2. A gets the writer, and its guard installs T2.
3. An import that queued for the read lease between A's and B's writer requests then takes T2.
4. B waits for the whole checksum.
Settings saves are serialised with each other, but the background path is a second, independent source of cutovers.
Smallest correct fix: count the active cutovers, and install a fresh token only when the count drops to zero and the app is not stopping.

### SERVICE-R2-5 — The `SettledFile` device check copies the whole profile for every settled file
Severity: Nit
Fix under review: SERVICE-R7
Evidence class: source-traced
Location: `service.rs:2042-2051`
Causal path:
- `self.store.read()?.books.iter().any(... Path::new(&book.path).exists())` clones the whole profile for every settled file. `has_book_at_path` exists precisely to avoid that.
- It also stats every book in the folder on the async runtime.
Smallest correct fix: compute `missing` only when the recorded device differs from the current one (when it matches, the rule records anyway), or use a path-only store accessor.

**Per-item status**
- **R1: closed.** `registered_at_path`'s prefilter is now `#[cfg(not(mobile))]`. My `android-clippy.sh` run finished: `Finished dev … in 12.07s`, exit 0.
- **R2: closed for filesystem books; follow-up SERVICE-R2-1 for provider books.**
  - The compare is correct. Both sides use `ContentIdentity::from_handles` (per part: dev, inode, size, mtime, ctime), so the same inode always matches and a rename over the path or a remount always differs.
  - In `registered_at_path`, a mismatch falls through to the verified import.
  - Cost: at most 16 retains per 15 s pass on desktop. Each is a directory open, a part listing, an open, and a non-blocking `try_flock_shared`; a busy lock is an `Err` that is skipped, never a false "stale".
  - Locks: the pass runs while `library_maintenance` already holds `localswarm_sources`. `move_archives`, `remove_book` and relink all take that same lock through `localswarm_source_mutation`, so no concurrent move or delete of the same file can interleave. On Windows the brief extra handle cannot race a move's file phase.
  - The only new lock is the brief std `archives` mutex in `cached_archives`.
- **R3: closed.** `record_watch_result` skips `IMPORT_CANCELLED`, which covers both the watcher and maintenance callers.
- **R4: partly closed** (SERVICE-R2-2).
- **R5: closed.** Readers are retired only for flips to unavailable and for identity mismatches; the cache is invalidated on any flip.
- **R6: implemented correctly for lock order; follow-ups SERVICE-R2-3 and SERVICE-R2-4.**
  - `ImportCutover::begin` and `drop` take only the `import_cancel` std mutex, briefly and never across an await. The documented order (update_admission → transfer_gate → settings_writes → privacy_gate) is therefore unaffected.
  - The claim that a cancelled intake stays pending without data loss is confirmed (no acknowledge, no removal, receipt unconsumed), apart from the orphaned copy in SERVICE-R2-3.
  - The per-book zoom claim is correct: `set_book_zoom` takes a read lease, not a writer.
- **R7: closed** (Nit SERVICE-R2-5). I walked `(record, retire) = (live && (same || !missing), same)` through these cases:
  - Single deletion and deleting every book: retired.
  - Launch while offline: kept.
  - Files written under an unmounted mount point, by the settled-file path or by a later reconcile: no device is adopted and nothing is retired.
  - Bind mounts (same `st_dev`) and a remount with a new `st_dev`: kept.
  - Windows: as before.
  - Residuals: a folder that holds one entry deleted while the app was closed never gets a recorded device, so monitored retirement stays off for that folder. A file imported from the directory under the mount point is retired once the volume shadows it again.
- **R8: closed.** The restart-without-relink assertion runs on a writable manager; the later removal proves the sweep really runs there.
- **R9: closed.** `diagnostics::record` stores only a category and `safe_operation`; it never stores the message.
- **R10: closed.**

**Implementer's doubts**
- 16 retains per pass: acceptable on desktop. Not acceptable for provider books (SERVICE-R2-1).
- Skipping an unreadable path: acceptable, because the availability flip covers it.
- The R4 notice lasting until restart: a real problem (SERVICE-R2-2).
- Cancelling a manual Android import on background: needs an owner decision, and the orphaned copy must be fixed in any case (SERVICE-R2-3).
- iOS `mobile_media_released`: the call has the same shape as the Android-compiled one (`super::ImportCutover::begin(self.as_ref())` with `self: Arc<Self>`), but it is not compiled here.

**What I ran**
- Lane tests (`service:: server:: spelling:: search:: archive:: imports:: integrity:: watch:: native_pdf::`, with `SHERPA_ONNX_ARCHIVE_DIR` set):
  - Run 1: `FAILED. 406 passed; 1 failed`, in `spelling::tests::cold_validation_is_single_flight…`. It is `finish_load` unwrapping a load handle that the worker has already removed itself (`spelling.rs:410`): a timing flake in a test this lane did not touch. It passed 6 of 6 times in isolation.
  - Run 2: `FAILED. 406 passed; 1 failed`, in `integrity::tests::real_valid_and_checksum_corrupt_results_survive_restart`. `integrity.rs` has not changed since base, and the test passed 3 of 3 times in isolation.
  - Run 3: `test result: ok. 407 passed; 0 failed; 6 ignored`. Load average was about 3.6 on 4 cores, with other lanes building.
  - I count both failures as flaky under load and not caused by this lane.
- `android-clippy.sh`: `Finished`, exit 0.

**What I could not verify**
- No iOS compile.
- No device runs: Android/iOS lifecycle on screen lock, provider descriptor identity, a real remount.
- How the shell renders the startup notice after a privacy switch.
- I did not re-run host or Windows clippy.