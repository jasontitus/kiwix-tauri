Lane `desktop` — follow-up on the adversarial review. All seven requested items are done. All checks are clean: lane tests (127 passed), host clippy, Windows-gnu clippy and the Android type-check. Each new or changed behaviour test fails against a mutated copy of the tree. The diffs below are against `d98cac2` and cover only what changed in this round.

### DESKTOP-R1 — The installer hook replaced the plugin's `cleanup_before_exit`
Disposition: fixed.
Verification: in `tauri-plugin-updater-2.14.1/src/lib.rs:120-123` the plugin's own hook only calls `app_handle.cleanup_before_exit()`. `UpdaterBuilder::on_before_exit` replaces any earlier hook. In `tauri-2.12.5 app.rs:1128` that cleanup clears the resource tables and, on Windows, hides every window.
Change: `update.rs` `updater_for` now captures `app.clone()`. The hook runs `run_before_installer_exit(&service)` and then `app.cleanup_before_exit()` on the calling thread, after the flush has finished or timed out. `docs/STATE_MACHINES.md` §1.5 says so.
Tests: none; this only runs on Windows inside the plugin. Linux and Windows-gnu clippy compile it.
Not exercised: real Windows window hiding before `ShellExecuteW`. If `ShellExecuteW` fails, the windows have already been hidden. That is the same as the plugin's default behaviour, and §1.5 records it.

### DESKTOP-R2 — `lastAvailableVersion` was cleared before the installer's outcome was known
Disposition: fixed differently. There is no new dependency: `semver` is only a transitive crate, so I wrote a small local comparator instead of adding it.
Change (`update.rs`):
- The hook no longer writes `update-state.json`.
- The Windows `map_err` re-record is removed, since nothing clears the version any more.
- New `still_available(recorded, current)` and `version_core`.
- `status` hides a recorded version that is ≤ the running version, comparing `MAJOR.MINOR.PATCH` with pre-release ordering. A pre-release of the same version counts as installed when a release is running. Build metadata is ignored. Anything unparsable, or two different pre-releases of the same version, keeps the recorded value.
Tests, all passing:
- `update::tests::a_recorded_update_is_hidden_once_this_version_has_it` (pure cases).
- `status_reports_an_update_only_while_it_is_not_installed`: a real `status()` call covering both relaunch cases. After a successful install the running build finds its own version recorded, so `None`. After a failed install the newer version is still shown.
- `the_windows_installer_hook_saves_what_quit_would_before_the_installer_runs` (renamed): now asserts that the hook leaves `lastAvailableVersion` and `lastError` untouched.
- Observed failing without the fix, in a copy.

### DESKTOP-R4 — `reset_saved_windows` rewrote a newer-version profile
Disposition: fixed.
Change (`store.rs`):
- New `foreign_version_error(&Value)` (`cfg(any(mobile, test))`) is shared by `load_mobile_state` and `reset_saved_windows`.
- `reset_saved_windows` refuses any version other than 1 or `STATE_VERSION`, with the same "different version of Kiwix… Your saved library was not changed." message, before it touches the file.
Tests: `store::tests::reset_saved_windows_refuses_a_profile_another_version_saved` checks the error and that the bytes are unchanged. It passes, and fails without the fix in a copy.

### DESKTOP-R5 — Leaving saved read-only mode dropped skipped records without a copy
Disposition: fixed.
Change (`store.rs`):
- New `Inner.skipped_unpreserved` flag. Both constructors set it when the load skipped records while read-only. On mobile it is computed before the relocation notices are appended.
- In `update_inner`, after `candidate.validate()` and before the mobile-relocation and portable blocks (either could write first), a commit that clears `settings.read_only` while the flag is set does three things: re-reads `state.json`, calls `preserve_skipped_original`, and clears the flag.
- If the copy cannot be kept, the commit is refused with "Kiwix could not keep a copy of the saved items it cannot read before leaving read-only mode (…). Free some disk space, then try again.", and the profile stays read-only and unchanged.
- `state_bytes` lost its `cfg(any(mobile, test))` gate because desktop now uses it.
- The edition-transaction code was not touched.
Tests: `leaving_saved_read_only_mode_keeps_the_skipped_records_first` and `leaving_read_only_mode_is_refused_while_the_copy_cannot_be_kept` both pass. Both fail without the fix in a copy.

