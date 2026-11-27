# Lane `features` — report

All nine findings in the lane are handled: eight fixed in code, FEAT-7 fixed by dropping the ETA. Host tests, host clippy and Windows-target clippy are clean. Nothing was run on macOS, iOS, Android or Windows, and the Kokoro playback loop itself was not exercised (it needs the model and an audio device).

Full change: `git diff d98cac2 -- src-tauri/src/maps.rs src-tauri/src/maps src-tauri/src/maps_tests.rs src-tauri/src/kokoro.rs src-tauri/src/native_pdf.rs src-tauri/src/native_pdf/cleanup.rs src/types.ts`

---

### FEAT-1 — A resumable map whose partial cannot be reclaimed permanently blocks every map download
Disposition: fixed (Rust side), with one deliberate narrowing of the reviewer's proposal.

Verification of the finding: Confirmed by reading `start()`, `delete_download`, `cancel`, `reclaim_job_partial`, `new()` and `reclaim_owned_file`.
- No path cleared `resume` once reclaim failed.
- The `if job.resume.is_some()` branch in `start()` was unreachable: the guard a few lines above returns `Err` for every job with `resume.is_some()`, under the same lock, with no mutation in between.
- Reproduced: with the fix neutralized, the new tests fail with "This partial map download was moved or changed, so Kiwix kept it…" and "Map file ownership changed; no bytes were deleted".

Change:
- `maps/manager.rs: reclaim_job_partial` (only called by `delete_download` and `cancel`). When the ownership-checked reclaim returns `Preserved` or any error, the record moves from `owned_partial` into `recovery_records`, `resume`/`resumable` are cleared, the recovery warning is rebuilt, and the result is persisted as one checkpoint. No filesystem operation happens on that path.
- If that persist fails, the function returns `Err` and the in-memory job is left as it was, so a retry is possible.
- Narrowing: an error from the library guard `with_acquisition_destinations` (read-only mode, name reserved by an installed book) still returns `Err` with the checkpoint intact. Those are retryable and say nothing about the file; demoting there would also write the journal in read-only mode. The existing test `deleting_a_paused_map_requires_unreserved_owned_identity…` now also asserts the record is not demoted.
- `start()`: dead branch removed, with a comment tying it to the guard.
- `maps.rs` header: one sentence added describing the demotion.

Contract checks that still hold:
- Never deleting unproven bytes: bytes are unlinked only inside `reclaim_owned_file`, after the journaled directory identity and inode identity are proven and the entry is captured into the private capability. Demotion adds no unlink.
- Never presenting a partial map as usable: demotion never sets `path`, `published` or `complete`; only the unchanged `Published` arm does.

Tests (`maps_tests.rs`):
- `deleting_a_map_whose_partial_cannot_be_reclaimed_keeps_the_bytes_and_unblocks_the_next_map` — three cases: folder recreated, folder missing, file replaced by a foreign file.
- `cancelling_a_map_whose_partial_cannot_be_reclaimed_demotes_it_instead_of_failing_forever`.
- Both pass, and both were observed failing with the fix neutralized.

Not exercised: Windows and macOS runs of these tests; the UI half.

### FEAT-6 — `map_forget_recovery` has no caller; Delete always fails while recovery records exist
Disposition: fixed (Rust side plus `MapJob` type); the Dismiss button is for the frontend lane.

Verification of the finding: The dispatch arm takes `{path, privacyGeneration}`, requires normal writable mode and an active window, and calls `forget_recovery(&PathBuf::from(path))`, which compares against `destination.join(publication name or record name)`. Nothing in `src/` references it.

Change:
- `maps.rs: MapJob.recovery_paths` — `#[serde(default, skip_deserializing, skip_serializing_if = "Vec::is_empty")]`, filled only in `status()`. It is never written to or read from the journal.
- `MapJob::recovery_path(record)` is now the single source for the warning text, `recovery_paths` and `forget_recovery`.
- `delete_download`: once everything the job owns is reclaimed and `path` is cleared, remaining recovery records no longer produce `Err`. The job stays listed (`history_dismissed` stays false), `error` is set to only the recovery warning, and it returns `Ok(())`.
- Two existing tests that pinned the old `Err` were updated: `map_cleanup_persists_reclaimed_publication_before_reporting_older_recovery_warnings` and `historical_recovery_warnings_remain_visible_when_carried_to_another_map`.

