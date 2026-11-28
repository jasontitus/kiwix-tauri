# Lane `files` — report

All six findings are addressed: LIB-1, 2, 5, 6, 10 fixed; LIB-9 partial (prefix and message fixed, cross-device preflight left as a residual). Targeted tests, Linux clippy and Windows-gnu clippy are clean on the final tree. Nothing was run on real NFS/FUSE, macOS, iOS or Android.

**No test was observed failing by reverting its fix.** I did not put the shared tree into a deliberately broken state while checkpoints were being committed. Every "fails without the fix" below is inferred, with the reason given.

---

### LIB-1 — Exclusive rename has no fallback on NFS/FUSE
Disposition: fixed differently — capture-then-unlink instead of the reviewer's compare-then-unlink; no `O_EXCL` placeholder for files.

Verification of the finding: Read `publication.rs` and every caller. `rename_noreplace_at` returned the raw error; only the path-based `platform_publish` fell back, and that fallback unlinked the source unconditionally. `library_root::publish_exclusive` had its own `renameat_with` copy. Kernel behaviour on NFS/FUSE is taken from the finding, not run.

Change (`src-tauri/src/publication.rs`):
- `rename_noreplace_at` (Linux/Android) now calls `rename_noreplace_with(…, rename, link, after_link)`, which has injectable primitives, then `exclusive_rename_fallback`.
- The fallback is taken only for `ENOSYS | EINVAL | EOPNOTSUPP`. `EEXIST` and every other error are returned untouched.
- **Files, symlinks, sockets:** `linkat` (exclusive; `EEXIST` passes through as `AlreadyExists`). The old name is then renamed to a private `.kiwix-renamed-<uuid>` in the same directory, and only that captured entry is inspected.
  - It is unlinked only if its (dev, ino) equals what the destination name holds.
  - If it differs, it is put back exclusively (link + unlink, or a plain rename for a directory); if the name is taken again it stays under the private name and the error names it.
  - Why not compare-then-unlink: that leaves a window between the `stat` and the `unlink` of the public source name, which is exactly what the comment at `publication.rs:352` forbids.
- **Directories:** `mkdirat` placeholder (exclusive), then a plain `rename`, which can only displace an empty directory. `ENOTEMPTY` is mapped to `EEXIST`.
- **No hard links either** (`EPERM/ENOSYS/EOPNOTSUPP`, plus `EACCES` on Android): refused with `ErrorKind::Unsupported` and a message naming the limitation. I could not show that an `O_EXCL` placeholder plus plain rename cannot overwrite a file that takes the placeholder's name.
- `platform_publish` (path-based) tries the native call by path as before, then opens both parents and uses the same fallback. Its unconditional `remove_file(source)` is gone.
- `library_root::publish_exclusive` now calls `publication::rename_noreplace_at`.
- The macOS/iOS variant keeps its original body; only its `cfg` was narrowed. **Not compiled here.**
- Comments updated at `publication.rs:352` and `library_files::move_exclusive`.

Overwrite/unlink guarantee: nothing is created except by `linkat`/`mkdirat` (both fail on an existing name). The only `unlinkat` targets are the private captured name, and only when the same inode is proven reachable under another name. All calls go through the two retained descriptors with `AT_SYMLINK_NOFOLLOW`.

Tests (`publication::tests`, Linux/Android-gated) — `test result: ok. 21 passed`:
- `unsupported_exclusive_rename_falls_back_to_an_exclusive_move` (EINVAL/ENOSYS/EOPNOTSUPP; same-dir and cross-dir; inode kept, nlink back to 1)
- `fallback_preserves_an_existing_destination_as_already_exists` (file, dangling symlink, directory)
- `fallback_is_taken_only_when_the_filesystem_lacks_the_flag` (EEXIST/EACCES/EXDEV/ENOENT/EIO/EROFS/ENOSPC never reach link)
- `fallback_never_unlinks_a_source_name_that_was_reused_after_the_link` (renamed aside, renamed over, replaced by a directory, removed)
- `fallback_keeps_the_source_when_the_destination_changes_after_the_link`
- `fallback_moves_a_directory_only_onto_its_own_exclusive_placeholder`
- `fallback_refuses_a_filesystem_without_hard_links_instead_of_risking_an_overwrite`

