## Verdict for lane files: BLOCKING findings exist

I reviewed the base `d98cac2` against the working tree. HEAD moved from 8a302d9 to 5bca6df and then 6abee86 while I worked, but the lane files did not change. These sha1 values were identical in the tree and in my scratch copy:
- `publication.rs` 294b761…
- `library_files.rs` 29c3d3b…
- `library_files_tests.rs` 5c94ba7…
- `edition_updates.rs` cdf901e…
- `device_scan.rs` 6d175db…

I read the full lane diff and these callers: `instance/socket.rs`, `library_root.rs` (publish_exclusive and the retired-name capture), `maps/partials.rs` capture/reclaim, `localswarm/storage.rs` and `cleanup.rs::erase`, `torrent/publication.rs`, `acquisition/finalize.rs`, `delete_book_impl`, `cleanup_migration_sources`, `Store` load, `edition_service` commit/rollback/discard, `remove_book`, all `edition_transactions` readers, and `sandbox_roots`.

### FILES-R1 — Abandon deletes a verified complete staged copy without proving the source still exists, then says the archive "is still in" a missing folder
Severity: Blocking
Fix under review: LIB-5
Evidence class: reproduced (scratch test `adv_abandon_deletes_verified_copy_when_source_folder_is_gone`)
Location: `src-tauri/src/library_files.rs:1343` (`abandon_unpublished_copy`), `:1273` (`clear_unpublished_copy`), `:1361` (`failed_move`)
Trigger:
1. A cross-volume move stages a part and records `target`.
2. The publish rename fails or the process dies.
3. At the next launch the old (source) drive is unplugged, has failed, or its folder was moved away.
Causal path:
- `recover` → `finish_move` fails at `Directory::open(&journal.source_directory)`.
- `failed_move` → `abandon_unpublished_copy`. `validate_journal` passes because the store still names the source path. `committed` is false.
- `clear_unpublished_copy` checks only the destination side. For the staged part with a matching `target` it runs `staging.dir.remove_file(&part.name)`, then removes the staging folder, then `journals.remove`.
- Output: `"No such file or directory (os error 2). This archive was not moved and is still in /tmp/…"` with that folder absent; `staged_exists=false journal_exists=false`.
- Before this change the retry path never deleted a staged copy that had a recorded `target`. If the source drive is dead, this was the only verified copy.
Smallest correct fix: before clearing anything (in abandon, and in the orphan branch at `:2041`), require that the source directory opens with `source_directory_identity` and every part is present there with a stamp equal to `part.source`. Stamp only, no hash. Otherwise return `Ok(false)` and keep the journal. Add the reproduced case as a regression test.

### FILES-R2 — Orphan-journal drop discards the journal of a split same-volume move after a state loss or skipped record
Severity: Should-fix
Fix under review: LIB-5
Evidence class: reproduced (scratch test `adv_orphan_drop_after_state_loss_discards_split_same_volume_move`)
Location: `library_files.rs:1374` (`journal_orphaned`), `:2041`
Trigger:
1. A same-volume move is interrupted after `commit_path`, with part `.zimaa` in the target folder and `.zimab` still in the source folder.
2. `state.json` is missing at the next launch (desktop starts with `State::default`), or the mobile state is quarantined, or the book record is skipped at load.
Causal path:
- No book has that id, so the journal is removed. Result: source=`["download.zimab"]`, target=`["download.zimaa"]`, journal gone.
- If the skipped or backed-up state is later restored, the book points at a half-moved archive and no journal remains to finish it.
Smallest correct fix: do not treat journals as orphaned when `store.load_warnings()` is non-empty or the state was created at this launch. Never drop a `same_filesystem` journal whose parts are split across the two folders.

### FILES-R3 — New tests make the suite flaky through the single-slot `MOVE_FAILURE_AT`
Severity: Should-fix
Fix under review: LIB-5, LIB-9 tests
Evidence class: reproduced (20 runs of `library_files:: --skip adv_`: `a_cross_volume_move_that_fails_before_publishing_leaves_nothing_to_replay` failed 2 times, the existing `a_failed_staged_rename_restores_every_part_and_leaves_no_copy` failed once with `unwrap_err() on Ok`)
Location: `library_files.rs:45`; `library_files_tests.rs:500`, `:1333`, `:1514`
Causal path: the static holds a single `Option<(ThreadId, u32)>`. Parallel tests overwrite each other's slot, so one test's injected failure never fires.
Smallest correct fix: replace it with `thread_local! { static MOVE_FAILURE_AT: Cell<Option<u32>> }`, or a map keyed by thread. Do the same for `FREE_SPACE_OVERRIDE`.

