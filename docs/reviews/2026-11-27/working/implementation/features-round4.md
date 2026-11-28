# Lane `features` — round 4 report (changes since the round-3 report only)

R11, R12, the `partials.rs` twin-name handoff and the FEAT-9 event state are all done. Lane tests pass and host and Windows-gnu clippy are clean on the final tree. Nothing was run on macOS, iOS, Android or Windows, and the real link-then-unlink fallback cannot be reached on this container's filesystem.

One thing to know first: the FEAT-9 item needed a two-line edit in `src-tauri/src/service.rs`, which is outside my standing file list. `finish_voice_job` lives there, not in `kokoro.rs`. I made it because your message assigned that function to me; it is the only edit outside my lane.

Diff: `git diff d98cac2 -- src-tauri/src/maps.rs src-tauri/src/maps src-tauri/src/maps_tests.rs src-tauri/src/kokoro.rs src-tauri/src/service.rs`

---

### FEATURES-R11 — ownership refusals classified as "busy"
Disposition: fixed, with a typed marker rather than `Ok(Preserved)`.

Verification of the finding: Confirmed. Every `Err` from the reclaim with the journaled folder intact became `Busy` and retryable, so Delete failed on every press for states no retry changes.

Why a marker and not `Preserved`: I re-checked the callers. `start()` and `OwnedPartialGuard` treat anything but `Reclaimed` as "keep". `new()` and `reclaim_published_map` show different text for `Preserved` than for `Err`, so returning `Preserved` would have changed their messages. A new variant keeps both byte-for-byte.

Change:
- `maps/partials.rs`: new `PartialRecovery::Refused(String)`, carrying the detail for the log. Returned from six branches of `reclaim_owned_file`:
  - unexpected name;
  - invalid publication filename;
  - publication unresolved;
  - invalid cleanup-capability name;
  - capability disappeared;
  - capability replaced.
- Also `Refused` (my addition): the post-capture "changed identity or size" check. An entry that is not ours fails it identically on every attempt.
- `new()` maps `Refused` to its existing `Err` text; `reclaim_published_map` returns the carried text as its `Err`, as before.
- `maps/manager.rs`: `reclaim_job_partial` and `retry_recovery_records` classify `Refused` like `Preserved`. With the folder intact that is `Unconfirmed`, report-only, so a later Delete returns `Ok`.
- `KeptAt` is now a small struct. When the kept path is the map's final name, the Delete error is "Kiwix did not remove {path}. {reason}". The words "a partial map download" are used only for a partial.

Tests:
- `a_foreign_file_at_the_final_name_is_reported_once_and_never_called_a_partial_download`
- `a_removed_cleanup_folder_is_reported_instead_of_failing_delete_forever`
- `cleanup_captures_replacements_and_preserves_exclusive_restoration_collisions` now expects `Ok(Refused)` instead of `Err`; its preservation assertions are unchanged.
- The first two were observed failing in a private copy with `Refused` classified as a failure. The "not called a partial download" wording was not separately mutated.

### FEATURES-R12 — two sentences
Disposition: fixed, with the reviewer's wording minus the final full stop (house convention for Rust error strings).
- `Unavailable`: "Its folder cannot be opened right now. If it is on a drive that is not connected, reconnect it and press Delete again. If the folder is gone, dismiss this warning"
- `Busy`: "It could not be removed just now. Press Delete again; if it keeps failing, free some space or close programs using that folder"
- When the message comes from `cancel()`: "…reconnect it and delete this download…" and "…Delete this download to try again; …".

Tests: the existing assertions were updated to the new text.

### Handoff from the files lane — captured entry with a second name
Disposition: fixed as you specified.

Verification of the finding: Read the current `publication.rs` (`exclusive_rename_fallback`). On Linux and Android, when the filesystem has no `RENAME_NOREPLACE`, the capture links the new name, renames the old one to `.kiwix-renamed-<dev:x>-<ino:x>` in the source directory, then unlinks it. An interruption leaves the staged entry with nlink 2. Before this change, the unpublished path did not check nlink: it unlinked the staging name and reported `Reclaimed` while the bytes stayed under the other name.

Change (`reclaim_owned_file`, after the capture block), when the captured entry has nlink 2:
1. If the public directory holds `record.name` as the same file, the same capture is called again under the same library guard; the primitive finishes that move.
2. Otherwise, if `.kiwix-renamed-{dev:x}-{ino:x}` in the public directory is the same file, it is removed and the directory synced.
3. After a re-stat, `nlink() == 1` is required before any unlink, published or not. Anything else is `Refused`, so the record is kept.
- I also added `nlink() != 1` to the pre-capture check, which returns `Preserved`. Without it a partial with a foreign hard link would be captured and put back on every Delete.

