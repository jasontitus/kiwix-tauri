Lane `desktop` — implementer report. All 8 findings are handled: 7 are fixed and BUILD-2 is partial (see below). Three changes outside my files are needed (handoffs below), and one of them fixes a test my change knowingly breaks.

### CORE-4 — Records skipped at load are lost at the next save; desktop users are never told
Disposition: fixed (plus the mobile version-mismatch variant). One residual: a profile saved read-only that is later switched out of read-only (see Doubts).
Verification of the finding: I read `decode_records` and `state_from_value`, which returns `normalized && warnings.is_empty()`. Both load paths (`open_profile` and `load_mobile_state`) only persist on migration or normalization, so the next ordinary `update` writes a `state.json` without the skipped records. `from_store` gave desktop no startup warning. On mobile, `load_mobile_state` quarantined the profile for any decode error when no sync metadata was present, including a version mismatch. The mobile caller (`mobile.rs` `start` → `Err`) shows the boot-guard failure panel, so returning `Err` there is safe.
Change:
- `store.rs`:
  - New `skipped_records_notice`, `preserve_skipped_original` and `write_new_private_file`.
  - When the load produced warnings and the profile is writable, the original bytes are kept once as `state.skipped-<first 16 hex of sha256>.json`. The file is created `create_new` with mode 0600 and fsynced, then the directory is fsynced (not on Windows, matching `persist`). A partial file is removed.
  - The name is content-addressed, so relaunching before any save reuses the copy. A same-named file with different content (an interrupted copy) is never followed or overwritten; a uuid-named copy is written beside it.
  - This runs before the v1 migration write in both `open_profile` and `load_mobile_state`.
  - New `Store.load_notice` field and `Store::load_notice()` accessor. The field doc that said "never persisted back over the original" is corrected.
  - `load_mobile_state` now detects a numeric `version` other than 1 or 2 right after parsing and returns an error instead of quarantining: "Kiwix cannot open your saved library because a different version of Kiwix saved it (profile version N; this version reads version 2). Install the latest version of Kiwix to open it. Your saved library was not changed."
  - What the mobile user sees: the existing boot-guard panel with that text and the existing "Reset saved window state" and "Quit" actions. Before, the app started empty after renaming the library away.
- `service.rs` `from_store`: `startup_warnings` now includes `store.load_notice()` on both desktop and mobile. The individual content-free warnings still go to diagnostics only.
- The four cases you asked about:
  - (a) No skipped records: no notice, no copy, no new write. `state.json` is byte-for-byte unchanged (tested).
  - (b) Read-only profile (runtime or saved) with skipped records: nothing is written. The notice says the file was left unchanged and asks the user to back it up before leaving read-only mode.
  - (c) Second launch: after any save, nothing is skipped, so there is no notice and no new copy. Before any save, the same copy is reused and the notice names it again.
  - (d) Low disk (or any write failure): the partial copy is removed and the profile still opens. The notice says "Kiwix could not keep a copy of the saved profile (<error>). Back up <state.json> now…". The next `persist` needs a temporary of about the same size, so on a full disk it most likely fails too and the records stay on disk.
Tests (`store_tests.rs`): `skipped_records_survive_the_next_save_in_a_preserved_copy`, `relaunching_before_a_save_reuses_the_preserved_copy`, `a_profile_without_skipped_records_opens_without_a_copy_or_notice`, `a_read_only_profile_with_skipped_records_is_not_written_and_says_so`, `a_copy_that_cannot_be_kept_is_reported_and_never_blocks_opening`, `mobile_loader_keeps_a_copy_before_its_migration_write`, `mobile_loader_refuses_another_schema_version_without_quarantining_it`. All pass. With the fix disabled, I observed the preservation tests and the mobile version test fail. The version test failed only after I rewrote it without sync keys: a `State::default()` serialization contains `readingSync`, which had been masking the bug.
Not exercised: the mobile runtime (Android type-check only) and the frontend band display.