Fail-without-fix: inferred. The tests call functions that did not exist, and with no fallback the injected `EINVAL` is returned to `.unwrap()`.

Not exercised: **real NFS/FUSE behaviour is unexercised** (this container has only ext4/tmpfs/squashfs). Android and macOS are not compiled. The path-based `platform_publish` fallback wiring has no injected test; the shared fallback it calls is tested.

---

### LIB-2 — Android device scan aborts on empty location or dot-name (Rust half)
Disposition: fixed (Rust half); Kotlin half is a handoff.

Verification of the finding: `clean()` rejected `""`, and every `?` in the candidate loop aborted `scan()`. `DeviceZimScan.kt` trims `RELATIVE_PATH` and `walk()` (line ~150 vs 225) lacks the dot-name skip on the tree path.

Change (`src-tauri/src/device_scan.rs`):
- `clean_optional` trims, bounds and rejects control characters but allows empty; used for `location`.
- Per-candidate validation moved into `admit()`. A failure skips that candidate and one summary limitation line is appended (a count only; no rejected text is echoed).
- Invalid or duplicate **tokens remain fatal**: they are minted by native code, not derived from device content, and the existing message is actionable.

Tests — `test result: ok. 10 passed`:
- new `an_archive_at_a_volume_root_has_an_empty_location`, `one_unusable_candidate_does_not_hide_the_others`
- **two existing tests changed on purpose**: `rejects_unsafe_names_and_tokens` and `rejects_invalid_split_names_even_when_unreadable` asserted `scan(..).is_err()`; they now assert the candidate is absent plus one limitation (the token cases still assert `is_err`).

Fail-without-fix: inferred. Those two tests' old assertions document the whole-scan failure.

Not exercised: no Android device; the Kotlin producer was only read.

---

### LIB-5 — "Move previous downloads": no space check, endless retry, anonymous warning
Disposition: fixed, with two residuals (below).

Verification of the finding: Read `move_archives`, `finish_move_with_rename`, `recover` and the dispatch warning assembly. Confirmed: no `volume_space` call, `journals.remove` only on success, and the warning carried only the error string.

Change (`src-tauri/src/library_files.rs`, `service/dispatch.rs`):
- **Up-front check** in `move_archives`: sums part sizes (identity-only `verified_parts`, no hashing) of books on a different device than the destination and refuses the whole move with "Not enough free space in … Nothing was moved". Unknown capacity is not treated as zero.
- **Per-journal check** in the cross-volume branch of `finish_move` before any hashing, so recovery fails fast too. A partial staged copy that will be deleted counts as free.
- **Abandon** (`abandon_unpublished_copy` / `clear_unpublished_copy`), cross-volume only. The journal is removed only if all of these hold:
  - the journal validates and the library still points at the source path;
  - the destination directory opens with the recorded identity and no destination part name exists;
  - the staging directory is cleared and removed.
- **Staging ownership:** the directory must be the private 0700 child with the journal-recorded identity. A finished staged copy is removed only if its device, inode and size equal the recorded `target`. An unfinished copy has no recorded identity of its own; it is removed on the same authority the existing retry path uses (it sits in the journal-bound private directory). The directory is removed only when empty; an unexpected entry keeps both it and the journal.
- **Orphan journals:** `recover` drops an owned journal whose book id is no longer in the library. It warns once; archive files are never touched; staged copies are cleared only under the same "nothing published" rule.
- **Warning:** `recover` error entries now carry `title`, `from`, `to` (directories). Dispatch formats "Moving “{title}” from {from} to {to} did not finish: {message}".
- The journal schema is unchanged (it is `deny_unknown_fields`).