Tests:
- `an_interrupted_link_then_unlink_capture_never_leaks_the_partial_under_its_other_name` — real hard link at the `.kiwix-renamed-…` name. Delete leaves neither name, and a bystander `.kiwix-renamed-0-0` file is untouched.
- `a_capture_interrupted_before_the_old_name_was_retired_is_finished_by_the_same_move` — twin at the public name. The primitive's "destination already is this file" behaviour is modelled by an injected capture.
- `a_partial_with_a_link_kiwix_did_not_make_is_never_unlinked_as_if_it_freed_the_bytes` — both before and after capture.
- All three were observed failing in the copy with the twin handling and both link-count checks removed.

Not exercised: the real fallback; step 1 against the real primitive.

### FEAT-9 — completion event state
Disposition: fixed.

Verification of the finding:
- `useVoiceModelStatus.ts` is the only listener for `kiwix-speech-model`, and it only refreshes on any state other than "downloading".
- After that refresh `status().error` is null (the round-1 `finish_download` fix), so `SpeechModels.tsx` and `KokoroDownloadPrompt.tsx` show no alert and the button reads "Install natural voices".
- Nothing in `src/`, the plugins or scripts matches on `"error"` for this event.

Change:
- `kokoro.rs`: `DownloadControl::cancelled()`.
- `service.rs: finish_voice_job`: `"state"` is `"installed"` on success, `"cancelled"` when `control.cancelled()`, otherwise `"error"`. `"error"` in the payload still carries the cancel text; no listener reads it.

Tests: `stopping_a_voice_download_is_not_shown_as_an_error` now also asserts `cancelled()` is true after Cancel and false for a download that failed on its own. `finish_voice_job` needs a `Service`, so the kokoro tests cannot reach it; the emitted state is untested.

---

## Commands run (final result lines)
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- maps:: kokoro native_pdf` (final tree) → `test result: ok. 140 passed; 0 failed; 3 ignored`
- Host clippy with `-D warnings` → exit 0 on attempt 8. One early failure was mine (`KeptAt` more private than `reclaim_job_partial`; fixed by making it `pub(super)`). The rest were in `store.rs` and `crash.rs` (other lanes).
- Windows-gnu clippy with `-D warnings` → exit 0.
- Fail-without-fix, in a private copy sharing the target directory → `FAILED. 69 passed; 5 failed`. The copy and its artifacts were removed. The artifact hashes matched last round's, so that attribution was right.

## Files changed since round 3
`src-tauri/src/maps/partials.rs`, `src-tauri/src/maps/manager.rs`, `src-tauri/src/maps_tests.rs`, `src-tauri/src/kokoro.rs`, and two lines in `src-tauri/src/service.rs`.

Strings for `docs/ERRORS.md` regeneration:
- New: "Kiwix did not remove {path}. {reason}", plus the two reworded sentences in both endings.
- Removed: the round-3 `Unavailable` and `Busy` sentences.

## Frontend contract for `map_delete_download`
The shape is unchanged; three details moved.
- **Report-only is wider.** `Ok` with the job still listed now also covers journal-versus-folder disagreements: a foreign file at the map's final name, a removed or replaced cleanup folder, a damaged record, or an entry with a link Kiwix did not make. These no longer return `Err` on later presses.
- **Second `Err` lead-in.** "Kiwix did not remove <path>. <reason>" is used when `<path>` is the map's final name. It appears only on the call that had to keep this download's own record.
- **Reworded reasons.** The `Unavailable` and `Busy` sentences are as quoted above. The first now mentions dismissing the warning, so it assumes the Dismiss button exists.

## Handoffs
- **Files lane.** `partials.rs` now duplicates the `.kiwix-renamed-{dev:x}-{ino:x}` naming from `publication.rs`. If that name changes, step 2 silently stops matching and the entry is refused, not leaked. A shared helper for the name would remove the coupling.
- **Not asked for, same family.** An interruption of the *publication* move on such a filesystem leaves the partial name and the final `.zim` as one file with two names. Recovery and reclaim both require nlink 1, so that map ends up reported as "could not be confirmed" instead of being finished. No bytes are lost and it is not new, but nothing heals it.

## Doubts
- **Step 1 on a filesystem with an exclusive rename.** If the public name and the captured entry are the same file there (only possible by hand), the repeated capture fails with "File exists" and Delete returns the retryable `Err` each time. I kept the error and did not fall through to a refusal.
- **Pre-capture link check.** A partial that has any second link is now reported as unconfirmed instead of having Kiwix's name removed. That matches what Resume already requires, but it is a behaviour change for someone who hard-linked a hidden partial.
- **`Refused` after capture when the name was taken meanwhile.** The entry stays in the cleanup folder and is report-only; the reported path points into that folder (round-2 probe). Previously this was an `Err`.