### BUILD-12 — No retry on Windows when replacing `state.json`; remove-then-rename branch in `update.rs`
Disposition: fixed.
Verification of the finding: Windows `replace_file` made a single `move_file` call. `retry_transient_sharing(FnMut() -> io::Result<T>)` fits, since each attempt repeats the same move of our own temporary file. Its other callers (`persist`, `persist_session`, `write_state_value`, `portable.rs`) are all temp-then-replace.
Change: `store.rs` Windows `replace_file` is wrapped in `retry_transient_sharing`. `update::save_state_at` now calls `crate::store::replace_file` (`io::Result` mapped to `String`), removes its temporary on failure, and the remove-then-rename branch is deleted.
Tests: `update::tests::saving_update_state_replaces_the_previous_file_and_leaves_no_temporary` passes. It is a behaviour-preserving refactor on Linux, so it does not fail without the fix.
Not exercised: the Windows retry at runtime. I added no `cfg(windows)` sharing-violation test because none can run here. The Windows clippy of the library is clean.

### BUILD-2 — Windows self-update exits inside the plugin
Disposition: partial. The shells' in-memory session flush is deliberately not run (handoff below).
Verification of the finding:
- Plugin 2.14.1 `updater.rs:1009-1050`: `on_before_exit` is an `Arc<dyn Fn() + Send + Sync>`, called synchronously after `extract` and before `ShellExecuteW` and `std::process::exit(0)`. It runs on whichever thread calls `Update::install`, here the tokio task running `update::install`.
- The context is cloned into each `Update`, so the hook travels inside `PENDING`.
- `install` holds `_privacy` (`privacy_gate.read_owned`) and `_installation` at that point, and `Service::shutdown` awaits `privacy_gate.write()`, so calling it from the hook would deadlock. I confirmed both in the source.
- `quit::prepare` is unsafe here: it leaves every shell in `quitting` with reader storage frozen and the attempt prepared. If `ShellExecuteW` then fails, `quit.rs` has no public function to release them.
Change (`update.rs`):
- `updater_for(.., service: Option<&Arc<Service>>)` registers `on_before_exit` with a `Weak<Service>`. It is a no-op outside Windows.
- `run_before_installer_exit` runs `before_installer_exit` on its own thread with a 5 s `recv_timeout`; on timeout the install continues.
- `before_installer_exit` takes no lease. It runs `windows::capture`, `save_session_snapshot` for the open shell labels (via `quit::shell_label`), `store.flush_browsing()`, and clears `lastAvailableVersion`/`lastError`.
- If `update.install` returns an error on Windows, the still-pending version is recorded again.
- `docs/STATE_MACHINES.md` §1.5 has a new Windows paragraph. Nothing else in that file was touched.
Tests: `update::tests::the_windows_installer_hook_saves_what_quit_would_and_clears_the_update`. It holds a privacy read lease with a writer queued, runs the hook, and asserts it finishes before the timeout, the coalesced scroll is on disk, and the update state is cleared. It passes, and I observed it fail with the hook disabled.
Not exercised: the real Windows installer path, and window capture inside the hook (the test has no AppHandle).

### BUILD-9 — A future `lastChecked` suppresses automatic checks
Disposition: fixed.
Verification of the finding: a negative `whole_seconds` was never `>=` the interval.
Change: `check_due` treats `checked > now` as due. `next_check_at(last, now)` returns `now` in that case, and its caller in `status` was updated.
Tests: `update::tests::a_check_recorded_in_the_future_is_due_now` passes, and I observed it fail without the fix.