Overwrite/unlink guarantee: source files and published destination parts are never touched by the new code. Deletions are confined to the identity-checked private staging directory and the journal file.

Tests (`library_files::tests`) — `test result: ok. 47 passed`:
- `a_copy_that_cannot_fit_is_refused_and_unknown_capacity_is_not_zero`
- `an_unpublished_copy_is_abandoned_with_its_owned_staging_and_journal`
- `a_journal_is_never_abandoned_once_anything_of_the_move_exists` (published part; store committed; same-volume)
- `abandoning_removes_only_staging_entries_that_are_still_the_recorded_ones`
- `recovery_drops_the_journal_of_a_book_that_left_the_library_and_names_it_once`
- `a_failed_recovery_names_the_archive_and_both_folders`
- Linux-only, real ext4→`/dev/shm` moves through `move_archives`: `a_cross_volume_move_that_fails_before_publishing_leaves_nothing_to_replay` and `a_cross_volume_move_is_refused_up_front_when_the_archives_cannot_fit`. Both ran here, not skipped: `cross_volume … 2 passed`.

Fail-without-fix: inferred. The old code kept the journal and the partial file, and the old warning had no title.

Not exercised: the dispatch warning string has no service-level test (`service_tests.rs` is not mine). Windows branches are type-checked only. A genuinely full disk was not used; a thread-bound test override supplies the free-space figure.

Residuals:
- A same-volume journal that can never finish (e.g. "Migration directory identity changed"), or a cross-volume one whose destination cannot be inspected or already holds a part, is still retried and warned about at every launch. It is now cheap and names the archive and folders.
- The modal still has no progress or cancel (frontend, not in this lane).

---

### LIB-6 — Superseded original can never be deleted
Disposition: fixed (backend); frontend button is a handoff.

Verification of the finding: Nothing ever removes or closes a transaction. I found these readers of `edition_transactions`, and two intent readers that matter:
- `service.rs` `register_books`
- `store.rs` acquisition reservation (`with_acquisition_destinations_available_for_http`)
- `store.rs` `retains_provider_reference`
- `store.rs` `sandbox_roots`
- `edition_service.rs` status/commit/rollback; `validate_state`; `reconcile_edition_relocation`
- **Beyond the reviewer's three:** the committed *intent's* `target` also reserves the original's path and provider grant, so the predicates had to learn about intents as well.

Change:
- `edition_updates.rs`:
  - `EditionTransaction.original_discarded` (`#[serde(default)]`, serialized as `originalDiscarded`).
  - `State::original_discarded(book)`, `discardable_original`, `discard_edition_original`.
  - `EditionManager::discard_original` and `remove_original`: re-retain, compare `ContentIdentity` with `original_identity`, `unchanged()`, drop the handles, then `library_files::delete_book(original, trash)`.
  - **No content checksum, unlike rollback** — a damaged original must stay deletable; `delete_book` still proves the exact part cohort and UUID.
  - `prepare_rollback` and `rollback_edition` refuse a discarded transaction: "The original of this edition was deleted, so it can no longer be rolled back".
  - A new `originals` mutex serializes rollback's commit with discard, so discard can never delete an original that a concurrent rollback just reinstalled.
- `edition_service.rs`: new arm with rollback's guards (`normal_writable`, window, generation, `localswarm_source_mutation`, sharing and torrent-share refusal, `readers` lock held across delete and record).
- Command name added at all five places `edition_rollback` is listed in `src-tauri/src`: `dispatch.rs` ×2, `service.rs` ×3. One of those (`_mobile_transfer_guard`) is `cfg(mobile)`, **not compiled here**. No `mobile_policy.rs` occurrence exists.
- Predicates: `register_books` ignores a discarded transaction like a rolled-back one. The acquisition reservation and `retains_provider_reference` skip a discarded original and any intent targeting it; `replacement` and `rollback_book` stay reserved.
- `sandbox_roots` left unchanged on purpose: it treats rolled-back transactions no differently, and retaining a folder grant cannot endanger data.
- Order: delete first, mark afterwards. An interruption leaves an open transaction with a missing file, which the next call closes as `missing`.
- No `acquisition*` file needed editing.