### FILES-R4 — Fallback leftovers are never healed and the error does not point to them; a lost NFS link reply strands two names
Severity: Should-fix
Fix under review: LIB-1
Evidence class: reproduced (scratch `adv_lost_link_reply_leaves_two_names` gives `Err(AlreadyExists)` with names `["complete","partial"]` and nlink 2; `adv_fallback_leftover_second_name_blocks_delete`); the crash windows are source-traced
Location: `publication.rs:721` (link error path), `:746` (random private name); `library_files.rs:78` (`FileStamp::of`)
Trigger:
- `linkat` succeeded but reported `EEXIST` (lost reply); or
- the process died between link and capture (source name plus destination); or
- it died between capture and unlink (hidden `.kiwix-renamed-<uuid>` plus destination); or
- for directories, it died after `mkdirat` (empty placeholder).
Causal path:
- Every later delete or move of that book fails with "Archive parts must be regular files without hard links". Nothing mentions the hidden file, and no sweep exists.
- A move journal then warns at every launch.
- For directories, `localswarm/cleanup.rs::erase` then reports "Both LocalSwarm cleanup and source names exist" forever.
- `maps/partials.rs` reclaim of an unpublished partial (nlink is not checked when `!published`) unlinks the staging name and leaves the hidden twin, which leaks space.
Smallest correct fix:
- On link `EEXIST`, if `entry_identity(destination)` equals the source's (dev, ino), continue to the capture/unlink step. This finishes both the lost-reply case and a re-run after a crash before capture.
- Name the capture deterministically, `.kiwix-renamed-<dev:x>-<ino:x>`, so it can be recognised.
- Make the nlink>1 message say a hidden `.kiwix-renamed-*` copy may exist in the archive's folder.
- Fix the stale "never a link/unlink fallback" comment in `maps/partials.rs:707`.

### FILES-R5 — Same-device but different-mount journals (bind mounts) still re-hash twice at every launch
Severity: Should-fix
Fix under review: LIB-5 (residual of the original finding)
Evidence class: source-traced
Location: `library_files.rs:1410-1456` (same-filesystem branch, `intact_source`)
Trigger: source and destination share `st_dev` but sit on different mounts, so the first rename returns EXDEV.
Causal path: each launch runs, in order:
1. `commit_path` to the destination (a store write);
2. `check_part_proven` with no proof, which is a full hash;
3. the rename, which fails;
4. the `intact_source` check, another full hash;
5. `commit_path` back.
Abandon returns early when `same_filesystem` is set, so the journal is kept forever.
Smallest correct fix: in that error path, once `intact_source` held and the source path was committed back, and no destination part name exists, remove the journal.

### FILES-R6 — `edition_discard_original` treats an unplugged drive as `missing` and permanently closes the record
Severity: Should-fix (or an explicit owner decision)
Fix under review: LIB-6
Evidence class: source-traced
Location: `edition_updates.rs:586` (`remove_original`)
Trigger: the original sits on a removable or network volume that is not mounted when the person chooses the action.
Causal path: `symlink_metadata` returns NotFound, so the reply is `Missing` and `original_discarded` is set. Rollback is gone and nothing was freed. The file still exists once the drive is reconnected.
Smallest correct fix: return `Missing` only when the parent directory exists. Otherwise return "The drive or folder holding the original is not available. Reconnect it and try again", matching the existing `maps/partials.rs` `Unavailable` wording.

### FILES-R7 — Message nits
Severity: Nit
Fix under review: LIB-1, LIB-9, LIB-10, LIB-2
Evidence class: source-traced
- `archive_basename` (`library_files.rs:237`) says "will not move or delete" but is also reached through `pin_archive_files`/`retain_archive_files` from LocalSwarm and torrent sharing. Use wording that covers sharing.
- The no-hard-links refusal (`publication.rs:~734`) says "move a file into place… Choose a folder on a different drive" even when the person pressed Delete. It also fires on `EPERM`, which `protected_hardlinks` or immutable files can cause.
- The LIB-9 permanent-delete partial path (`:971`) says "had already been deleted" when some parts were put back.
- The LIB-5 cross-volume abandon refuses when a foreign file sits at a part name whose `target` is `None`. That file can never be the move's own publication, because the target is recorded before the rename.
- The device-scan skip line says "hidden or cannot be shown safely" for every skip reason.

**Per-fix assessment**
- LIB-1: fixes it on Linux NFS/FUSE, but with R4.
  - On Android `linkat` is refused, so the library-root switch and rollback still cannot proceed there. They now fail with a clear message instead of EINVAL. This is inference; I could not test whether Android SD storage lacks the flag.
  - macOS/iOS are unchanged: the body is byte-identical to base and only the `cfg` was narrowed.
  - All calls go through the two retained descriptors with no-follow. The `EINVAL/ENOSYS/EOPNOTSUPP` allow-list is correct, because cross-mount gives EXDEV before the filesystem's EINVAL.
