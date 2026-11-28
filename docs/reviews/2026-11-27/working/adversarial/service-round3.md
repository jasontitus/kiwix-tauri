## Verdict for lane service (round 3): only non-blocking findings

One Should-fix remains, SERVICE-R3-1 (an import-cancelled notice that comes back after being dismissed stays hidden), so the lane is not closed yet. Everything else in round 3 is closed. Once R3-1 is fixed, the lane can close with the residuals listed at the end.

I reviewed `git diff 0b669c7` for `service.rs`, `dispatch.rs`, `service_tests.rs`, `spelling.rs`, `store.rs`, `App.tsx`, `src/startup-notice.ts` and `scripts/test-startup-notice.mjs`. The crash, quit, maps and lib edits in the same range belong to other lanes and I did not review them.

### SERVICE-R3-1 — A cancellation notice that comes back after being dismissed stays hidden
Severity: Should-fix (frontend, `src/startup-notice.ts`)
Fix under review: SERVICE-R2-2 (R4)
Evidence class: reproduced with `adv-service/notice-repro.mjs`, which prints `second cancellation visible: false`
Location: `src/startup-notice.ts` (`startupNoticeVisible`, `dismissStartupNotice`); `src/App.tsx:504, 3722, 3730`
Trigger:
1. An import is cancelled by a privacy switch or by the app going to the background, and the band shows "Import cancelled …".
2. The person closes the band.
3. A later import succeeds, and `forget_startup_warning` withdraws the notice.
4. A further import is cancelled, and `note_startup_warning` adds the identical sentence again.
Causal path:
- `dismissed` only ever grows. `startupNoticeVisible` is `warnings.some(w => !dismissed.includes(w))`, so the re-added text counts as already dismissed and the band stays closed. That is the silent cancellation R4 was meant to remove.
- The new test even asserts that a withdrawn notice stays in the dismissed set (`assert.deepEqual(again, later)`).
Smallest correct fix:
- Prune `dismissed` to the current `startupWarnings` whenever the snapshot changes, for example in a `useEffect` on `snapshot.startupWarnings`, or inside both helpers.
- A notice that was withdrawn and then re-added is then new. A dismissed notice that is still present stays hidden, and a withdrawn one never reopens anything.
- Add the withdraw-then-re-add case to `test-startup-notice.mjs`.

### SERVICE-R3-2 — An unrelated successful import withdraws the notice
Severity: Nit
Fix under review: SERVICE-R2-2
Evidence class: source-traced
Location: `service.rs` `import_paths` (`if !books.is_empty() && !errors…any(IMPORT_CANCELLED) { forget_startup_warning(…) }`)
Trigger:
- Archive A's import is cancelled.
- The person then imports a different archive C, or reopens an already-registered archive (the LIB-3 fast path also counts as a book).
- The "Add the archive again" reminder disappears although A was never added again.
- The sentence names no archive, so nothing else tells them.
Smallest correct fix: keep the set of cancelled paths, and withdraw the notice only once each of them has been admitted (or the set is empty).

### Per-item status
- **SERVICE-R2-1: closed.**
  - Provider books are skipped in the `stale` filter.
  - No host test can tell the old and new code apart: on desktop, `archive_source::retain` already refuses provider references. Android type-check passed.
- **SERVICE-R2-2: partly closed** (SERVICE-R3-1, SERVICE-R3-2).
  - The R11 snapshot path adds no failure mode. `snapshot` already calls `self.store.read()?`, which goes through the same `Store::lock()`. `current_load_notice()` takes that lock only after the `startup_warnings` guard has been dropped, so the locks are never nested.
  - `unreadable_book_record_does_not_block_startup_or_snapshot`, `store::…unreadable_book_records…` and `launch_tests::unreadable_launch_archives…` pass.