### BUILD-7 — Closing while minimized overwrites the saved geometry
Disposition: fixed, with a slight change to the suggestion.
Change (`windows.rs`):
- `capture` records an `Observed` frame (now including `is_minimized`).
- New pure `captured_geometry(observed, previous)` returns `None` when the window is minimized or reports a non-finite or zero size, so the last good frame is kept.
- When maximized or fullscreen, it keeps the previous normal width and height but uses the current position. This keeps a maximized window on the monitor it is on, instead of moving it back to the old frame's monitor.
- `persist_capture` merges inside `update_window_capture`. The stored schema is unchanged.
Tests: `windows::tests::a_minimized_window_keeps_its_last_saved_frame` and `a_maximized_window_keeps_its_normal_size_on_its_current_screen` pass. I observed both fail without the fix, after changing the minimized case to an icon-sized 160×28 so it tests the flag and not only the zero size.
Not exercised: real Win32 minimized rectangles.

### APPLE-2 — A dead security-scoped bookmark is never retried
Disposition: fixed, plus one handoff for the availability pass.
Change (`sandbox.rs`):
- `Bookmarks::present` is a new default trait method (symlink_metadata; NotFound means absent), so the native implementation is unchanged.
- New `Registry::revive(native, now)`, rate-limited to once per 5 s (`REVIVE_INTERVAL`). It only resolves dead roots whose path is present, so an unplugged volume or share is not looked up repeatedly. A success removes the root from `dead`, applies a refreshed bookmark and sets `dirty`; it writes nothing itself.
- It is called at the start of `Registry::grant`.
- New free function `sandbox::revive()` (macOS block mirrors `restore`), called at the start of `folders_needing_access`.
- `Fake` gained a switchable `broken: Cell<bool>` and a `mounted` set.
Tests: `a_bookmark_that_failed_at_launch_works_once_its_volume_is_mounted` and `a_save_revives_a_dead_bookmark_before_deciding_to_replace_it` pass, and I observed both fail without the fix.
Not exercised: the `#[cfg(target_os = "macos")]` body of `sandbox::revive()` is not compiled here.

### APPLE-3 — macOS menus
Disposition: fixed in Rust and the label contract; the Reader shortcut table is a handoff.
Change (`menus.rs`):
- New `MACOS_ACCELERATORS` and pure `platform_accelerator(action, default, macos)`. On macOS: Settings is `CmdOrCtrl+,`, Table of contents is `Alt+CmdOrCtrl+T` (⌥⌘T), Fullscreen is `Ctrl+Cmd+F`. Windows and Linux keep the `ACTIONS` keys exactly.
- On macOS, Settings is placed in the application menu.
- New application menu items under `cfg(macos)`: About Kiwix, Services, Hide Kiwix, Hide others, Show all, each followed by separators as standard. Constructors checked against tauri 2.12.5 `menu/predefined.rs`: `about(app, Some(..), None)`, `services`, `hide`, `hide_others`, `show_all`, `separator`. The `NativeMenus.application: [PredefinedMenuItem; 5]` field follows the existing `standard` pattern.
- `MenuLabels` gained `about`, `services`, `hide`, `hideOthers`, `showAll`, validated on every platform and applied on macOS.
- `src/App.tsx` `nativeMenuLabels` gained the same five keys, and nothing else in that file.
- `macos_menu_keys::normalize` only rewrites Shift items, and none of the new keys use Shift.
Tests: `menus::tests::macos_accelerators_leave_system_keys_alone_and_none_repeat` checks the reserved macOS chords, uniqueness per platform, and the predefined Edit/Quit keys. `windows_and_linux_keep_their_documented_accelerators` checks that the other platforms are unchanged. Both pass; I observed the macOS test fail without the overrides.
Not compiled here: every `cfg(target_os = "macos")` edit in `menus.rs` (the application menu block, the struct field, the label loop).

