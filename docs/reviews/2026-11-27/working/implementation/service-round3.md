Lane `service`, round 3: all items are done except the integrity flake, which I investigated and could not reproduce. Every gate is green: lane tests twice, host clippy, Windows-gnu clippy, the Android clippy script, tsc, and the new node test. Each round-3 behavioural test failed at its intended assertion with the fix reverted in a scratch copy. Nothing ran on a device, macOS or iOS.

Two deviations from the instructions:
- **store.rs needed three edits, not one.** Once the `from_store` line was deleted, `load_notice()` and its field `load_notice` became dead outside tests, and host clippy failed (`field load_notice is never read`, `method load_notice is never used`). I removed the attribute from `current_load_notice` as asked, added `#[cfg_attr(not(test), allow(dead_code))]` to that method and field (they are now used only by `store_tests`), and reworded the now-stale two-line comment above `current_load_notice` to one line.
- **Two new frontend files** besides App.tsx, for the band rule and its test: `src/startup-notice.ts` and `scripts/test-startup-notice.mjs`.

### SERVICE-R2-1 — maintenance re-acquired provider-backed archives on every pass
Disposition: fixed.
Change: `dispatch.rs`, `library_maintenance` `stale` filter: books with `source_reference.is_some()` are skipped, with a comment explaining why.
Tests: none. On the host, `archive_source::retain` refuses provider references ("require their original mobile platform"), so the old code already skipped them there. A host test cannot tell the two versions apart. Type-checked for Android only.

### SERVICE-R2-2 — the cancellation notice could stay hidden, or stay after it went stale
Disposition: fixed, backend and frontend.
Change:
- `service.rs: import_paths` calls a new `forget_startup_warning(IMPORT_CANCELLED)` after a call that admitted at least one book and had no cancelled paths.
- `src/startup-notice.ts` adds `startupNoticeVisible` and `dismissStartupNotice`.
- `src/App.tsx` now keeps the list of dismissed notices instead of one boolean. The band shows while any current notice has not been dismissed. A new notice brings it back; withdrawing a notice does not reopen ones already dismissed. This is a set of notices rather than the joined text the coordinator suggested, because with the joined text the band would reopen with already-dismissed notices whenever one was withdrawn.
Tests:
- `scripts/test-startup-notice.mjs` (2 tests): `pass 2`. No existing harness covers `startup-notice`.
- `a_privacy_switch_cancels_a_running_import_and_later_imports_still_work` now also checks that the notice is gone after a later successful import. It failed reverted.

### SERVICE-R2-3 — cancelling on background orphaned a full staged copy
Disposition: fixed, as decided.
Change: new `service.rs: discard_cancelled_staged_copy(path)`, `cfg(any(mobile, test))`. It removes a copy only when all of these hold:
- the copy's canonical parent is a canonical-UUID folder directly inside `<data>/archives/mobile-imports-v1`;
- no library entry's path is that file;
- the archive opens and yields its UUID.

It then calls the existing ownership-checked `library_files::remove_duplicate_intake_copy` (which proves the cohort and identity and unlinks only those files), followed by `std::fs::remove_dir(attempt)`, which removes only an empty folder. Anything it cannot prove is left in place.

It is called from two places:
- `admit_intake_batch`: when a verification error is `IMPORT_CANCELLED`, every staged path of that attempt is discarded. Nothing from that attempt was relinked or admitted, because verification comes first.
- `import_paths` (picker copies): each cancelled path is discarded.

What is never removed:
- Pending requests hold source URIs, not staged paths, so a staged copy is never something a pending request needs.
- An admitted neighbour in the same attempt folder keeps both its file and the folder.
- Files outside staging (library or scan folders) and folders with foreign names are not touched.
- A retry calls `mobile_staging()` again and copies afresh.

Tests:
- `a_cancelled_intake_stays_pending_and_does_not_orphan_its_copy`: nothing admitted, receipt unconsumed, copy and attempt folder gone; a fresh copy on retry is admitted.
- `a_cancelled_staged_copy_is_removed_only_where_the_app_owns_it_exclusively`: a real cancelled `import_archives` under a privacy switch removes only the cancelled copy; the admitted neighbour, a library file and a non-UUID folder survive.
- Both failed reverted.
Not exercised: the native copy and retry flow on a device.

### SERVICE-R2-4 — two cutover sources can overlap
Disposition: fixed.
Change: `import_cancel` is now `Mutex<ImportCancel { token, cutovers }>`. `begin` increments the count and cancels the token. `drop` decrements it and installs a fresh token only at zero, and never once stopping has begun.
Tests: `overlapping_cutovers_keep_imports_cancelled_until_the_last_one_ends` (including Quit outlasting a cutover). It failed reverted.

