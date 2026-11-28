# Lane `files` — review follow-up (FILES-R1 … R7)

All seven review items are fixed. One part of R2 is a superset proxy rather than the exact check, because `Store` exposes no "created at this launch" flag (handoff below). Lane tests pass, `library_files::` passed 20 of 20 consecutive runs, and host, Windows-gnu and Android clippy are clean.

I did not revert fixes to watch tests fail. For R1, R2 and R4 the reviewer's own repros failed before the fix and the ported versions now pass. The other new tests are inferred to fail without their fix, with the reason given.

### FILES-R1 (blocking) — abandon deleted a verified staged copy without proving the source exists
Disposition: fixed.
Verification of the finding: Confirmed in the code; `clear_unpublished_copy` checked only the destination side.
Change (`library_files.rs`):
- New `source_intact(journal)`: the source directory must open with `source_directory_identity`, and every part must be present with a stamp equal to `part.source` (stamp only, no hash).
- It is the first check inside `clear_unpublished_copy`, so both the abandon path and the orphan path go through it. If it fails, nothing is cleared and the journal stays.
- The "This archive was not moved and is still in …" text is now produced only after that proof held.

Guarantee: no staged file, staging folder or journal is removed unless the full source cohort is present as recorded.
Tests: `a_staged_copy_and_its_journal_survive_while_the_source_is_not_provably_present` (port of the repro). It covers folder gone, one part missing, one part replaced by another inode, and folder recreated at the same path; then, with the source back, the abandon proceeds. The reviewer's repro failed before the fix.

### FILES-R2 — orphan drop after state loss / split same-volume move
Disposition: fixed, with a proxy for "fresh state".
Verification of the finding: Confirmed. `Store` exposes `load_warnings()` but nothing that says the state was created at this launch.
Change (`journal_orphaned`): a journal is an orphan only if all of these hold:
- the journal is owned by this profile;
- `store.load_warnings()` is empty;
- the library still lists at least one non-session book, and none has the journal's book id;
- `source_intact(journal)` holds.

The "at least one book" rule stands in for the fresh-state check: a fresh state has no books, so it can never orphan anything. `source_intact` means a split same-volume move is never dropped.
Cost of the proxy: a person who removed their only book mid-move keeps the (now named) warning at each launch, as before this lane.
Tests: `a_lost_profile_or_a_split_move_never_makes_a_journal_an_orphan` (port of the repro, plus the split, whole and empty-library cases). `recovery_drops_the_journal_of_a_book_that_left_the_library_and_names_it_once` was updated so another book remains in the library.

### FILES-R3 — flaky single-slot test hooks
Disposition: fixed.
Change: `MOVE_FAILURE_AT` and `FREE_SPACE_OVERRIDE` are now `thread_local!` `Cell`s; four test sites updated.
Tests: the built test binary ran `library_files::` 20 times: `20 runs: pass=20 fail=0`, each `ok. 52 passed; 0 failed`.

### FILES-R4 — fallback leftovers never healed
Disposition: fixed for files; the directory placeholder case is reported, not changed.
Change (`publication.rs`, `exclusive_rename_fallback`):
- **Link `EEXIST` with the same inode:** if the destination's (dev, ino) equals the source's, the move continues to capture and unlink. This finishes a lost NFS reply and a re-run after a crash following the link. Any other `EEXIST` is returned as before.
- **Deterministic capture name:** `.kiwix-renamed-<dev:x>-<ino:x>` of the published file. If that name already exists as the same inode (an earlier interrupted move), it is unlinked first. If it is another file, the move stops with "… is in use by another file; the old name was kept".
- **Defect of my own found while doing this:** healing a same-entry move (`a → a`) with the old logic would have removed the file's only name. The destination is now read again after the capture, and the captured name is unlinked only if the destination holds that same inode at that moment. The private name is also never allowed to equal the source or destination name (it gets a `-1` suffix).
- **nlink>1 refusal** (`FileStamp::of`): on Linux/Android it now adds that a hidden `.kiwix-renamed-…` second name may be left in the archive's folder, and to delete it and try again.
- **`maps/partials.rs`:** the stale comment is fixed (comment only).

Tests (`publication::` → `ok. 24 passed`):
- `fallback_finishes_a_move_whose_link_already_exists` (port of the lost-reply repro, plus the pre-linked case);
- `fallback_capture_name_is_recognizable_and_a_leftover_is_cleared`;
- `fallback_never_removes_the_only_name_of_a_file`;
- in `library_files`, `a_hard_linked_archive_refusal_points_at_a_leftover_hidden_name` (port).

What the two other callers still need (I did not edit their code):
- **`maps/partials.rs` reclaim.**
  - When the staging entry already exists it skips `capture`. After an interruption the staged file has nlink 2, its twin being either the public name or `.kiwix-renamed-<dev>-<ino>` in the public directory.
  - A published partial is then refused forever ("Captured map partial changed identity or size").
  - An unpublished partial is unlinked from staging and the twin leaks its bytes.
  - Needed: when the staged entry has nlink 2, call `capture` again if the public name is the same inode (the primitive now finishes that move), or unlink the `.kiwix-renamed-{dev:x}-{ino:x}` twin if it is the same inode; and require nlink == 1 in the unpublished case too.
- **`localswarm/cleanup.rs::erase`.**
  - A crash after the directory fallback's `mkdirat` leaves an empty directory at the cleanup name while the source directory still exists. Both "Both LocalSwarm cleanup and source names exist" sites (lines ~280 and ~392) then fail forever.
  - Needed: when both exist and the cleanup-name entry is an empty directory that is not the journaled identity, remove it (empty-only `remove_dir`) and capture again.
  - I did not make the primitive adopt an existing empty directory: it cannot know the directory is its own, and adopting it would break "an existing destination is `AlreadyExists`".

