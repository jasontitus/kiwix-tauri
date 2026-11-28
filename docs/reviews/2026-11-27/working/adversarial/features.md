## Verdict for lane features: BLOCKING findings exist

One blocking finding (FEAT-1 demotion is irreversible and fires on retryable failures). FEAT-3 and FEAT-6 are correct as far as they go but are not closed until their handoffs land. FEAT-2, 4, 7, 8, 9 hold up. Lane tests pass here.

### FEATURES-R1 — Demotion makes every retryable reclaim failure permanent, and Delete reports success
Severity: Blocking (judgement: no bytes are lost, but a retryable failure becomes a non-retryable outcome reported as `Ok`; if the owner consciously accepts stranding, this drops to Should-fix)
Fix under review: FEAT-1 (with FEAT-6's `Ok`)
Evidence class: source-traced for the consequence; the unreachable-folder variant is reproduced by the lane's own "folder missing" case, which I ran; the Windows and full-disk triggers are inference (no Windows runtime; tmpfs here does not charge blocks for `mkdir`)
Location: `src-tauri/src/maps/manager.rs:740-753` (`reclaim_job_partial`, arm `Ok(PartialRecovery::Preserved) | Err(_)`), reached from `delete_download:878` and `cancel:700`
Trigger: Any of these while a map has a checkpoint, then Delete or any `cancel()`:
- Destination volume absent (unplugged drive, ejected SD card): `MapDirectory::open` fails.
- Windows: an indexer or AV tool holds the just-closed `.part`, so `capture_partial` (opens with DELETE access, share READ|DELETE only) gets a sharing violation. This is plausible on an ordinary Cancel of an active download: the worker's `OwnedPartialGuard` reclaim fails, then `cancel()` retries microseconds later.
- Destination volume at zero free blocks (the FEAT-2 scenario) when it is not the journal's volume: `create_dir_with(".streetzim-cleanup-…")` fails.
- Any `sync_map_directory`, unlink or mid-reclaim journal-save error.

Causal path:
- `reclaim_owned_file` returns `Err` for all of the above (`partials.rs:320, 420-434, 482-512`). The new arm discards the error text, moves the record to `recovery_records`, clears `resume`, persists, and returns `Ok(())`.
- Nothing ever reclaims a recovery record. Every use of `recovery_records` is carry, report or forget (`manager.rs:425-476, 650-652, 745, 899, 929, 948-952`).
- At base the same call returned `Err` with the record intact, so a second Delete/Cancel moments later, or after replugging, removed the file.
- Before this change a stranded record was at most an empty file, because the checkpoint is written at creation (`manager.rs:1395-1397`) and the old `start()` branch for resumable records was unreachable. Now it can be most of a multi-GB map, in a hidden dotfile, on the disk the user was trying to free.
- `delete_download` then returns `Ok`. The current `Maps.tsx:727-738` and `Downloads.tsx:789-803` close the dialog as success.
- FEAT-1 asked for `Err` on the call that demotes; FEAT-6 asked for `Ok` only "once everything it owns is gone". This call satisfies neither.

Callers of `cancel()` (shutdown is not one; it uses `pause()` at `service.rs:3597`, so the implementer's "and shutdown" is wrong in the safe direction):
- `dispatch.rs:2641` `map_cancel` (user).
- `dispatch.rs:1396` switch to private or read-only mode.
- `mobile_policy.rs:472` mobile network-policy retirement, fully automatic.
- `mobile_service.rs:2301` `mobile_pause_transfers` (Android notification Pause, transfer timeout or error).

On lost resumability: `cancel()` already deleted a reachable prefix at base ("maps cancel using their owned-partial journal and require explicit restart"), so nothing new is lost there. What is new is the strand. At base the automatic paths returned `Err`, kept the record, and the pending retirement retried once the volume was back. Do not simply restore `Err` for automatic callers, though: the map error keeps mobile retirement pending, which appears to refuse every Internet transfer ("The previous network connection is still being closed…"); I did not trace that gate end to end.

Smallest correct fix: keep demotion, because it unblocks, but make it reversible and honest.
1. In `delete_download` (explicit user action only), before the `recovery_records.is_empty()` test, re-run the same `reclaim_partial` under `with_acquisition_destinations` on each recovery record with `identity: Some`, and drop those that return `Reclaimed`. The proof is identical (journaled directory identity, inode identity, exclusive capture), so nothing unproven is unlinked. A returned volume or released handle then lets Delete finish, and warnings for files already gone clear themselves. The implementer declined this as "widening what Delete may unlink"; I disagree, but it is an owner decision.
2. Return `Err` from the `delete_download` call that performed a demotion, naming the kept path; later calls return `Ok`.
3. Keep the reclaim error instead of discarding it (see R5).

Narrower alternative if (1) is rejected: demote only on `Ok(Preserved)`, a directory-identity mismatch, an invalid name, or destination `NotFound`; return every other `Err` unchanged. This leaves the old wedge for persistent I/O errors.

Tests to add: Delete with the folder renamed away, rename it back, Delete again must leave no file and no warning. A failing-capture case via `reclaim_partial_with_capture` must not strand the file.

### FEATURES-R2 — Crashed-export scratch is still never removed unless the user exports another PDF at least an hour later
Severity: Should-fix (the lane's code is correct; FEAT-3 stays open until the handoff lands)
Fix under review: FEAT-3
Evidence class: source-traced
Location: `src-tauri/src/native_pdf.rs:334` is the only caller of `cleanup::sweep_stale`; `service.rs` has none
Trigger: Private-mode export, process killed, relaunch, and the user never exports a PDF again (or exports within the hour).
Causal path: The sweep runs only inside `render`, and only removes directories whose mtime is at least `STALE_AFTER` (1 hour) old. The finding's own statement, "a private session's article text can persist in the cache indefinitely", is still true for that user.
Smallest correct fix: Land handoff 1: `pub(crate) use cleanup::sweep_stale;` plus a `spawn_blocking` call at service start on `<app cache>/article-pdf` (Android: native `cachePath`).

### FEATURES-R3 — FEAT-6 is not user-reachable, and Delete on a kept-files job is now a silent no-op loop
Severity: Should-fix (land the UI half in the same release)
Fix under review: FEAT-6 (backend half)
Evidence class: source-traced against the current tree (`Maps.tsx` and `Downloads.tsx` are unchanged from base)
Location: `manager.rs:899-908`; `src/components/Maps.tsx:727-738, 289-333`; `src/components/Downloads.tsx:789-803`
Trigger: Delete download on a job with recovery records.
Causal path:
- `map_delete_download` resolves, the dialog closes, and `setJob(null)` plus `noteJobUpdate()` re-run the poll immediately. `map_status` returns the same job, now `cancelled` with the "could not be cleaned up… kept here: …" alert.
- The Delete button is offered again and repeats the loop.
- Nothing in `src/` calls `map_forget_recovery` or reads `recoveryPaths`, so the warning cannot be dismissed and follows every later job.
- At base the same press produced an explicit error in the dialog.

Is it safe to ship as is: yes for data and for flow. Get map is enabled, since `cancelled` is terminal and the veto is gone. The "cannot be resumed. Get the map again" copy is now accurate. No worse than base for pre-existing records, but it is a dead end.
Smallest correct fix: Ship with the frontend handoff (Dismiss button calling `map_forget_recovery`, re-read after Delete), plus R1 item 2.

### FEATURES-R4 — A failed sync still leads to a checkpoint of bytes the kernel may have dropped (pre-existing, adjacent to the fix)
Severity: Should-fix
Fix under review: FEAT-2
Evidence class: source-traced; the kernel behaviour is inference
Location: `manager.rs:1098` (`checkpoint_output`, "Cannot sync map prefix"), then `manager.rs:1512-1522`
Trigger: `sync_all` fails at a periodic checkpoint (writeback EIO, or ENOSPC on NFS or thin storage).
Causal path: The `String` error converts through `From<String>` to `retain: true, checkpoint_current: true`. The error path calls `sync_all` again; Linux reports a writeback error once per descriptor, so the second call can succeed. `checkpoint_resume(written: chunks.written)` then journals bytes that may not be on disk. Resume re-hashes from the page cache, can pass, and can publish a file with a hole.
Smallest correct fix: Make sync failures `CompositionFailure { retain: true, checkpoint_current: false }` so the last durable checkpoint is kept.

### FEATURES-R5 — The reported recovery path can name a location where no file exists
Severity: Nit
Fix under review: FEAT-1 / FEAT-6
Evidence class: reproduced for the first case (the lane's tests assert `recovery_paths == [partial]` after the folder was renamed away); source-traced for the second
Location: `maps.rs:393-399` (`recovery_path`); `manager.rs:740` discards the error
Trigger and path:
- "Folder missing" or "folder recreated": the warning says "The files were kept here: X" and X does not exist.
- Reclaim failed after capture and restoration failed: the file is in `.streetzim-cleanup-<uuid>/`. The discarded error said so ("Retained entries are recoverable at…"); the warning names the old path.

Smallest correct fix: Carry the reclaim error into `job.error`. When the destination opens and none of the record's names exist, drop the record instead of reporting it (R1 item 1 does this).

### FEATURES-R6 — The FEAT-8 test cannot fail if the fix is removed
Severity: Nit
Fix under review: FEAT-8
Evidence class: source-traced
Location: `maps_tests.rs` `the_map_worker_can_hold_the_power_assertion_across_awaits`
Causal path: It asserts `Guard: Send` on the platform-neutral type. Deleting the gated line in `run` leaves it green on every platform.
Smallest correct fix: The finding's proposal: a `#[cfg(test)]` holder count on `PROCESS`, asserted while a composition is parked on `before_worker`. Otherwise drop the test and record FEAT-8 as untested.

### FEATURES-R7 — Stale documentation claims
Severity: Nit (integration owner)
Fix under review: FEAT-7, FEAT-8
Evidence class: reproduced by grep
Location: `docs/STATE_MACHINES.md:255` still says `MapJob` tracks `eta_seconds`. `docs/PLATFORM-GAPS.md` item 5 needs the macOS power guard noted.
Note: `node --test scripts/test-error-docs.mjs` fails 1 of 4, but with about 3,600 rows of drift from another lane's generator change, not from this lane.

**Per-fix assessment**
- FEAT-1: fixes the wedge on all platforms (the dead branch really was dead, and no bytes are unlinked without proof), but see R1.
- FEAT-2: fixes it.
  - Checked against tokio 1.54.1 `fs/file.rs`: `poll_write` returns `Ok(n)` straight after spawning; `poll_flush` returns the blocking `write_all` result (`:1206-1235`); the blocking side uses std `write_all`, so a short write cannot report success.
  - All three composition writes go through `write_observed` before hash or counter advance; `checkpoint_output` and the final sync flush first.
  - Cost is bounded: slices are at most 16 MiB, written as 2 MiB sub-writes each already awaited by the next, so the flush only waits for the last one and adds no fsync.
  - Residue in R4.
- FEAT-3: does not fully fix it (R2). The sweep itself is sound; I could not make it delete anything it should not.
  - A new directory has a fresh mtime before `LIVE` insertion.
  - `Owner::drop` removes the directory, or records it in `PENDING`, before leaving `LIVE`.
  - A symlink child, a symlink root and symlinks inside the tree are not followed.
  - A backward clock keeps everything.
  - It runs in a detached `spawn_blocking`, and every native lease is a `Files` clone.
- FEAT-4: fixes it. I read `run` end to end: forward skip terminates at `chunks.len()`; all-unpronounceable gives the original error; only chunk 0 pronounceable ends normally; a stale prefetch is dropped and joined; Stop is checked every iteration.
  - The backward skip is correct and should stay. Without it, Previous/Rewind into an unpronounceable chunk would bounce forward to where the user was, a stuck transport the forward-only fix would introduce. It turns forward once at index 0 and cannot loop.
  - Engine "produced no audio" still aborts the article, a deliberate deviation from the finding's proposal; defensible.
- FEAT-6 (backend): correct, but not user-visible (R3). `recoveryPaths` is never journaled.
- FEAT-7 (backend): fixes it by removal. No reader of a map `etaSeconds` remains in Rust, TypeScript, Kotlin or Swift; the remaining hits are `DownloadJob`, torrent, LocalSwarm and the always-`None` `RetainedMapJob`. `tsc --noEmit` is clean.
- FEAT-8: looks right by reading. It is identical in form to `localswarm/mod.rs:1112`: path, `acquire(&'static str)`, `Send`, underscore binding, module declared unconditionally. Not compiled on macOS.
- FEAT-9: fixes it.
  - A genuine failure that coincides with a cancel is suppressed, which is right because the user cancelled.
  - `stop()` refuses once publishing, so later errors still show.
  - `kokoro_download` returns immediately, so neither component gets a thrown cancel error; `SpeechModels` ends on "Install natural voices" with no alert.

**Journal compatibility**: claim holds. `MapJob` has no `deny_unknown_fields`, so a leftover `etaSeconds` is ignored and a previous build ignores anything new. The nested `deny_unknown_fields` structs are unchanged. `recovery_paths` is filled only on the `status()` clone, which is never persisted. The compat test is built with the current serializer plus two injected keys, not a real old journal.

**Implementer's doubts**
- Demotion gives up in-app deletion: real problem, fix now (R1).
- Reclaim error text dropped: real, small (R5).
- `remove_terminal` message inaccurate: acceptable; the UI gates on `canRemoveFromList`.
- FEAT-4 delays the error for a wholly unpronounceable article: acceptable; the frontend's 45-second start timer bounds it.
- FEAT-3 cutoff across instances: narrow but real, since two processes on different profiles share the OS cache directory and a print dialog held open over an hour in one could lose its scratch to the other. Owner decision; raising `STALE_AFTER` to 24 hours costs the privacy goal little.
- FEAT-8 option set (`LatencyCritical` for hours): owner decision.
- FEAT-2 residue: real (R4).
- Windows not run: stands, and matters most for R1.

**What I ran**
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- maps:: kokoro native_pdf`: the first attempt failed to compile only in `edition_updates.rs` and `sharing_interop_tests.rs` (other lanes); the retry gave `test result: ok. 128 passed; 0 failed; 3 ignored; 0 measured; 1272 filtered out`. All ten new lane tests are in that run.
- `npx --no-install tsc --noEmit`: exit 0.
- `node --test scripts/test-error-docs.mjs`: pass 3, fail 1 (see R7).
- A scratch tmpfs mount to probe `mkdir` on a full volume; it succeeded there, so that trigger stays inference.

**What I could not verify**
- Anything on macOS, iOS, Android or Windows.
- The `run` loop with a real model and audio device.
- A real full volume or a failing fsync.
- How often R1's Windows sharing-violation trigger occurs.
- The implementer's "observed failing with the fix neutralized" claims; I did not mutate the tree.