**Commands run** (all with `SHERPA_ONNX_ARCHIVE_DIR` set):
- `cargo test --lib -- store::tests:: update::tests:: windows::tests:: sandbox::tests:: menus::tests::` → `test result: ok. 111 passed; 0 failed; 1 ignored`
- The same run with every fix disabled by script → 12 of my new tests FAILED, then I restored the fixes.
- `cargo test --lib -- unreadable startup skipped quarantin` → 32 passed, 1 failed: `service::tests::unreadable_book_record_does_not_block_startup_or_snapshot`, the expected contract change (handoff 1).
- Linux `cargo clippy --lib --tests --features custom-protocol -- -D warnings` → `Finished` (clean).
- Windows-gnu clippy `--lib` → `Finished` (clean). With `--tests` it fails only on `src/library_files_tests.rs:1252` (`single_fixture` unused), which is another lane's file.
- Android clippy script → `EXIT=0`. It finished in 0.39 s from cache, so it was likely built by another lane's run after my edits.
- `npx --no-install tsc --noEmit` → exit 0.
- `node scripts/check-unsafe-census.mjs` → passes; I added no `unsafe`.
- `node --test scripts/test-*.mjs` → the localization manifest test fails on my 5 new UI keys (plus many reader failures from other lanes). The `docs/ERRORS.md` test also fails; it is generated, for the owner to regenerate.

**Files changed:** `src-tauri/src/store.rs` (allowed areas only), `src-tauri/src/store_tests.rs`, `src-tauri/src/service.rs` (`from_store` warning plumbing only), `src-tauri/src/update.rs`, `src-tauri/src/windows.rs`, `src-tauri/src/sandbox.rs`, `src-tauri/src/menus.rs`, `src/App.tsx` (`nativeMenuLabels` only), `docs/STATE_MACHINES.md` (§1.5 only).
**New user-visible strings:**
- Frontend `t(...)`: "About Kiwix", "Services", "Hide Kiwix", "Hide others", "Show all".
- Rust: the three skipped-records notices in `skipped_records_notice`, and the mobile "Kiwix cannot open your saved library because a different version…" error.
- The localization manifest and `docs/ERRORS.md` need regenerating.

**Handoffs:**
1. `src-tauri/src/service_tests.rs` around line 5026: `unreadable_book_record_does_not_block_startup_or_snapshot` asserts `startupWarnings` is empty, which contradicts the requested notice. Replace that assertion with: `let warnings = snapshot["startupWarnings"].as_array().unwrap(); assert_eq!(warnings.len(), 1); assert!(warnings[0].as_str().unwrap().contains("state.skipped-"));`
2. `src-tauri/src/service/dispatch.rs`, `"library_maintenance"`: call `crate::sandbox::revive();` as the first statement of the `blocking(move || { ... })` closure, before availability is computed. Otherwise the library keeps showing a re-mounted volume as missing until a save or folder prompt revives it.
3. `src/components/Reader.tsx:7585`: change `[t("Fullscreen"), t("F11")]` to `[t("Fullscreen"), isMacos ? t("⌃⌘F") : t("F11")]` and import `isMacos` from `"../api"` (the file already imports from there). Table of contents and Settings are not listed in that table.
4. Optional, `quit.rs` (lane `startup`): to also flush the shells' in-memory sessions before the Windows installer, `update.rs` would need a public `quit::abandon(app, reason)` that cancels a prepared attempt and emits `kiwix-quit-cancelled`, to call when `update.install` fails. Without it, `quit::prepare` cannot be used safely from the hook.

**Doubts:**
- A profile saved read-only that has skipped records, where the user then turns read-only off in the same session: the first save drops the records with no copy. The notice warns about this, but closing the gap needs a lazy preserve-before-first-persist in `update_inner`, which is outside my allowed edits.
- On mobile, "Reset saved window state" on a newer-version profile rewrites `windows`/`tabs` in that newer file (the version stays the same, so it still fails to open). `reset_saved_windows` arguably should refuse a foreign version.
- The notice text names a file path and suggests putting the copy in place of `state.json`. That is meaningful on desktop but not actionable on mobile.
- The Windows hook does not flush shell sessions, so changes made within the 600 ms autosave debounce before the hook are lost.
- I chose ⌥⌘T for Table of contents myself (it is not a system key). Product may prefer another key.