Not exercised: real NFS/FUSE; the lost reply is modelled by injection.

### FILES-R5 — same-device, different-mount journals re-hash at every launch
Disposition: fixed.
Change (`finish_move_with_rename`, same-filesystem error path): once `intact_source` held, the source path was committed back, and no destination part name exists, the journal is removed. The error gains "This archive was not moved and is still in …".
Behaviour change to note: a failed same-volume first rename is now cancelled instead of retried at the next launch. A destination collision still keeps the journal. The comment in `dispatch.rs` near the Windows gate ("a pinned book only leaves a journaled warning") is now slightly stale; that file is outside my allowance for this.
Tests: the existing `failed_first_rename_restores_source_path_on_every_recovery_attempt` was **changed on purpose**. It asserted the journal survives; it now asserts the journal is gone after the failed rename, then re-writes it to exercise the unchanged collision half. Fail-without-fix: inferred (the old code kept the journal).

### FILES-R6 — unplugged drive treated as `missing`
Disposition: fixed.
Change (`edition_updates::remove_original`): `Missing` is returned only when the file is absent and its parent directory exists. Otherwise the call fails with "The drive or folder holding the original is not available. Reconnect it and try again" and the transaction stays open.
Tests: `discarding_a_retained_original_deletes_it_and_frees_its_name` gained that case (folder moved away → error, not discarded, still reserved; folder back → proceeds). `edition` → `ok. 24 passed`. Fail-without-fix: inferred.

### FILES-R7 — nits
Disposition: fixed, all five.
- **`archive_basename`:** "This archive's file name {reason}. Kiwix can read it, but will not move, delete or share a file with such a name. Rename the file and use Locate …; to remove it instead, remove the archive from the library and delete the file yourself".
- **No-hard-links refusal:** "Kiwix cannot rename a file on this storage without the risk of overwriting another one: it has no exclusive rename, and a hard link could not be created ({error}). Nothing was changed. Keep archives that Kiwix should move or delete on a different drive". This wording is also true for EPERM from `protected_hardlinks` or immutable files.
- **Partial permanent delete:** new `settled_delete_failure`. It says "N of the archive's M files had already been deleted; the others were put back" when only some are gone. Test: `a_settled_delete_failure_says_what_is_gone_and_what_came_back`. This is a unit test of the wording only; there is no seam to make the second unlink fail.
- **Foreign file at an unpublished part name:** the destination check now blocks only for parts with a recorded `target`. Test: `a_foreign_file_at_an_unpublished_part_name_does_not_block_abandoning` (the foreign file is untouched). `a_journal_is_never_abandoned_once_anything_of_the_move_exists` now records the target for its published part.
- **Device-scan line:** "N ZIM file(s) were found but could not be listed, so they were left out."

## Commands run (final tree)
- `cargo test --lib publication::` → `ok. 24 passed; 0 failed`
- `cargo test --lib library_files::` → `ok. 52 passed; 0 failed`; then 20 runs of the same binary → `pass=20 fail=0`
- `cargo test --lib edition` → `ok. 24 passed; 0 failed`
- `cargo test --lib device_scan::` → `ok. 10 passed; 0 failed`
- `cargo test --lib library_root::` → `ok. 35 passed; 0 failed`
- Host clippy (`--lib --tests --features custom-protocol -- -D warnings`) → `Finished`, no errors
- Windows-gnu clippy (brief's command) → `Finished`, no errors
- `android-clippy.sh` → `Finished`, no errors. This also type-checks my earlier `cfg(mobile)` edits, so their "not compiled here" flags from the first report no longer apply for Android; iOS and macOS remain uncompiled.
- `node scripts/check-unsafe-census.mjs` → `75 matches / 75 blocks / 2 impls in 20 files; every item documented`

## Files changed in this round
- `src-tauri/src/publication.rs`
- `src-tauri/src/library_files.rs`
- `src-tauri/src/library_files_tests.rs`
- `src-tauri/src/edition_updates.rs`
- `src-tauri/src/device_scan.rs`
- `src-tauri/src/maps/partials.rs` (comment only, as allowed)

New or changed user-visible strings (Rust): the five R7 wordings above, the nlink hint, "… is in use by another file; the old name was kept", and "The drive or folder holding the original is not available. Reconnect it and try again". `docs/ERRORS.md` needs regenerating.

## Handoffs
- **`store.rs` owner:** a `Store::created_at_launch()` accessor (true when `state.json` was absent and a default state was written) would let `journal_orphaned` use the exact check instead of "library lists at least one book".
- **Maps lane and LocalSwarm lane:** the two needs listed under R4.
- **Frontend:** `edition_discard_original` can now fail with the "not available. Reconnect it" message; show it as an ordinary error. The rest of the contract is unchanged.

## Doubts
- **R2 proxy:** a profile restored from an older backup that lacks the book but lists others would still drop a source-intact journal. Nothing is deleted except the move's own unfinished staging, but a same-volume move whose store commit preceded a crash before the first rename would leave that book pointing at the destination until Locate.
- **R4 healing treats a user-made hard-link pair as a half-done move:** if source and destination are already the same inode, the source name is removed and the call succeeds, where native `RENAME_NOREPLACE` would return `EEXIST`. No data is lost and the destination keeps its inode.
- **R4 crash between capture and unlink:** this still needs the person to delete the hidden name unless the same file is moved again from that folder. The name is now recognisable and the refusal points at it.
- **R5:** it removes the automatic next-launch retry for same-volume failures that might have been transient (for example a file briefly open on Windows during startup recovery). The library stays consistent and the message says the archive was not moved.