## Verdict for lane desktop: only non-blocking findings

I found nothing in this lane that loses or corrupts data, hangs, crashes or fails to compile on a platform I could build. Two caveats: APPLE-2 is only partly closed (R3), and the localization manifest and `docs/ERRORS.md` checks fail and will turn CI red (R7). Review base: `d98cac2`, working tree as of 2026-11-28 ~00:40 UTC. Lane files were last modified at 00:20:44.

### DESKTOP-R1 — The Windows installer hook replaces the plugin's own `on_before_exit` (`cleanup_before_exit`)
Severity: Should-fix
Fix under review: BUILD-2
Evidence class: source-traced
Location: `src-tauri/src/update.rs:466` (`updater_for`); plugin `tauri-plugin-updater-2.14.1/src/lib.rs:120-123`, `updater.rs:405-409`
Trigger: any Windows self-update.
Causal path:
- The plugin's `updater_builder()` already registers `builder.on_before_exit(move || { app_handle.cleanup_before_exit(); })`.
- `UpdaterBuilder::on_before_exit` calls `self.context.on_before_exit.replace(Arc::new(f))`, so the new registration overwrites that hook.
- `tauri-2.12.5/src/app.rs:1128-1139` `cleanup_before_exit` clears the resource tables and, on Windows, calls `window.hide()` on every window. That no longer happens before `ShellExecuteW`/`exit(0)`.
- The windows now stay visible and interactive for up to the 5 s flush. Anything the user does in that time is lost.
Smallest correct fix: at the end of `run_before_installer_exit`, after the flush thread finishes or the 5 s timeout passes, call `app.cleanup_before_exit()` on the calling thread, as the plugin default did. Say so in STATE_MACHINES §1.5.

### DESKTOP-R2 — `lastAvailableVersion` is cleared before the installer's outcome is known
Severity: Should-fix (low frequency)
Fix under review: BUILD-2
Evidence class: source-traced
Location: `src-tauri/src/update.rs:1087` (`before_installer_exit`); `src/components/UpdatesSettings.tsx:327-339`
Trigger: on Windows, `ShellExecuteW` succeeds but the NSIS installer then fails or is closed (disk full, file in use, the user ends it). The user relaunches the old version.
Causal path:
- The hook writes `stored.last_available_version = None`, then the process exits.
- The re-record in `install`'s `map_err` only covers a `ShellExecuteW` failure.
- On relaunch, `PENDING` is empty and `lastChecked` is set, so Settings shows "You have the latest version." and nothing about the update.
- `automatic_check` does not run again until 24 h after `last_checked`.
- At base this path showed "Version X was available at the last check."
Smallest correct fix: do not clear the version in the hook. In `status` (or once at startup), drop or hide `last_available_version` when it is ≤ `current_version` (semver compare). This covers success and failure on every platform.

### DESKTOP-R3 — APPLE-2 is not revived from the library availability pass or the archive-open path (handoff 2 not applied)
Severity: Should-fix (needed to close APPLE-2)
Fix under review: APPLE-2
Evidence class: source-traced
Location: `src-tauri/src/service/dispatch.rs:1852` (`"library_maintenance"`, unchanged); `src-tauri/src/library_files.rs:110-111` (`Directory::open` → `explain_folder_error`)
Trigger: launch with the external volume unplugged, plug it in, then open a library book before any store save has happened at least 5 s after the last revive.
Causal path:
- `revive` runs only from `Registry::grant`, which runs inside `persist`, and from `folders_needing_access`.
- `folders_needing_access` is reached only from import or drop (`src/api.ts:1402`, `App.tsx:2046`) and Open With (`lib.rs:844`).
- Neither the availability pass nor the book-open path calls it, so the book still shows as missing or fails with "Kiwix does not have access to the folder…".
- The finding's smallest fix named the availability pass explicitly.
Smallest correct fix:
- Apply handoff 2: call `crate::sandbox::revive()` first inside the `library_maintenance` blocking closure.
- Preferably also, where `Directory::open` gets a sandbox denial, call `sandbox::revive()` and retry once.

### DESKTOP-R4 — The mobile boot-guard "Reset saved window state" rewrites a newer-version profile and reports success
Severity: Should-fix
Fix under review: CORE-4 (mobile version variant); implementer doubt 2
Evidence class: source-traced
Location: `src-tauri/src/store.rs:1930-1959` (`reset_saved_windows`); `src-tauri/src/mobile.rs:808-815`
Trigger: the mobile app opens a `state.json` saved by schema version 3. The panel says "…Your saved library was not changed." The user taps "Reset saved window state".
Causal path:
- `reset_saved_windows` parses any JSON object and inserts `windows: {}`, `tabs: []`, `activeTab: null`.
- `write_state_value` then rewrites the whole newer-schema document through `serde_json::Value`: keys are reordered and the newer build's window and tab data is lost.
- It returns "Saved window state was reset. Quit Kiwix and open it again."
- The next launch fails with the same version error. The panel is newly reachable for this case, because before the change the file was quarantined.
Smallest correct fix: after parsing in `reset_saved_windows`, refuse unless `version` is 1 or `STATE_VERSION`, returning the same "different version of Kiwix" message. Add a store test.