Refusals: transaction rolled back; already discarded; any library row with the original's installation id, path or provider reference; identity changed since the update (the message tells the person to delete the file themselves and choose again).

**Frontend contract**
- Command: `edition_discard_original`
- Arguments: `{ transactionId: string, privacyGeneration: number }` — no `expectedInstallationId`.
- Reply: `{ transaction: EditionTransaction, originalFile: "deleted" | "trashed" | "missing" | "external" }`
  - `missing`: nothing was at the path; nothing deleted; record closed.
  - `external`: provider-referenced original Kiwix cannot delete; only the hold (and on mobile the provider grant) is released.
- Status: `edition_status.transactions[]` now always carries `originalDiscarded: boolean`.
  - Discardable ⇔ `!originalDiscarded && rolledBackInstallationId === null`.
  - Rollback must be hidden when `originalDiscarded`.
  - `original.sourceReference !== null` means the action will only stop keeping it, not delete it.
- `src/types.ts`: added `originalDiscarded` and `EditionOriginalDiscarded`.

Tests — `edition_updates:: … 17 passed`; `edition … 24 passed`; `store:: … 72 passed`; `delayed_session_ipc … 1 passed`:
- `a_discarded_original_closes_its_transaction_and_refuses_rollback` (includes loading a record without the field)
- `an_original_the_library_lists_again_is_never_discarded`
- `discarding_a_retained_original_deletes_it_and_frees_its_name` — through `Service::dispatch` on real files: reserved and not importable before; failed Trash changes nothing; changed original refused; then deleted, rollback and second discard refused, publishable and importable again, other transaction still protected, `missing` path, durable after restart.
- `a_discarded_provider_original_gives_up_its_capability`

Fail-without-fix: inferred (the command and field did not exist).

Not exercised: the `cfg(mobile)` grant release (`forget_registered`, mirrored from `dispatch.rs` remove_book) is **not compiled here**. The Windows branch is type-checked only. The "trashed" outcome is unexercised because tests refuse the real Trash.

Downgrade impact: `EditionTransaction` is not `deny_unknown_fields`, so an older build loads a profile written by this one. It ignores `originalDiscarded`: the transaction looks open again, rollback is offered and fails with "Original archive is unavailable", and the path is reserved again. No data is at risk. A profile written by the old build loads unchanged.

---

### LIB-9 — Linux "Move to Trash"
Disposition: partial — prefix and misleading sentence fixed; cross-device preflight not implemented.

Verification of the finding: Confirmed the prefix line, the unconditional "recoverable at" sentence, and in `trash-5.3.0/src/freedesktop.rs` the PermissionDenied → home trash → `CrossesDevices` → `copy_dir_all` path. No sign in history or docs that Linux was excluded deliberately.

Change (`library_files.rs`):
- `deletion_staging_prefix(trash)`: `kiwix-deleted` whenever trashing, on every desktop.
- Failure message now depends on what is actually left:
  - all parts back with their stamps → "…The archive was left where it is";
  - staging still exists → the old "recoverable at …" sentence;
  - staging gone and parts gone → "…had already been moved to the Trash / deleted".

Residual (cross-device copy): the crate picks the trash from its private mount-table lookup (`/proc/mounts`, longest prefix), then `.Trash/$uid` validity, then `create_dir(.Trash-$uid)`, and falls back only on PermissionDenied. Predicting that needs a mirror of private logic in a `trash = "5"` dependency. Any divergence (bind mounts, btrfs subvolumes, escaped mount names) would refuse Trash where it works today. The device-boundary shortcut fails differently on the same cases. I also cannot exercise it: the container runs as root with no unwritable mount root, and tests refuse the real Trash. A follow-up would mirror the mount-table logic exactly and test it on a real second volume.