- LIB-2 (Rust half): fixes it. Tokens stay fatal, the limitation line echoes only a count, and the two changed tests still prove rejected candidates never become entries.
- LIB-5: fixes it but R1 (Blocking), R2 and R5 remain; R3 affects its tests.
- LIB-6 (backend): fixes it but R6.
  - It is not reachable by users until the frontend button and the `api.ts` `privacyGeneration` injection exist.
  - The predicates cover every reader.
  - Lock order is consistent with rollback: privacy lease, LocalSwarm guards, sharing, readers, `originals`, `OPERATIONS`, store.
  - Delete-first then crash: the next call closes the record as `missing`, and nothing stays reserved.
  - Identity plus `delete_book`'s cohort/UUID proof is sufficient without a checksum.
  - Downgrade claim verified: an older build ignores the field, the record reopens, and its rollback fails on the identity or availability check.
- LIB-9: fixes the prefix and the message; it can never say "left where it is" unless every part is back with its exact stamp. The cross-device home-Trash copy is still a residual.
- LIB-10: fixes it. Reveal stays non-destructive and opens the folder. No mutating path was relaxed.

**Implementer's doubts**
- LIB-1 crash window: a real problem; fix now via R4.
- LIB-1 `EEXIST` strictness: a real problem; the cheap fix is in R4.
- LIB-1 `EINVAL` ambiguity: acceptable. In the directory-into-itself case the placeholder is created and then removed.
- LIB-1 directory crash window: real but rare (R4).
- LIB-1 Android: needs an owner decision. The finding's Android scenario remains unfixed.
- LIB-5 orphan drop: a real problem (R2, reproduced).
- LIB-5 unfinished staged copy removed on the directory's identity: acceptable. The real problem is the finished copy (R1).
- LIB-6 same-UUID sharing guard: acceptable, because it is conservative.
- LIB-6 identity-changed originals: acceptable.
- LIB-6 `missing`: fix or decide (R6).
- LIB-2 limitation strings: acceptable.
- Windows: type-checked only.

**What I ran**
- On the main tree, `cargo test … -- publication:: device_scan:: library_files:: library_root:: edition store:: cross_volume` did not compile because of an E0382 in `update.rs`, which belongs to another lane.
- I copied the tree to scratch, made a one-line `update.rs` fix there only, and built it with the shared target:
  - The same filter gave 213 passed and 2 failed; both failures are my repro tests, R1 and R2.
  - `cross_volume` ran rather than skipping.
- I reverted each fix in scratch through environment toggles and re-ran the same binary to see which new tests fail:
  - **LIB-1 reverted:** `unsupported_exclusive_rename_falls_back…`, `fallback_preserves…`, `fallback_never_unlinks…`, `fallback_keeps_the_source…`, `fallback_moves_a_directory…` and `fallback_refuses…` fail. `fallback_is_taken_only…` does not fail.
  - **LIB-5 reverted:** `an_unpublished_copy_is_abandoned…`, `abandoning_removes_only…`, `recovery_drops_the_journal…`, `a_cross_volume_move_that_fails…` and `a_cross_volume_move_is_refused_up_front…` fail. `a_journal_is_never_abandoned…`, `a_failed_recovery_names…` and `a_copy_that_cannot_fit…` do not.
  - **LIB-9 message reverted:** `a_failed_trash_names_a_recovery_folder_only_while_one_exists` fails.
  - **LIB-10 reverted:** `a_platform_valid_name_can_be_revealed…` fails.
  - **LIB-6 store predicates reverted:** `discarding_a_retained_original_deletes_it_and_frees_its_name` fails.
- Host clippy and Windows-gnu clippy (`--lib --tests -D warnings`) failed only in other lanes' code: `store.rs:3074` (`state_bytes`), `service.rs:211` and `:2342`, and `localswarm/mod.rs:24`. No diagnostic in lane files, and the test-config passes reached lints.
- The Android type-check (run twice) failed only on the unused import at `localswarm/mod.rs:24` in another lane, so the lane's `cfg(mobile)` code type-checks.

**Could not verify**
- Real NFS/FUSE behaviour.
  - Inference: high-level FUSE such as sshfs without `use_ino` reports different inode numbers per path, so the fallback would end in "both names were kept" with an extra hard link.
  - Inference: NFS silly-renames an unlinked open file, so a permanent delete of an open book could report a recovery folder.
- macOS, iOS and Android runtime.
- The Windows runtime.
- The real Trash.

All scratch artifacts are in `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-files/`: the test logs, the toggled scratch tree under `tree/`, the repro tests at the end of `tree/src-tauri/src/library_files_tests.rs` and in `tree/src-tauri/src/publication.rs`, and the binary `scratch-test-bin`.