- **SERVICE-R2-3 (`discard_cancelled_staged_copy`): closed.** It cannot remove a file the user or the library still needs:
  - Only the canonical parent `<data>/archives/mobile-imports-v1/<canonical uuid>/` qualifies. Both sides are canonicalized, `mobile_directory` creates exactly that shape, and the iOS data directory is Application Support, which file sharing does not expose.
  - Any library entry whose path is that file stops it. Admitted intake copies live permanently in these folders, so this check matters.
  - Split cohorts come from the primary's stem, so they cannot reach another archive in the same folder. `remove_dir` removes only an empty folder.
  - The UUID check in `remove_duplicate_intake_copy` is tautological here, since the UUID is read from the same file. Ownership rests on the folder rule, which is sufficient.
  - A retry cannot race it:
    - Intake attempts are serialized by `incoming_gate`, and every attempt gets a fresh `<uuid>` folder.
    - For picker copies, the discard runs inside the cancelled `import_paths` while that call still holds the read lease, so the cutover's writer, and every import admitted after it, waits for it.
    - Nothing was relinked or admitted before verification.
  - Accepted residuals:
    - An unopenable copy is kept, because its cohort cannot be proved.
    - If Android kills the process while it is in the background, the incoming URI grant may be lost after the staged copy was discarded. The person then shares again; the original is untouched at its source.
- **SERVICE-R2-4 (cutover count): closed.**
  - There is no `panic` profile setting, so panics unwind and `Drop` runs. It also runs when a future holding the guard is dropped.
  - Poisoning is tolerated through `into_inner`. `saturating_sub` cannot underflow.
  - A fresh token is installed only at a count of zero and never once `stopping` is set; Quit's cancellation stays in force.
  - No `mem::forget` path exists. `cancel_imports` is now gated to match `shutdown`.
- **SERVICE-R2-5: closed.** When the folder is already on its recorded device, there is no profile read and no stat; otherwise the work runs in `blocking`.
- **Flaky `spelling` test: closed.**
  - The "cannot panic in production" claim is right. `dictionary_with_loader` holds `inner` across `spawn` and `loads.insert`, so the worker's `loads.remove` (`spelling.rs:410`) always comes after the insert. There is no unwrap in production.
  - The panic was only the test helper's `.unwrap()` on a handle the worker had already removed.
- **Integrity flake: not reproducible, and I have no panic line.** My round-2 second run was piped through `grep` and its output was not kept. I saw only `test integrity::tests::real_valid_and_checksum_corrupt_results_survive_restart ... FAILED`. It then passed 3 of 3 in isolation, and `integrity.rs` is unchanged since base.
- **Desktop R8 comments: fine.**

### What I ran
- Lane tests including `store::`: `test result: ok. 484 passed; 0 failed; 6 ignored`.
- `launch_tests::unreadable_launch_archives…`, `service::…unreadable_book_record…` and `store::…unreadable_book_records…`: `3 passed`.
- Host clippy `--lib --tests --features custom-protocol -D warnings`: `Finished`, exit 0.
- `android-clippy.sh`: `Finished`, exit 0.
- `npx --no-install tsc --noEmit`: exit 0.
- `node --test scripts/test-startup-notice.mjs`: `pass 2, fail 0`.
- Scratch reproduction script for SERVICE-R3-1.

### Lane residuals once SERVICE-R3-1 is fixed
- LIB-7: a folder never seen in this run with its archives present keeps entries that were deleted while the app was closed, shown as unavailable and removable by hand. Windows behaves as before. A file imported from the directory under an unmounted mount point is retired once the volume covers it again.
- Provider-backed books get no rename-over detection, by design.
- A cancelled copy that cannot be proved is kept on disk (accepted leak).
- Mobile manual imports and intakes are cancelled on background or screen lock (owner decision). The person adds the archive again, or the intake is re-offered.
- The band heading "Some saved items need attention." is not about imports, and when the band reopens it also lists notices that were dismissed earlier (Nit).
- Docs: `docs/ERRORS.md` needs regenerating (its check fails repo-wide), and `mobile_policy.rs:566` still lists the removed command.
- Not verified here: no iOS compile; no device runs (lifecycle, provider descriptors, real mounts); the real shell rendering.