Tests: `the_trashed_folder_is_visible_and_a_permanent_delete_stays_hidden`, `a_failed_trash_names_a_recovery_folder_only_while_one_exists` — in the 47 passed. Fail-without-fix: inferred (the old message always contained "recoverable at"; the old Linux prefix was dotted).

Not exercised: real Trash on any OS.

---

### LIB-10 — Platform-valid "unsafe" names
Disposition: fixed.

Verification of the finding: `basename → filename` gave "Invalid single-component archive filename" for delete, list, retain, move and reveal, while reading uses `read_filename`.

Change (`library_files.rs`):
- `archive_basename` keeps the same rule but names the cause (longer than 240 bytes; colon, backslash or control character; trailing dot or space) and the way out. Used by `archive_files`, `verified_archive_files`, `retain_archive_files`, `delete_book_impl`, `move_archives`, `remove_duplicate_intake_copy`.
- **No mutating path was relaxed.**
- `reveal_book` now uses `retain_read_archive_files` plus a UUID check (`reveal_target`). It therefore also accepts hard-linked and legacy single-file archives, as reading does.
- `basename` is now `#[cfg(mobile)]` (its only remaining callers are mobile); body unchanged, **not compiled here**.

Tests: `a_platform_valid_name_can_be_revealed_while_mutation_explains_its_refusal` (`a:b.zim`, 249-byte name, trailing space, wrong archive) — in the 47 passed. Fail-without-fix: inferred.

Not exercised: actually opening a file manager (the test checks the target folder only).

---