### SERVICE-R2-5 — the settled-file device check copied the profile for every file
Disposition: fixed.
Change: `missing` is computed only on Unix, and only when the recorded device is absent or differs from the current one. It runs in `blocking`, off the async runtime. When the device matches, there is no profile read and no stat.
Tests: the existing watcher tests and the R7 test pass. No separate test.

### Flaky tests
- `spelling::tests::cold_validation_is_single_flight…`: test-only race, fixed.
  - The production code at `spelling.rs:410` has no unwrap. The caller spawns the loader while holding the `inner` lock, so a worker cannot remove its handle before the caller inserts it. That cannot panic.
  - The panic was in the test helper `finish_load`. It now joins the handle if present; if absent, the worker already published its result and removed itself in the same locked step.
- `integrity::tests::real_valid_and_checksum_corrupt_results_survive_restart`: not reproduced, nothing changed.
  - 12 loaded runs of `integrity:: spelling::`, each alongside a concurrent `service::` run: all `ok. 32 passed`.
  - `integrity.rs` is unchanged since base, and the test and `verify` touch no file of mine.
  - Without the reviewer's panic line I cannot point to a cause. A possibility I could not confirm is the Store instance lock still being held when it reopens after `drop(store)`.

### DESKTOP-R11
Disposition: done, with the store.rs deviation described at the top.
Change:
- `from_store` no longer extends `startup_warnings` with `load_notice`. Both cfg branches are back to non-`mut` `let`.
- The `"snapshot"` handler builds `startupWarnings` as `self.startup_warnings` followed by `self.store.current_load_notice()?`.
Tests: `unreadable_book_record_does_not_block_startup_or_snapshot` passes.

### DESKTOP-R8 comments
Disposition: done.
- `service.rs` `from_store` and `service_tests.rs` (~5026) now say the skipped record is preserved and the snapshot's one notice names the copy.
- `dispatch.rs` Windows recovery comment: a same-volume move whose rename is refused because a launch-time open pins the file is now cancelled ("not moved and still in …"), not retried; a destination collision keeps its journal.

### Also changed
`cancel_imports` is now `#[cfg(any(desktop, target_os = "ios", test))]`, the same gate as `shutdown`. After R2-4 it was dead on Android, which the Android clippy run caught.

**Commands run** (all with `SHERPA_ONNX_ARCHIVE_DIR` set)
- `cargo test --lib -- service:: server:: spelling:: search:: archive:: imports:: integrity:: watch:: native_pdf:: store::`: run 1 `ok. 484 passed; 0 failed; 6 ignored`; run 2 identical. No flake in either run; `store::` was added because store.rs was edited.
- Scratch copy with the R2-2 backend, R2-3 and R2-4 fixes reverted (target deleted afterwards): `FAILED. 0 passed; 4 failed`, each at its intended assertion.
- Host clippy `--lib --tests --features custom-protocol -D warnings`: `Finished`.
- Windows-gnu clippy: `Finished`.
- Android clippy script: exit 0, `Finished`.
- `npx --no-install tsc --noEmit`: exit 0.
- `node --test scripts/test-startup-notice.mjs`: `pass 2`.
- `node --test scripts/test-*.mjs`: `pass 517 / fail 2 / skipped 14`. The two failures are the generated-file checks (`docs/ERRORS.md`, and the localization manifest on "kiwix-mcp is missing from the Kiwix folder. Reinstall Kiwix."), neither from my changes; the owner regenerates both.
- `node scripts/check-unsafe-census.mjs`: passes.

**Files changed this round:** `src-tauri/src/service.rs`, `service/dispatch.rs`, `service_tests.rs`, `spelling.rs` (test helper only), `store.rs` (three edits, see top), `src/App.tsx` (startup band only), new `src/startup-notice.ts`, new `scripts/test-startup-notice.mjs`.

**Strings:** no new or changed user-visible strings and no new `t()` keys. The only new text is the log-only "Kiwix import: removing a cancelled staged copy failed: …".

**Doubts**
- The SERVICE-R2-1 skip has no host test that can fail.
- `discard_cancelled_staged_copy` keeps a copy whose archive no longer opens, because its cohort cannot be proved. That leak is accepted to avoid deleting unproven files.
- The `startupWarnings` band now reappears when a new notice arrives after the person dismissed it.