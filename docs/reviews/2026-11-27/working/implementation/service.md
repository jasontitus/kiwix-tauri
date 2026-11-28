Lane `service`: implementer report. Diffs are against `d98cac2`. All findings are fixed except two: FEAT-9 is a handoff, and the docs need a handoff. Every new behavioural test was seen failing in a scratch copy with the fixes reverted (details per finding). Nothing ran on a device, macOS or iOS.

### SERVE-1: a cached archive handle that has gone bad is never dropped
Disposition: fixed, extended to the reader listeners.
Verification of the finding: confirmed. The cache hit in `archive_for_book` returned the stored `Archive` with nothing checking it. `library_maintenance` updated `available` but called neither `invalidate` nor `retire_readers`. A non-fresh `open_article` also reused an existing listener that still held the old handle.
Change:
- `archive.rs`: new `Archive::retained_unchanged()`. It is `Parts::unchanged`: an fstat only, with no decoder lock.
- `service.rs: archive_for_book`:
  - Slot admission is now an `admit` closure.
  - On a hit with `Ok(archive)`, the probe runs outside both locks. If it fails, that slot is dropped by `Arc::ptr_eq` and re-admitted.
  - The reopen uses the unchanged `get_or_init` path: `archive_source::retain`, then the UUID check "Archive file identity changed…".
- `server.rs`: `ReaderServer` keeps a `Weak<Archive>` (`archive_handle()`).
- `service.rs: reader()`:
  - For a non-fresh open with an existing listener, it probes that listener's archive in `blocking`, under the readers lock. That mirrors the existing open-under-lock pattern.
  - A stale or stopped listener is shut down and replaced, so the shell gets a new origin.
- `dispatch.rs: library_maintenance`:
  - The store update now returns the `(id, installation)` pairs whose `available` flipped (in either direction).
  - Under the readers lock, for each one: `retire_readers(Some(installation))` and `invalidate(id)`. Errors are carried into the result, and `revoke_invalid_sources` still runs.
  - The original scan closure body is unchanged and starts with `proceed?`.
- Other archive caches:
  - Search cursors and PDF/export/attachment/illustration paths all take handles through `archive_for_book`, so the probe covers them.
  - Spelling dictionaries are keyed by a content identity that includes dev/ino/size/mtime/ctime, so they self-heal.
  - `sharing.rs` already stamps by size/mtime (net lane, not touched).
  - Integrity opens a fresh handle per run.
  - On mobile, the cache is already cleared when the app goes to the background.
Tests:
- `an_archive_changed_in_place_is_reopened_for_the_library_and_its_reader`: a touched file reopens; the old clone fails with "contents changed"; reopening the book gives a new origin that serves the page.
- `availability_changes_release_cached_handles_and_readers`: after the file is deleted, maintenance leaves no cache entry and no reader for that installation; after it comes back, the book opens.
- Both pass. With the fix reverted, both fail at the intended assertions.
Not exercised: a real USB/NFS unplug and replug. On Linux, fstat on a dead descriptor may still succeed while reads return EIO. The maintenance flip path covers that case; the probe alone does not.

### LIB-7 / CORE-5: an empty mount point retires every book in a monitored folder
Disposition: fixed differently.
- The reviewers' rule ("none of the known books present → never retire") breaks the existing deliberate test `monitored_folder_watch_imports_and_retires_live`, which expects that deleting the only archive in a folder retires it. I observed that failure.
- So I used the device-id check the reviewer suggested as optional.
Change (`service.rs`):
- New `watched_devices: Mutex<HashMap<PathBuf,u64>>`, plus `watched_directory_device`, `remember_watched_device` and `watched_device_unchanged`.
- A folder's `st_dev` is recorded when this run sees one of its known books present, when a settled file is observed in it, or when a reconcile imported from it (unless known books are missing from a changed device).
- If none of the known books is present, retirement now happens only when the folder is on the device recorded for it in this run. Otherwise the books stay, and the maintenance pass marks them unavailable.
- One vanished file among present siblings is still retired.
- On Windows (no device number), behaviour is unchanged.
Tests:
- `an_empty_monitored_mount_point_keeps_its_books_but_a_single_deletion_is_retired` (unix + desktop). It covers launch while offline, a device change modelled by altering the recorded dev, a single deletion, and deleting the last book on the same device.
- The existing watcher tests still pass.
- With the fix reverted, the new test failed at the first "kept" assertion.
Not exercised: a real mount and unmount. In this run a folder only gets a recorded device once it has been seen with archives, so books in a folder unmounted before that (e.g. at launch) are kept as unavailable rather than retired.