Tests: `a_deleted_map_with_kept_files_reports_them_until_each_warning_is_dismissed` checks the serialized `recoveryPaths`, that the journal has no such key, that `remove_terminal` refuses, that dismissal leaves both files untouched, and that a second Delete then returns `Ok` with `status()` `None`. Passes; observed failing with the fix neutralized.

### FEAT-2 — Disk-full records a checkpoint past the bytes on disk
Disposition: fixed.

Verification of the finding: Read tokio 1.54.1 `src/fs/file.rs` (the version in `Cargo.lock`).
- `poll_write` returns `Ok(n)` straight after spawning the blocking write.
- `sync_all` and `into_std` call `complete_inflight`, which stores the error kind in `last_write_err` and returns nothing.
- Observed: with the flush removed, `checkpoint_output` returned `Ok` after a rejected write.

Change:
- `maps/compose.rs: write_observed` does `write_all` then `flush`. All three composition writes (inline, md5, source slice) use it before any hash or counter advances.
- `checkpoint_output` now takes `&mut File` and flushes before `sync_all`.
- A flush was also added before the final `sync_all`.
- After a rejected write the error-path checkpoint records the un-advanced `written` and hash; any partial tail is truncated on resume by the existing `set_len(checkpoint.written)`.

Tests: `a_write_the_file_rejects_is_reported_before_any_counter_or_checkpoint_advances` uses a read-only handle as the rejecting file. Passes; observed failing without the flushes.

Not exercised: a real full volume.

### FEAT-7 — `speed`/`etaSeconds` not shown and not honest
Disposition: fixed by dropping the ETA. An honest ETA cannot be derived cheaply: how much of the remainder needs the network is unknown until each block has been tried against the cache and local maps.

Change:
- `MapJob.eta_seconds` removed from the struct, the three struct literals, `types.ts`, and every reset site.
- `speed` is cleared when a block is served from the cache or a local candidate (`ChunkReader::served_without_network`); it was already cleared on error, finish, resume and startup.
- `RetainedMapJob.eta_seconds` (the Downloads row shape, always `None`) is untouched.

Tests: `a_block_served_without_the_network_clears_the_stale_transfer_rate`. Passes; observed failing without the fix.

### FEAT-8 — No idle-sleep assertion during map composition (macOS)
Disposition: fixed. **Not compiled here.**

Verification of the finding: `power_activity::Guard` is `{ &'static Activity, bool }` with `acquire(reason: &'static str) -> Self`. It is defined for every platform and is `Send`. `run` is spawned with `runtime.spawn`, so `Send` is required.

Change: `maps/manager.rs: MapManager::run` gets `#[cfg(target_os = "macos")] let _power = crate::power_activity::Guard::acquire("Downloading an offline map");` beside `_activity`. It is macOS-only on purpose: on Android the same guard takes a high-performance Wi-Fi lock.

Tests: `the_map_worker_can_hold_the_power_assertion_across_awaits` is a compile-time `Send` assertion on the platform-independent type. It passes on Linux; the gated line itself is unverified.

### FEAT-4 — One unpronounceable chunk ends Kokoro read-aloud
Disposition: fixed.

Verification of the finding: misaki-rs 0.9.0 `g2p.rs` substitutes `self.unk` ("❓") for unresolved tokens, and `Cargo.toml` disables the fallback. Measured with the real `G2P`:
- a Chinese line gives `"❓"`;
- a Cyrillic sentence gives `"❓ ❓ ❓ ❓ ❓"`, so the whitespace-only residue the reviewer inferred is real;
- a bare URL is partly pronounceable, so it is not a trigger.

Change (`kokoro.rs`):
- `speakable_phonemes` returns `None` when only `❓` and whitespace remain. `synthesize` and `Prefetch::finish` return `Result<Option<File>, String>`.
- `run` skips such a chunk and moves the playhead with `skip_chunk`. No prefetch is outstanding at that point, so the next chunk is synthesized by the loop.
- If the end is reached with chunks skipped and none played, the existing error "The natural voice found no pronounceable text" is returned.
- Engine `None` ("produced no audio") and cancellation remain errors as before.
- Addition beyond the brief: after a backward transport command (Rewind, Previous, backward Seek) an unpronounceable chunk is skipped backwards. Otherwise a 15-second rewind could never cross a long unpronounceable block. It turns forward at chunk 0, so it terminates.

Tests: `an_unpronounceable_chunk_is_skipped_and_the_playhead_moves_to_the_next_one` covers the real-G2P decision over an English/Chinese/English text and the playhead arithmetic. Passes. The decision part was observed failing when the helper returned the old error; the backward-skip assertions were added afterwards and are inferred.

