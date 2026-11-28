# Lane `features` — round 3 report (changes since the round-2 report only)

R8, R9 and R10 are done in my owned files, except the polled `status()` probe, which I left in place (reason below). Lane tests pass and host and Windows-gnu clippy are clean on the final tree. Nothing was run on macOS, iOS, Android or Windows.

Diff: `git diff d98cac2 -- src-tauri/src/maps.rs src-tauri/src/maps src-tauri/src/maps_tests.rs`

---

### FEATURES-R8 — `record_vanished` dropped a proven record on path-only evidence
Disposition: fixed (strict rule).

Verification of the finding: Confirmed. `record_vanished` never compared `directory.identity` with `record.directory_identity`, so a recreated or shadowing empty folder made Delete forget the record and return `Ok` with `map_status` null.

Change:
- `maps/partials.rs: record_vanished` returns false unless the opened directory is the journaled one. A record is now dropped without a reclaim only when the journaled directory itself holds none of its partial, publication and cleanup-capability names.
- `maps/manager.rs: retry_recovery_records`: a directory-identity mismatch is report-only, like `Preserved`. The record is kept and is not counted as failed, so a later Delete returns `Ok` with the warning still on the job.
- The call that demotes this download's own partial still returns `Err` once, whatever the reason.

Tests:
- `deleting_a_map_whose_folder_was_replaced_says_so_once_and_keeps_reporting_it` (renamed and rewritten): `Err` once, then `Ok` twice with the record and path still reported; once the journaled folder is back, Delete removes the file.
- `a_recreated_mount_point_never_makes_delete_forget_the_partial_on_the_absent_volume`: folder gone, unattended `cancel()` keeps the partial, the path is recreated empty with a different identity, Delete returns `Ok` and the record stays; after "remount", Delete removes it.
- `a_record_is_gone_only_when_its_journaled_folder_holds_none_of_its_names`: unit test of `record_vanished`.
- Observed failing in a private copy with the identity check removed. Also observed with the mismatch counted as retryable: the two folder tests failed with `Err` on the second Delete.

### FEATURES-R9 — raw OS text and Debug identities on the job card
Disposition: fixed.

Change:
- New `partials.rs: Kept`, classified by the reclaim's own first checks in order: destination opens, then is the journaled directory, then declined or failed.
- The raw reclaim text goes only to stderr: `Kiwix map cleanup kept a partial download (<kind>): <raw>`.
- Sentences that reach `job.error` and the `Err`:
  - Destination not available: "Its folder is not available right now. Reconnect the drive that holds it, then press Delete again"
  - Folder replaced or moved: "Its folder was moved or replaced, so Kiwix left the file alone"
  - Not confirmed: "It could not be confirmed as the file this download wrote, so Kiwix left it alone"
  - Any other failed removal: "It could not be removed just now; another program may be using it. Close anything that has it open, then press Delete again"
- When the message comes from `cancel()` rather than a pressed Delete, the ending is "…then delete this download".
- Each reason is attached to its own path: `…The files were kept here: <path A> (<reason A>), <path B>`. `append_recovery_warning_because` now takes one optional reason per record.
- "Delete the map download, then start it again" no longer reaches any Delete or cancel message.
- `NOT_OUR_FILE` is removed. A cosmetic `KeptAt` alias names the `(path, reason)` pair.

Tests: the unreachable-folder, failed-capture, dismissal and mount-point tests assert the exact sentences, the per-path placement, and the absence of "os error", "FileIdentity" and the injected raw text. Observed failing in the copy with raw-style sentences and per-record placement disabled (6 failed in that pass, together with the R8 mutation).

### FEATURES-R10 — small items
Disposition: fixed, except the `status()` probe, which is left as is.