### DESKTOP-R5 — Saved read-only profile with skipped records: turning read-only off in-session drops them with no copy
Severity: Should-fix (needs owner decision; implementer doubt 1)
Fix under review: CORE-4
Evidence class: source-traced
Location: `src-tauri/src/store.rs:2132-2135` (`open_profile` `writable`); `update_inner`
Trigger: the profile has `settings.readOnly` saved and a record this build skips. The user turns read-only off in Settings; the next update persists.
Causal path:
- At load `writable` is false, so only the read-only notice is produced and no copy is kept.
- The settings change that leaves read-only goes through `update_inner` → `persist` with the already-reduced `State`.
- Nothing preserves the original. Only the band text ("Back up that file before you leave read-only mode") protects the data.
Smallest correct fix: when a commit clears `settings.read_only`, `load_warnings` is non-empty and no copy was kept, re-read `state.json` before `persist` and call `preserve_skipped_original`. The file cannot change while read-only. Runtime `--read-only` cannot leave in-session, so it is not affected.

### DESKTOP-R6 — The minimized skip also applies to macOS and Linux, where a minimized frame is valid
Severity: Nit
Fix under review: BUILD-7
Evidence class: inference (macOS reports the real frame for a miniaturized window; not run)
Location: `src-tauri/src/windows.rs:515` (`captured_geometry`)
Trigger: on macOS, move a window, minimize it, then Quit. The move made since the last capture is not saved.
Smallest correct fix: gate `minimized` to `cfg!(windows)`, or detect the parked frame (x or y ≤ -32000, or zero size). The zero-size and non-finite guards already cover Windows.

### DESKTOP-R7 — Localization manifest and `docs/ERRORS.md` checks fail
Severity: Should-fix (CI)
Fix under review: APPLE-3, CORE-4
Evidence class: reproduced
- `node --test scripts/test-localization-inventory.mjs scripts/test-localization-resources.mjs` → `# fail 1`, including "Uncollected UI key: About Kiwix / Hide Kiwix / Hide others / Services / Show all".
- `node --test scripts/test-error-docs.mjs` → `not ok 1 - docs/ERRORS.md matches the error copy in the current sources`.

Smallest correct fix: regenerate the manifest and `docs/ERRORS.md` (integration owner).

### DESKTOP-R8 — Stale comments and an overclaiming test name
Severity: Nit
Evidence class: source-traced
Locations:
- `service_tests.rs:5026` still says "Recoverable record skips stay out of the blocking migration band" directly above the new assertion that one notice is there. `service.rs:760-762` has the same stale wording.
- `store_tests.rs` `a_copy_that_cannot_be_kept_is_reported_and_never_blocks_opening` calls `skipped_records_notice` on a missing directory and never opens a `Store`, so "never blocks opening" is not exercised.
- `run_before_installer_exit` logs "not saved in time" when the flush thread panicked (`Disconnected`), not only on a timeout.

### DESKTOP-R9 — Preserved copies are never cleaned up and keep history after Clear history
Severity: Nit (owner decision)
Evidence class: source-traced
Location: `store.rs:1873` (`preserve_skipped_original`)
Trigger: one portable profile used alternately by a newer and an older build. Each older-build launch after a newer-build save creates another full copy (up to 128 MiB), named after its distinct content. Each copy keeps reading history in plain text beside the profile, untouched by Clear history or private mode.
Smallest correct fix: decide on a policy, such as keeping the newest N copies, or mentioning the copies in Clear history. No code change is needed for correctness.

**Per-fix assessment**
- **CORE-4:** fixes it on desktop and mobile writable loads.
  - The copy is written before the v1 migration persist on both paths.
  - No new `?` can fail the open, and nothing in it can panic.
  - `create_new` (O_EXCL) never follows or overwrites an existing name, whether it is a symlink, a directory, a FIFO or a different file; a FIFO is never read.
  - Write failures (read-only directory, ENOSPC, FAT) only change the notice.
  - Directory fsync failure is only logged.
  - A profile with no skips takes the byte-identical path; the persist condition is unchanged.
  - The mobile version check returns the error only for a numeric version other than 1 or 2; a missing or non-numeric version is quarantined as before.
  - The notice reaches only `startupWarnings` and the UI band. `crash.rs` and `diagnostics.rs` never read startup warnings, so the path does not leak there.
  - Residuals: R4 and R5.