## Commands run (final tree)
- `cargo test --lib publication::` → `ok. 21 passed; 0 failed`
- `cargo test --lib device_scan::` → `ok. 10 passed; 0 failed`
- `cargo test --lib library_files::` → `ok. 47 passed; 0 failed`
- `cargo test --lib library_root::` → `ok. 35 passed; 0 failed`
- `cargo test --lib edition` → `ok. 24 passed; 0 failed`
- `cargo test --lib store::` → `ok. 72 passed; 0 failed`
- `cargo test --lib delayed_session_ipc` → `ok. 1 passed`
- `cargo test --lib cross_volume -- --nocapture` → `ok. 2 passed` (no skip line printed)
- On an earlier snapshot of the tree: `migration` → 12 passed; `provider_reference` → 12 passed
- Linux clippy (`--lib --tests --features custom-protocol -- -D warnings`) → `Finished`, no errors
- Windows-gnu clippy (brief's command) → `Finished`, no errors
- `npx --no-install tsc --noEmit` → exit 0
- `node scripts/check-unsafe-census.mjs` → `75 matches / 75 blocks / 2 impls in 20 files; every item documented` (no `unsafe` added)

Other lanes' in-progress edits broke compilation several times (`sharing*`, `maps*`, `shared_library.rs`, `instance.rs`); I waited and retried and touched none of them.

## Files changed
Owned:
- `src-tauri/src/publication.rs`
- `src-tauri/src/library_files.rs`
- `src-tauri/src/library_files_tests.rs`
- `src-tauri/src/library_root.rs`
- `src-tauri/src/device_scan.rs`
- `src-tauri/src/edition_updates.rs`
- `src-tauri/src/edition_service.rs`

Shared, within the allowance:
- `src-tauri/src/service/dispatch.rs` — recovery-warning assembly; the command in two lists
- `src-tauri/src/service.rs` — three list additions; the `register_books` predicate
- `src-tauri/src/store.rs` — two reservation predicates
- `src/types.ts` — `originalDiscarded`, `EditionOriginalDiscarded`

New user-visible strings (all Rust; no `t(...)` strings):
- Exclusive rename: the "supports neither an exclusive rename nor hard links" refusal and five fallback anomaly messages.
- Device scan: "N ZIM file(s) … left out because … hidden or cannot be shown safely."
- Move: "Not enough free space in …", "…Nothing was moved", "…was not moved and is still in …", "The unfinished move is kept and will be tried again at the next start: …", the orphan message, three "preserved at" staging messages, and the dispatch "Moving “…” from … to … did not finish: …".
- Delete: "The archive was left where it is", "…had already been moved to the Trash/deleted".
- Names: the three `archive_basename` messages; "The file at this archive's location is a different archive…".
- Editions: the discard refusals and "The original of this edition was deleted, so it can no longer be rolled back".

`docs/ERRORS.md` needs regenerating.

## Handoffs
- **Android lane:** `DeviceZimScan.kt` `walk()` should skip `name.startsWith('.')` like the media and storage walks. Until then each scan of such a folder shows one "left out" limitation line.
- **Frontend lane:**
  - Add the "Delete retained original" action with a confirmation naming the file, per the contract above.
  - Hide rollback when `originalDiscarded`.
  - Add `"edition_discard_original"` to the `privacyGeneration` auto-inject list in `src/api.ts` (~line 1196, beside `edition_rollback`), or pass `privacyGeneration` explicitly.
  - Device-scan limitations already render.
- **Maps lane (comment only):** `maps/partials.rs` says capture uses "never a link/unlink … fallback". `capture_partial`, `localswarm::rename_directory_noreplace` and `instance/socket.rs` now inherit the fallback through `rename_noreplace_at`.
- **Changed contract:** `recover` error entries now have `title` = archive title (it was the journal file name) plus `from`/`to`; `id` is still the journal file name.

## Doubts
- **LIB-1 crash window.** A crash between link and unlink leaves two names for one inode. After the capture step the second name is a hidden `.kiwix-renamed-<uuid>` in the source directory. Until someone removes it, destructive operations refuse the file ("regular files without hard links"). Nothing is lost, but nothing self-heals. Compare-then-unlink would leave the leftover at the known source name but keep the TOCTOU on a public name; I chose against that.
- **LIB-1 `EEXIST` strictness.** If `linkat` reports `EEXIST` after a lost NFS reply although it succeeded, the call returns `AlreadyExists` with both names present. I did not add "same inode ⇒ finish".
- **LIB-1 `EINVAL` is ambiguous.** It also covers moving a directory into itself. The fallback is exclusive in every case, so taking it is harmless.
- **LIB-1 directory crash window.** A crash after `mkdir` leaves an empty directory at the destination.
- **LIB-1 Android.** `linkat` is normally denied to apps, so on a volume without `RENAME_NOREPLACE` the outcome there is the clear refusal, not a working move. The claim that Android SD storage lacks the flag is the reviewer's inference; I could not test it.
- **LIB-5 orphan drop keys on "no book with that id".** If a profile's state were reset while `library-migrations-v1` survived, all its journals would be dropped with one warning each. Files are untouched.
- **LIB-5 unfinished staged copy.** Removing it rests on the directory's recorded identity, not a per-file identity. That is the existing retry's authority, but it is weaker than "recorded identity matches" read literally.
- **LIB-6 same-UUID editions.** The sharing guard keys on `original.metadata.id`, so it can ask the person to stop sharing the current edition first. This is conservative, not unsafe.
- **LIB-6 identity-changed originals** can only be closed after the person deletes the file by hand.
- **LIB-6 `missing`.** Closing on a missing file also closes it when the drive is merely unplugged; the file can be re-imported later, but rollback is gone.
- **LIB-2 limitation strings.** A native limitation with a control character or over 512 bytes still fails the whole scan (unchanged; not part of the finding).
- **Windows.** All Windows-specific paths in my changes are type-checked only.