- Widening test added: `delete_removes_a_proven_partial_carried_from_an_earlier_download_and_nothing_beside_it`. A carried record with its real inode is removed by another job's Delete. A never-journaled partial-named file and a user file beside it survive byte-for-byte, and `cancel()` before the Delete does not touch the carried partial. Passes; not separately mutated this round. The retry it relies on was observed failing-when-disabled in round 2 through other tests.
- Wording: now "could not be confirmed as the file this download wrote", used for both a foreign inode and a never-proven reservation.
- `status()` probe: left. It runs only for a recovery record that has a capture, which is rare. Avoiding it means caching the resolved location across the six places that mutate `recovery_records` (`start`, demotion, retry, `forget_recovery`, `new`, the warning rebuild). A stale cache would report a wrong path, which is what R5 fixed. I judged that not cheap. On a hung mount, `map_status` can block in that state.

---

## Commands run (final result lines)
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- maps:: kokoro native_pdf` (final tree) → `test result: ok. 135 passed; 0 failed; 3 ignored`
- Host clippy with `-D warnings` → exit 0 on attempt 6. Earlier attempts failed only in `store.rs` and `service.rs:211` (other lanes).
- Windows-gnu clippy with `-D warnings` → exit 0.
- Fail-without-fix, in a private copy as instructed: pass 1 → `FAILED. 63 passed; 6 failed`; pass 2 → `FAILED. 67 passed; 2 failed`.
  - The copy was `<scratchpad>/features/copy`: `src-tauri/src` copied, everything else symlinked, path dependencies made absolute, same `CARGO_TARGET_DIR`. The shared tree was not mutated.
  - Afterwards I deleted the copy and the artifacts it produced in the shared target, about 2 GB.
  - `debug/build/kiwix-workbench-b133e859b5beec34` was attributed to the copy by creation time only. If it was the real build script's directory, cargo rebuilds it.

## Files changed since round 2
`src-tauri/src/maps.rs`, `src-tauri/src/maps/manager.rs`, `src-tauri/src/maps/partials.rs`, `src-tauri/src/maps_tests.rs`.

Strings for `docs/ERRORS.md` regeneration:
- New: the four sentences above, each in its two endings where it has one.
- Removed: "The file there is not the one this download wrote, so Kiwix left it alone".
- Unchanged: "Kiwix could not remove a partial map download, so it was kept at {path}. {reason}".

## Updated frontend contract for `map_delete_download`
- **`Err(message)`** — one of:
  - the old refusals (active job, read-only, library reservation, published file changed, journal not saved);
  - "Kiwix could not remove a partial map download, so it was kept at <path>. <sentence>", returned either
    - once, by the call that had to keep this download's own partial, for any reason, or
    - again, by a later call, only while a proven kept file still cannot be removed for a reason the user can act on (folder not available, or removal failed and the file may be in use).
  - In the "kept" case the job stays listed as `cancelled` with `recoveryPaths`; Get map is not vetoed.
- **`Ok` with `map_status` null**: everything is gone. This is no longer reached on path-only evidence.
- **`Ok` with the job still listed**: only report-only entries remain. That is a file that could not be confirmed as this download's, or (new) a kept file whose path now leads to a different folder. Only Dismiss (`map_forget_recovery`) clears these, or Delete once the original folder is back at that path.
- `job.error` format: `Some earlier map downloads could not be cleaned up. The files were kept here: <path> (<reason>), <path>, …`. A reason appears only for entries the latest Delete or cancel examined. It is not parseable; use `recoveryPaths` for the paths.
- `recoveryPaths` and `map_forget_recovery` are unchanged.

## Doubts
- **"Another program may be using it" is a hedge.** It covers every reclaim failure with the journaled folder intact: a real sharing violation, but also a full disk when creating the cleanup directory, a journal-save failure, or a replaced cleanup capability. Reclaim errors are strings, so I could not separate "in use" without matching OS text.
- **"Folder is not available… Reconnect the drive" covers any failure to open the destination**, including permission denied or the path now being a symlink.
- **The strict rule leaves a stale warning** after a phone restore or migration, where the partial is really gone and the directory identity never matches again, until Dismiss exists. This is the trade-off you chose.
- **stderr now carries the raw reclaim text**, which can include the path of Kiwix's own partial or cleanup directory inside the download folder, plus device and inode numbers. The brief says not to put raw paths of user documents in diagnostics. These are not user documents, but they do reveal the download folder path.