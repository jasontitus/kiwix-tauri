# Lane `files` — round 3 (FILES-R8, FILES-R9)

Both nits are fixed. The requested tests pass and host clippy is clean; Windows-gnu clippy is also clean. The Android clippy run fails, but only on lints in other lanes' files (`maps/manager.rs:756`, `store.rs:2456`, `crash.rs:530` and `:820`); it reported nothing in my files.

### FILES-R8 — hidden-name hint points at the wrong folder after a move
Disposition: fixed.
Verification of the finding: Correct. The capture rename happens in the source directory of the interrupted rename, which for a library move is the folder the archive came from.
Change: `library_files.rs`, `FileStamp::of` hint now reads "…a hidden second name for the file (.kiwix-renamed-…) may be left in the archive's folder, or in the folder it was last moved from; delete that hidden entry and try again".
Tests: `a_hard_linked_archive_refusal_points_at_a_leftover_hidden_name` still passes; it asserts the `.kiwix-renamed-` mention, not the new clause.
Not exercised: nothing beyond the wording.

### FILES-R9 — `missing` at the root of an unmounted mount point
Disposition: fixed.
Verification of the finding: Correct. The previous check accepted any existing parent directory, including the empty directory a mount point leaves behind.
Change: `edition_updates.rs`
- New `folder_held_original(parent, recorded_device)`. It is true only if the parent opens as a directory and either its device equals the device recorded for the original's first part, or it has at least one entry.
- `remove_original` returns `Missing` only when that holds; otherwise it returns the "not available. Reconnect it and try again" error and the record stays open.
- The recorded device is read by serializing `transaction.original_identity` and taking `[0].device`. `FileIdentity`'s fields are private in `integrity.rs`, which I do not own.
- The device comparison uses cap-std's `dir_metadata().dev()` on all platforms, the same source `ContentIdentity` uses on Windows.

Tests:
- `discarding_a_retained_original_deletes_it_and_frees_its_name` gained the requested case. **What was modelled:** an original alone in its own folder; the transaction's stored `original_identity` is rewritten to device + 1, then the file is removed, so the folder is empty and not on the recorded device. Discard fails with "is not available. Reconnect it" and the path stays reserved. With the true identity restored (the same empty folder, on the recorded device) discard returns `missing`. A real unmounted mount point cannot be created in this container.
- New `only_the_folder_that_held_the_original_makes_it_missing` covers the helper directly: empty with matching device, empty with mismatched or unknown device, non-empty with mismatched device, and no folder.
- Fail-without-fix: inferred. The old check returned `Missing` for any existing parent directory, so the new "not available" assertion would fail.

Not exercised: a real unmounted mount point; Windows and Android are type-checked only.

## Commands run
- `cargo test --lib library_files::` → `ok. 52 passed; 0 failed` (first attempt hit other lanes' compile errors in `localswarm/cleanup.rs` and `kokoro.rs`; passed on retry)
- `cargo test --lib publication::` → `ok. 24 passed; 0 failed`
- `cargo test --lib edition` → `ok. 25 passed; 0 failed`
- Host clippy (`--lib --tests --features custom-protocol -- -D warnings`) → `Finished`, no errors
- Windows-gnu clippy → `Finished`, no errors
- `android-clippy.sh` → failed after retries, on the four other-lane lints listed above only

## Files changed in this round
- `src-tauri/src/library_files.rs` (one string)
- `src-tauri/src/edition_updates.rs`

Changed user-visible string: the hard-link hint above. No new strings; the "not available" error is reused.

## Handoffs
- **`integrity.rs` owner (optional):** a `ContentIdentity::first_device()` accessor would replace the serialize-and-read used to get the recorded device.

## Doubts
- On filesystems whose device number changes between mounts, an original that was alone in its folder and was deleted by hand will get "not available" instead of `missing` if the number has changed. The record then stays open (the conservative side) until something else is in that folder.
- Host and Windows clippy finished in about a second each, so they reused cached results for these sources rather than re-linting from scratch in my run.