### DESKTOP-R6 — The minimized skip also applied to macOS and Linux
Disposition: fixed.
Change: `windows.rs` `captured_geometry` skips a minimized window only under `cfg!(windows)`. The zero-size and non-finite guards still apply on every platform. The doc comment was updated.
Tests: `windows::tests::a_minimized_window_keeps_its_last_saved_frame` was extended: on non-Windows, a minimized window's moved frame is saved. It passes, and fails with the old unconditional skip in a copy.
Not exercised: real macOS or Linux minimized frames.

### DESKTOP-R8 (my parts)
Disposition: fixed.
Change:
- New test-only `fail_next_preserved_copy_for_test` / `FAIL_NEXT_PRESERVED_COPY` injection in `write_new_private_file`.
- `a_copy_that_cannot_be_kept_is_reported_and_never_blocks_opening` now actually opens a `Store` with the copy failing. It asserts the book loads, no copy exists, `state.json` is unchanged, and the notice reports the failure.
- `run_before_installer_exit` now logs `Disconnected` (a panicked flush thread) separately from `Timeout`.
- I left the stale comments in `service.rs` and `service_tests.rs` alone, as instructed.

### Item 7 — Relaunch wording after the startup-lane change
Disposition: fixed (docs and comments only).
Verification: I read `lib.rs` `finish_authorized_quit` and `ask_after_failed_quit`, and `quit.rs` `failed_quit_action`:
- Retry keeps the relaunch.
- "Quit Without Saving" retires again and then relaunches via `finish_authorized_quit`.
- Cancel or closing the dialog clears it only after a `Flush` failure; after a failed retirement it is kept.
Change: rewrote the `RESTART_AFTER_QUIT`, `take_restart_after_quit` and `clear_restart_after_quit` doc comments and the comment in `install`, plus the matching §1.5 sentence. `lib.rs` and `quit.rs` were not edited.

**Commands run:**
- `cargo test --locked --lib -- store:: update:: windows:: sandbox:: menus:: unreadable_book_record` → `test result: ok. 127 passed; 0 failed; 1 ignored`.
- Fail-without-fix, in a copy of the tree under my scratch directory with the shared target dir, filtered to the changed tests → `6 failed` (`a_recorded_update…`, `status_reports…`, `reset_saved_windows_refuses…`, both `leaving_…`, `a_minimized_window…`). The copy has been deleted.
- Host `cargo clippy --lib --tests --features custom-protocol -- -D warnings` → `Finished`. An earlier attempt failed only on another lane's `service.rs:211` (`import_hook` type complexity); that was gone on retry.
- Windows-gnu `cargo clippy --target x86_64-pc-windows-gnu --lib --tests --features custom-protocol -- -D warnings` → `Finished`.
- Android `android-clippy.sh` → `Finished`. The first attempt hit other lanes' mid-edit errors in `localswarm/mod.rs` and `dispatch.rs`.
- Not run: `tsc` and the node tests, since nothing in this round touched TypeScript.

**Files changed this round:** `src-tauri/src/update.rs`, `src-tauri/src/store.rs` (loader, `reset_saved_windows`, `state_bytes` gate, `Inner` flag, the one block in `update_inner`), `src-tauri/src/store_tests.rs`, `src-tauri/src/windows.rs`, `docs/STATE_MACHINES.md` (§1.5 only).

**New user-visible strings (Rust):**
- "Kiwix could not keep a copy of the saved items it cannot read before leaving read-only mode ({error}). Free some disk space, then try again."
- `reset_saved_windows` now returns the existing "different version of Kiwix" message. That message moved into `foreign_version_error`; its text is unchanged.
- `docs/ERRORS.md` needs regenerating (R7, your item).

**Handoffs / contract changes:**
- The `update_status` JSON is unchanged in shape. `lastAvailableVersion` is now omitted when it is ≤ the running version.
- A profile saved read-only with skipped records now has a new possible error on the save that leaves read-only mode.

**Doubts:**
- My comparator is not full semver: two different pre-releases of the same core version are always kept.
- The read-only exit preservation re-reads `state.json` and assumes nothing changed it while the profile was read-only, which holds because the store refuses all writes then. A hand edit made during the session would be what gets copied.