Not exercised: the `run` loop itself, including the "all chunks skipped" error.

### FEAT-9 — Cancelling the voice download is shown as an error
Disposition: fixed in `kokoro.rs`; optional handoff for the event.

Change: `finish_download` does not set `state.error` when the download's cancel token is cancelled. That covers both Cancel and the privacy/read-only drain. The call still returns `Err`.

Tests: `stopping_a_voice_download_is_not_shown_as_an_error` also asserts that a genuine failure is still recorded. Passes; observed failing without the fix.

### FEAT-3 — Article-PDF scratch never removed after a crash
Disposition: fixed for the export path; startup call is a handoff.

Verification of the finding: Removal existed only in `Owner::drop` and `retry_cleanup` over `PENDING`.
- `PENDING` holds only failed removals, not live jobs, so "not in PENDING" alone would not protect a running export.
- Lifetime ceilings in code: render 16 min (print/Android) or 90 s; macOS print panel a further 16 min + 20 s; share 120 s; iOS picker 90 s.
- A lease held by native UI (an open print dialog, GTK print completion) has no bound.

Change:
- `native_pdf/cleanup.rs`: new `LIVE` set (inserted in `Files::new`, removed at the end of `Owner::drop`, after any `PENDING` insert) and `sweep_stale(root, report)`.
- The sweep removes only real directories directly under `root` named exactly `pdf-<uuid>` (parse and re-format), whose own mtime is at least 1 hour old, and that are in neither `LIVE` nor `PENDING`.
- Symlinks are skipped (`symlink_metadata`), and `remove_dir_all` does not traverse links.
- A failed removal goes through the existing `fail()`: reported once and retained in `PENDING`. Sweeps are serialized.
- `native_pdf.rs: render` starts the sweep in a detached `spawn_blocking` before `OwnedPdf::create`.

Tests: `stale_sweep_removes_only_abandoned_pdf_scratch` checks that the abandoned tree is removed and that the following all survive: a recent directory, a live lease, look-alike names, a plain file, a symlink child, and the target of a symlink inside the removed tree. Passes; observed failing with the `LIVE` check removed. The age and name checks were not separately mutated. The `PENDING` exclusion is untested, because `PENDING` is a process global that other tests' shutdown paths drain.

---

## Commands run (final result lines)
- `cargo check --locked … --lib --tests` → `Finished`
- `cargo test … --lib -- kokoro native_pdf maps::` (final tree) → `test result: ok. 128 passed; 0 failed; 3 ignored`
- `cargo test … --lib -- map pdf kokoro voice speech power_activity` → `test result: ok. 178 passed; 0 failed; 3 ignored` (run before the last Kokoro backward-skip edit)
- Neutralization run (fixes temporarily disabled, then restored from copies) → `test result: FAILED. 73 passed; 10 failed`; the ten were the eight new tests above (all except the `Send` assertion and the journal test) plus the two updated ones
- `cargo clippy … --lib --tests --features custom-protocol -- -D warnings` → exit 0 (re-run after the last edit)
- Windows-gnu clippy command from the brief → `Finished` (run before the last Kokoro edit, which is platform-neutral)
- `npx --no-install tsc --noEmit` → no output
- `node scripts/check-unsafe-census.mjs` → `74 matches / 74 blocks / 2 impls in 19 files; every item documented`
- `node --test scripts/test-*.mjs` → `pass 417 / fail 5 / skipped 3`. The five failures are `isUserCancelledError is not defined` and `displayedFrame is not defined`, in files outside this lane.

## Files changed
`src-tauri/src/maps.rs`, `src-tauri/src/maps/manager.rs`, `src-tauri/src/maps/compose.rs`, `src-tauri/src/maps_tests.rs`, `src-tauri/src/kokoro.rs`, `src-tauri/src/native_pdf.rs`, `src-tauri/src/native_pdf/cleanup.rs`, `src/types.ts` (`MapJob` only).

New user-visible strings: none. Removed Rust error strings (`docs/ERRORS.md` needs regenerating): "Map file ownership changed; no bytes were deleted" and "Map recovery warnings remain unresolved; this download stays visible and no unproven files were deleted".

## Frontend contract

**`recoveryPaths?: string[]` on `map_status`**
- Present only when non-empty. Absent in private or read-only mode, where `map_status` returns the raw journal.
- Each string can be passed unchanged: `api("map_forget_recovery", { path, privacyGeneration })` returns `null`.
- It needs normal writable mode and an active window.
- A stale path fails with "This map recovery entry is no longer in the current download. Refresh and try again".
- Dismissal never touches the file.
- When the last entry is dismissed, the warning sentence leaves `error` and `canRemoveFromList` can become true. The job itself stays listed as `cancelled` until Delete or remove-from-list.