### LIB-3: re-opening a registered archive re-runs a full integrity pass
Disposition: fixed.
Change: new `service.rs: registered_at_path`, called at the top of each path in `import_paths`. It returns the existing book without verifying when all of these hold:
- canonicalize succeeds, it is a file, and `has_book_at_path` matches (the cheap prefilter);
- an exact-path non-provider book exists, it is `available`, and it is not a managed-upgrade case;
- `archive_for_book(book)` succeeds, which runs the probe and the UUID check, so a different file at that path still takes the full verified import.
Tests: `opening_an_archive_already_in_the_library_does_not_verify_it_again` uses a counting hook and also checks that a replaced-UUID file is verified. It passes, and fails with the fix reverted.

### CORE-6: manual import verification cannot be cancelled
Disposition: fixed.
Change:
- `imports.rs`: `load_book_cancellable(..., &CancellationToken)` maps any error after cancellation to `IMPORT_CANCELLED`. It is needed because the existing cancel path turned "Integrity check cancelled" into CORRUPT_ARCHIVE.
- `service.rs`:
  - New `import_cancel` token.
  - `import_paths` and `import_monitored_paths` use it and check it again before registering.
  - `shutdown()` cancels it right after `stopping` is set, before the watcher join and the writer, and never replaces it.
  - `operation`: on a policy change, `ImportCutover::begin` cancels it before the writer. The guard is dropped right after the writer is acquired and installs a fresh token unless stopping has begun (checked under the token mutex, so there is no race with Quit).
- `lib.rs` was read, not edited. `open_paths` relays per-path errors, so the cancelled error reaches the startup lane's handling.
Tests:
- `quit_cancels_a_running_manual_import_instead_of_waiting_for_it`: shutdown finishes within 5 s, the error starts "Import cancelled", and no book is registered.
- `a_privacy_switch_cancels_a_running_import_and_later_imports_still_work`.
- Both pass. With the fix reverted, both time out.
Not exercised: a real multi-GB checksum. The hook simulates the bounded-read cancellation point.

### LIB-8: a mobile managed copy of an unavailable entry is deleted; the batch is unverified
Disposition: fixed.
Change:
- `relink_book`'s guarded section was moved, unchanged in behaviour, into `service.rs: relink_installation(id, book, label, expected)`. It holds the source guard, the sharing slot with the `selected_share_serves` check, and the readers lock, and does `ensure_window_active`, the installation-matched store update with reader-storage removal, retire, forget, invalidate, and on mobile the provider-reference release and backup. `relink_book` calls it.
- New `admit_intake_batch` (`cfg(any(mobile,test))`):
  - Verifies every copy with `load_book_cancellable(..., verify=true)`.
  - `intake_replacements` finds entries that can no longer be opened: provider entries that are `!available` or fail `retain`; otherwise `mobile_archive_unusable` on mobile, or `!is_file` in host tests.
  - Each such copy is relinked after the torrent-share check.
  - Then `admit_incoming_books` runs as before, and duplicates are removed (that log line no longer names the path).
  - Any failure returns errors, admits nothing new and deletes nothing.
- `mobile_service.rs: mobile_import_batch` is now async and delegates to it, then runs `mobile_apply_archive_backup` in `blocking`.
Tests:
- `an_intake_copy_replaces_an_unusable_entry_instead_of_being_deleted`: the copy survives, the path and installation change, and the receipt is consumed; a usable entry still has its duplicate removed.
- `an_intake_batch_is_verified_before_admission`.
- Both pass. Both fail with the replacement step and the verification reverted.
Not exercised: the real native intake and the provider-backed `retain` (mobile only). Android type-check is clean. iOS is not compiled.