- **BUILD-12:** fixes it. Each retry repeats the move of our own temporary file, which has a UUID name. The last error is returned unchanged, so `committed: false` still leaves `state.json` intact. `win32_error` preserves codes 5 and 32. The `store.lock` flock excludes other writer processes. `save_state_at` removes its temporary on failure and maps the error to `String`.
- **BUILD-2:** fixes the main gap (geometry, session, coalesced browsing, stale version on success), but see R1 and R2.
  - The hook runs on the thread executing `Update::install` (a tokio worker), after `extract` and before `ShellExecuteW`.
  - Its body runs on a helper thread, so a panic there cannot unwind into the plugin.
  - Private and read-only mode are re-checked under the store lock in `persist_capture`, `save_session_snapshot`, `flush_browsing` and `save_state`.
  - `install`'s privacy read lease is held for the whole hook, so no cutover can interleave. The `Weak` handle is a no-op once the service is gone.
  - The Windows error path re-records the version. The `cfg(windows)` placement is correct and Windows-gnu clippy is clean.
- **BUILD-9:** fixes it. A future timestamp is due now, and `next_check_at` shows now. A forward clock jump costs at most one extra check after the correction.
- **BUILD-7:** fixes it for Windows, with an unchanged stored schema. The merge reads `previous` under the store lock inside `update_window_capture`. Maximized or fullscreen keeps the earlier normal size at the current monitor's position. R6 applies.
- **APPLE-2:** fixes it but see R3.
  - It cannot resurrect a removed bookmark: `release` prunes `dead` to `saved.roots`, and `revive` drops keys that are missing.
  - Each root starts access at most once, because a success leaves `dead`.
  - No re-entrant REGISTRY lock: `grant` calls the method, not the free function.
- **APPLE-3:** fixes it as far as I can read without compiling.
  - Signatures match tauri 2.12.5 / muda 0.20.0: `about(&M, Option<&str>, Option<AboutMetadata>)`, `services`, `hide`, `hide_others`, `show_all(&M, Option<&str>)`, `separator(&M)`, `Submenu::append(&dyn IsMenuItem)`. The `let [..] = &array` borrow ends before the move into `NativeMenus`.
  - Keys match exactly (`about`, `services`, `hide`, `hideOthers`, `showAll`).
  - Windows and Linux accelerators are byte-identical to base (`platform_accelerator` returns the default when not on macOS).
  - `tabbing.mm` uses ⌘{ ⌘} ⌘M only, so there is no collision with ⌥⌘T, ⌘, or ⌃⌘F.
  - muda parses `Ctrl`, `Cmd`, `Alt` and `,`. `normalize` only rewrites Shift items, so `Alt+CmdOrCtrl+T` is untouched.
  - The Reader shortcut table (handoff 3) is already applied (`Reader.tsx:7590`).

**Implementer's doubts**
1. Read-only toggle-off drops records: a real problem (R5); fix it, or the owner accepts the notice-only protection.
2. Reset on a newer-version profile: a real problem, small fix now (R4).
3. Notice text not actionable on mobile: acceptable for now. A mobile-specific wording would be better.
4. The hook does not flush shell sessions (600 ms debounce): acceptable and documented in §1.5. A `quit::abandon` API is an owner decision for lane `startup`.
5. ⌥⌘T for Table of contents: needs a product decision. I found no collision with the app's menus, `tabbing.mm`, or the AppKit/WebKit editing defaults I know (⌥⌘T is the usual "Show Toolbar" key in other apps, not system-wide).

**What I ran** (`SHERPA_ONNX_ARCHIVE_DIR` set)
- `cargo test --locked --lib -- store:: update:: windows:: sandbox:: menus:: unreadable_book_record` → `test result: ok. 122 passed; 0 failed; 1 ignored`. All 15 new lane tests and `service::tests::unreadable_book_record_does_not_block_startup_or_snapshot` passed.
- Host `cargo clippy --lib --tests --features custom-protocol -- -D warnings` → `Finished`, EXIT=0.
- Windows-gnu `cargo clippy --target x86_64-pc-windows-gnu --lib --tests --features custom-protocol -- -D warnings` → `Finished`, EXIT=0.
- `android-clippy.sh` → recompiled kiwix-workbench, `Finished`, EXIT=0.
- `npx --no-install tsc --noEmit` → EXIT=0.
- Localization and error-docs node tests → fail (R7).

Logs: `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-desktop/`

**What I could not verify**
- None of the `cfg(target_os = "macos")` code was compiled (`menus.rs` application menu, struct field, label loop; `sandbox::revive()` body). It was read against the sources only.
- Whether AppKit auto-inserts an "Enter Full Screen" ⌃⌘F item into the Window menu.
- Whether the App Sandbox permits the `lstat` in `present()`. If it is denied, it counts as present, and every dead root is re-resolved every 5 s under the REGISTRY lock and the store lock, which could block on a network bookmark.
- `lstat` hangs on dead network mounts; this was already true of `accessible()`.
- Windows runtime: sharing-violation retries, a real minimized rectangle, the installer hook with real windows, and what dropping `cleanup_before_exit` does there.
- Mobile runtime and the boot-guard panel.