**`map_delete_download` result**
- `Err`: nothing was changed that cannot be retried (active job, read-only, library reservation, published file changed, journal not saved).
- `Ok` (`null`) with `map_status` then `null`: everything is gone.
- `Ok` with `map_status` still returning the job: files were kept. The job has `status: "cancelled"`, `resumable: false`, `canRemoveFromList: false`, non-empty `recoveryPaths`, and `error` = "Some earlier map downloads could not be cleaned up. The files were kept here: …". Get map is no longer vetoed.
- `Maps.tsx` currently does `setJob(null)` on success. It must re-read `map_status` after Delete instead of assuming removal.
- `map_cancel` behaves the same way: `Ok`, with the warning on the job.

**`speed` and ETA**
- `speed?: number` is bytes per second: the mean rate of the block being fetched from the network, kept while the next block is prepared. It is 0 when the latest block came from the cache or a local map, on error, and when idle.
- Render the rate when `speed > 0` on an active job.
- `etaSeconds` no longer exists on `MapJob`. `downloadLiveSummary` takes a `DownloadJob` and does not apply.

## Journal compatibility
- No journal field was added.
- A journal from the previous build loads unchanged; a leftover `etaSeconds` key is ignored and disappears at the next persist. Test: `a_journal_written_by_the_previous_build_loads_unchanged`.
- Journals from this build differ only in that `recoveryRecords` can gain an entry on Delete/cancel (the previous build already wrote such entries from `start()`) and `etaSeconds` is never written. The previous build reads them.

## Handoffs
1. **`service.rs` (FEAT-3 startup)**: off the main thread at service start, call the sweep on `<app cache>/article-pdf` (Android: the native `cachePath`). This needs `pub(crate) use cleanup::sweep_stale;` in `native_pdf.rs` and a `Report`. Without it, leftovers are removed only at the next PDF export.
2. **`service.rs: finish_voice_job` (FEAT-9, optional)**: it still emits `state: "error"` with the cancel text on `kiwix-speech-model`. The UI only uses that event to refresh, so the alert is already gone; emitting `"cancelled"` would be tidier.
3. **Docs**: `docs/PLATFORM-GAPS.md` item 5 says map downloads never hold the power guard on any platform (now held on macOS). `docs/STATE_MACHINES.md:255` mentions `MapJob.eta_seconds`. `docs/ERRORS.md` needs regenerating.
4. **Frontend lane**: the Dismiss button, the re-read after Delete, the rate display, and the `Maps.tsx:972` copy.

## Doubts
- **Demotion gives up in-app deletion.** If the destination is only temporarily unreachable (unplugged drive, ejected SD card) when Delete or cancel runs, the prefix becomes a reported recovery entry and must be removed by hand at the listed path. `cancel()` is also called by mobile network-policy retirement and shutdown, so this can happen without a tap; when the drive is present the same call deletes the prefix outright. Retrying reclaim of proven recovery records on a later Delete would close this; I did not add it, because it widens what Delete may unlink.
- **The reclaim error text is dropped on demotion.** If cleanup failed after capture and restoration also failed, the file is inside `.streetzim-cleanup-<uuid>/`, not at the reported path. `start()` already behaved this way.
- **`remove_terminal`'s message is inaccurate with only recovery records.** It says "Resume or delete retained map data first." The UI gates on `canRemoveFromList`, so I left the string alone.
- **FEAT-4 delays the error for a wholly unpronounceable article.** Every chunk is now tried before the error. I measured 23.6 ms per 675-byte CJK chunk in the debug test build; release was not measured. Stop and transport remain responsive during the skipping.
- **FEAT-3 cutoff across instances.** One hour is judged on the top-level directory mtime only. Another running copy holding a print dialog open for over an hour could lose its scratch to this copy's sweep.
- **FEAT-8 scope.** The assertion (`UserInitiated | LatencyCritical | IdleSystemSleepDisabled`) is held for the whole run, including local-only composition. That is the existing guard's option set, not tuned for multi-hour jobs.
- **FEAT-2 residue.** A failed `sync_all` followed by the error-path re-sync can still checkpoint data the kernel dropped; the prefix hash catches it at resume, at the cost of the prefix. `map_chunk_cache.rs` writes were not examined.
- **Windows**: clippy only; none of the new tests ran there.