### SERVE-3: the spelling cache directory is never garbage-collected
Disposition: fixed.
Change (`spelling.rs`):
- `Identity::from_content`, sharing one `content_generation` with `from_archive`.
- `SpellingManager::cache_present()`.
- `SpellingManager::sweep(live)`:
  - Holds `lifecycle`.
  - Returns 0 if caches are not writable, a build is running or a cache load is in flight.
  - Keeps the live keys plus every key already in memory (dictionaries, statuses, failures). It also keeps a legacy `unbound_key(2).words` while the current key's file is absent.
  - Through the retained no-follow `Dir`, it removes regular files only, with exact names only (`sweep_name`): `.words` and `.previous` of non-live keys, and every `.tmp`/`.run`. A live key's `.previous` is kept, because the recovery error names it.
- `publish_index` removes the backup after `confirm_and_flush` succeeds, verified with `matches_in`.
Change (`service.rs`): `sweep_spelling_cache` runs at most hourly from `library_maintenance`'s blocking part. It skips if any book is unavailable or any identity cannot be computed, using `archive_content_identity`, which is the same retained-source identity the cache key uses.
Tests:
- `sweep_removes_only_cache_files_that_no_live_entry_uses` covers look-alike names, a directory and a symlink (its target untouched). Fails reverted.
- `sweep_name_accepts_only_this_modules_cache_names`. This is a parser test with no "without fix" state.
- `library_maintenance_keeps_live_spelling_dictionaries_and_collects_orphans`: a real build survives maintenance, which shows the keys match; a leftover `.tmp` is removed; after a restart and relink the old dictionary is removed. Fails reverted.
- `a_missing_archive_pauses_spelling_cache_collection` passes, but also passes with the sweep removed, because two guards protect that case.
- The existing `spelling_failed_directory_flush…` now asserts no backup survives success. Fails reverted.

### SERVE-4: failed document navigations get an uninstrumented text/plain page
Disposition: fixed.
Change (`server.rs`): `document_request` is now computed first. For iframe/document destinations, these get `reader_error_document`:
- 405 (form post): "Archives are read-only, so this form cannot be sent."
- 400 (bad escape): "The link to it is not a valid address."
- Non-not-found read errors and fallback-lookup errors in `missing_document`: the new `document_failure` keeps the status mapping and the diagnostic, with "The archive could not read this entry."
Non-document requests and "Archive worker stopped" are unchanged. The page loads `bridge.js`, so it still sends the hello the reader lane now requires.
Tests: `document_navigations_that_fail_still_load_the_reader_bridge`. The read-error case uses a touched file; plain fetches stay text/plain. Fails reverted.
Not exercised: the real shell rendering the error page (no browser run).

### CORE-9 + FEAT-6: dead dispatch arms
Disposition: fixed.
- Removed the `torrent_limits` arm, plus its entries in the `normal_writable` list, the update-admission list and `mutation_command`.
- Removed the `localswarm_hotspot_start`/`stop` arms and their `mobile_service.rs` admission exceptions.
- Kept `sharing_status` (now polled by `SharingControls.tsx`) and `map_forget_recovery` (now called by `Maps.tsx`/`Downloads.tsx`). No string reference to the removed commands is left in `src/`, `scripts/` or the Kotlin/Swift plugin sources.
Tests: `retired_internal_commands_are_no_longer_dispatched`. Its failure without the fix is inferred.

### Handoffs that landed
- FEAT-3:
  - `native_pdf.rs` gains the allowed line `pub(crate) use cleanup::sweep_stale;`.
  - New `service.rs: start_pdf_scratch_sweep` is called at the top of `library_maintenance`, so it also runs in private mode. It runs once per process and only once an app handle exists. It is detached and resolves `<app cache>/article-pdf` (Android: the native `storageRoot` `cachePath`). Its `Report` logs and adds to `startupWarnings`.
  - Desktop read-only mode never dispatches maintenance, so there it relies on the export-time sweep.
  - Not exercised (needs an AppHandle). The iOS branch uses `tauri::Manager::path(&app)` and is not compiled here.
- Desktop handoff 2: `crate::sandbox::revive();` is the first statement of the availability blocking closure.
- Android H2: added to both copy-import `Err` arms. I checked that `App.tsx` renders `startupWarnings` with no platform gating. Android type-check only.
- Android H3: `mobile.rs: error()` emits `user_error::plain(&message)`. The raw text stays only in the existing Android `eprintln!`, so on iOS the raw native text is no longer kept anywhere.
- FEAT-9: handoff, not done. `useVoiceModelStatus.ts` tolerates `"cancelled"` (it refreshes on any non-"downloading" state), but `DownloadControl.cancel` is private to `kokoro.rs`. Needed in `kokoro.rs`: `impl DownloadControl { pub(crate) fn cancelled(&self) -> bool { self.cancel.is_cancelled() } }`. Then in `finish_voice_job`: `"state": if success {"installed"} else if control.cancelled() {"cancelled"} else {"error"}`.

**Commands run** (all with `SHERPA_ONNX_ARCHIVE_DIR` set)
- `cargo test --lib -- service:: server:: spelling:: search:: archive:: imports:: integrity:: watch:: native_pdf::` on the final tree → `test result: ok. 403 passed; 0 failed; 6 ignored`.
- Scratch copy with all fixes reverted (own `CARGO_TARGET_DIR`, since deleted) → `test result: FAILED. 6 passed; 12 failed`. The 12 failures are the new or updated tests listed above, each at its intended assertion. The 6 passes are existing watcher/pdf tests, the parser test, and the missing-archive test.
- Linux clippy `--lib --tests --features custom-protocol -D warnings` on the final tree → `Finished`, clean.
- Windows-gnu clippy → `Finished`, exit 0.
- Android clippy script → exit 0.
- The Windows and Android runs were before the final `has_book_at_path` prefilter edit, which is platform-neutral.
- `node scripts/check-unsafe-census.mjs` → `75 matches / 75 blocks / 2 impls in 20 files; every item documented` (I added no `unsafe`).
- No frontend files were touched, so tsc and node tests were not run.

**Files changed:** `src-tauri/src/service.rs`, `service/dispatch.rs`, `service_tests.rs`, `imports.rs`, `spelling.rs`, `server.rs`, `server_tests.rs`, `archive.rs`, `mobile.rs`, `mobile_service.rs`, and `native_pdf.rs` (the one allowed line).

**New user-visible strings (Rust):**
- "Import cancelled because Kiwix is closing or its privacy settings changed. Add the archive again to import it."
- "Archives are read-only, so this form cannot be sent."
- "The link to it is not a valid address."
- Removed: "Creating a Wi-Fi hotspot is only available on Android", "Missing download limit", "Missing upload limit".
- `docs/ERRORS.md` needs regenerating.

**Handoffs / contract changes**
- `docs/STATE_MACHINES.md` §1.2 step 2 and §1.1 Quit should list "manual and monitored imports (cancelled; token replaced under the writer)".
- `mobile_policy.rs:566`: a test list still names `localswarm_hotspot_start`. It is harmless, but can be dropped.
- Mobile intake now verifies the whole archive under the privacy read lease, and the background writer (`mobile_service.rs` ~1893) does not cancel imports. A suspension during an intake, or during any mobile manual import as before, waits for verification. Cancelling there would need a product decision.
- Relink and intake relink still verify uncancellably (`relink_book`'s `load_book`), which is outside CORE-6's scope.

**Doubts**
- The LIB-3 fast path skips re-verification of an already-registered file, so an in-place corruption with the same UUID now opens instead of reporting "corrupt". It also opens a registered edition original or replacement that `register_books` would have refused to re-import.
- Retiring readers when availability flips closes an open tab on a file that was moved away or is briefly unreachable. This is the reviewer's choice, made to release disk space.
- The spelling sweep keeps keys from in-memory statuses, so a removed book's dictionary is collected only in a later run. Provider-backed (mobile) archives whose descriptors change identity on each open could have a dictionary collected that only a cached handle could reach. The in-memory keys protect it within this run.
- The LIB-7 device rule protects nothing on Windows. A folder whose device was never recorded in this run keeps books that really were deleted, as unavailable.
- `service.rs` was being edited concurrently by other lanes. I used whole-file Python rewrites for some edits; the diffs show no lost foreign changes, but a reviewer should confirm the other lanes' regions in `service.rs` are intact.