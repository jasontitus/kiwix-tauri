# Lane `startup` — report

All five findings are fixed in my owned files; `service.rs`, the frontend and every other lane's files are untouched. Nothing here was shown as a real dialog on a real desktop: Linux host tests ran, Windows code was type-checked only, macOS code was not compiled. A fully clean Windows clippy run was not obtained because of errors in other lanes' files (details under Commands).

One correction to the resume note: the `lib.rs` tests were already written when I was stopped; after resuming I ran them, fixed one Windows-only compile error of mine (`windows::` resolved to the crate's own `mod windows` in `lib.rs`; now `::windows::`), and did the fail-without-fix checks.

### CORE-2 / BUILD-1 — Fatal desktop startup errors are shown to nobody
Disposition: fixed (both paths), plus the cheap `--help`/`--version` fix for Windows.

Verification of the finding: Confirmed in the current tree.
- `run()` only did `eprintln!` + `exit(1)`.
- The setup closure returned `Err(error.to_string().into())`, and `tauri-2.12.5/src/app.rs:1443-1444` turns that into `panic!("Failed to setup app: {e}")`.
- `main.rs` sets `windows_subsystem = "windows"` for release builds.

Change:
- `lib.rs: startup_failure(error, headless) -> StartupFailure { dialog, exit_code }` is the testable decision. The dialog text is `"Kiwix could not start.\n\n{error}"` (store text kept whole); `--headless` gets no dialog; the exit code is always 1.
- `lib.rs: run()` now calls `exit_after_startup_failure`: the stderr line stays, then a native dialog, then `exit(1)`. Headless is decided from raw argv, so an unparsable `--headless` command line still reports on stderr only.
- `lib.rs: setup_reporting_failure(...)` wraps the setup closure (two-line diff at its open and close; body not re-indented). Any `Err` from setup goes to `startup_failed_in_setup`: stderr, hide any window already built, Tauri dialog, `exit(1)` in the callback, `Ok(())` to Tauri. This covers the store-open arm and the later `?`s (window build, menu install).
- I hide rather than close windows there: closing the last window raises an ordinary Quit, which would exit 0 before the message is read.
- `lib.rs: show_native_message` (Windows) is one `MessageBoxW` call via the already-enabled `Win32_UI_WindowsAndMessaging` feature. It lives in `lib.rs` because `instance.rs` is `#![forbid(unsafe_code)]` and in the forbid baseline.
- `instance.rs: show_native_message` is now `pub(crate)`, takes a `NativeMessageKind` (Warning/Error) and returns whether a dialog was shown; `show_profile_busy` keeps its stderr fallback.
- Linux: the helper returns immediately when both `DISPLAY` and `WAYLAND_DISPLAY` are unset or empty, so a no-display launch cannot wait on a dialog. I also added `--no-markup` to zenity, because a `&` or `<` in a path is otherwise parsed as Pango markup and blanks the message.
- Argument safety: text is passed as separate argv values through `Command::args`, never a shell; osascript text goes through the existing `apple_script_string` quoting. The message always starts with a fixed sentence, so it cannot begin like an option.
- `--help`/`--version`: `print_cli_answer` prints as before and, on Windows only, shows a message box when stdout's handle is null (std's documented behaviour for a windows-subsystem process with no redirected stdout). Redirected output gets no box.
- No startup-failure text reaches the crash report any more, because there is no longer a setup panic.
- Dialog text is English, like the busy-profile dialog: no profile, and so no saved language, has been read at that point.

Unsafe census, precisely what I added:
- One `unsafe { MessageBoxW(None, &message, &title, MB_OK | icon | MB_SETFOREGROUND) }` in `src-tauri/src/lib.rs`, with a `// SAFETY:` comment (owned NUL-terminated `HSTRING`s outlive the call; null owner permitted).
- `scripts/unsafe-census-baseline.json`: `matches` 74→75, `blocks` 74→75, new entry `"src-tauri/src/lib.rs": 1`.
- The script compares only that file and `scripts/unsafe-forbid-baseline.json` (unchanged); it checks no docs, so I edited none (see Handoffs).

Tests (all pass):
- `launch_tests::fatal_startup_errors_reach_a_dialog_and_exit_1_unless_headless` — uses the real `store::startup_hint` error for a `{` profile.
- `launch_tests::a_profile_saved_by_another_version_shows_the_stores_recovery_text` — real `Service::new_checked` on a version-3 file; asserts the three store sentences are intact and the file is unchanged.
- `instance::tests::native_message_needs_a_display_and_passes_text_only_as_argument_values`.
- Fail-without-fix is inferred: the functions did not exist before.

Not exercised:
- No dialog was shown on any OS; zenity and kdialog are not installed here.
- Windows `MessageBoxW` and `print_cli_answer` are type-checked only.
- macOS `show_native_message` (icon parameter) is not compiled here.
- The setup wrapper and `startup_failed_in_setup` need a real Tauri app and have no runtime test.

### CORE-1 — Quit cannot complete while the profile cannot be written
Disposition: fixed, with no `service.rs` change.

Verification of the finding: Confirmed. `windows::capture(...)?` was fatal in `prepare`; a `prepare` error only reset `exiting`; the shutdown-failure dialog was `OkCustom("Retry")`. Additional consequence I traced: closing the last window with an unwritable profile left the process running with no window, because that capture was the failing write.

Change:
- `quit.rs: prepare` — the geometry capture failure is logged, not fatal, like the session snapshot beside it.
- `quit.rs` — new `QuitFailure {Flush, Retirement}`, `FailedQuitLabels`, `FailedQuitAction {Retry, QuitWithoutSaving, Stay{clear_relaunch}}`, `failed_quit_action`, `failed_quit_message`.
- `lib.rs: ask_after_failed_quit` — one native dialog for both failures, buttons Retry / Quit Without Saving / Cancel (`YesNoCancelCustom` + `show_with_result`). `exiting` stays true while it is open, so dialogs cannot stack.
- Why three buttons: I read the plugin (`tauri-plugin-dialog-2.8.4/src/desktop.rs`) and rfd 0.17.2 backends. A closed dialog is reported as the Cancel-slot label, so with a two-button form the destructive action would have been reachable by closing the window. It sits in the "No" slot, which only its own button returns.
- `failed_quit_action` gives up state only for the exact Quit Without Saving label, and only if all three labels are distinct. Everything else maps to Retry or Stay.
- Quit Without Saving runs `retire_without_saving` = `Service::shutdown()` once more, with its error logged instead of vetoing, then `authorized=true`, then `finish_authorized_quit` (relaunch if armed, else exit).
- Self-update relaunch: a failed flush no longer clears `RESTART_AFTER_QUIT` behind the person's back. Retry keeps it, Quit Without Saving relaunches, Cancel clears it. After a failed retirement, Cancel keeps it (pre-existing semantics). No path sets `authorized` after a failure except the explicit button.
- `docs/STATE_MACHINES.md` §1.1 only: new Asking state and edges, two new invariants, and `begin_attempt` corrected to the real `needs_flush`. Another lane's §1.2/§1.4 edits are intact.

Steps on the forced path — every `Service::shutdown` stage runs, because it is the same function; only its `Err` stops being a veto:

| Stage | Runs on forced path |
|---|---|
| `stopping=true` | yes |
| `agent_connector_stop` | yes |
| `retire_private_sync`, `editions.retire_authorizations` | yes |
| `nearby_retire` | yes |
| folder watcher shutdown | yes |
| PDF jobs cancel/drain | yes |
| speech block | yes |
| `retire_searches`, `integrity.retire` | yes |
| `retire_localswarm` | yes |
| privacy write lease | yes |
| `kokoro.drain_download` | yes |
| `integrity.cancel_and_wait` | yes |
| `imports.shutdown_and_wait` | yes |
| `spelling.set_writable(false)` | yes |
| sensors deactivate | yes |
| downloads `shutdown` (checkpoint/pause) | yes |
| LocalSwarm `stop` | yes |
| map cache clear and `pause` | yes |
| torrents `shutdown` | yes |
| sharing server shutdown | yes |
| `retire_readers`, archive cache clear | yes |
| `native_pdf::retry_cleanup` | yes |
| `store.flush_browsing` | attempted; failure tolerated |

Exit-handler steps skipped only after a failed flush:
- The shell session flush is not repeated: it is the write that failed and the thing the person agreed to lose.
- `windows::capture` and `save_session_snapshot` are skipped: both are convenience store writes. Next launch restores the last saved session and geometry.
- After a failed retirement all three had already succeeded.

Tests (all pass):
- `quit::tests::a_failed_quit_can_be_retried_left_open_or_overridden_only_by_the_explicit_button` — covers closed dialog, bare results, wrong case, and colliding labels.
- `quit::tests::the_failed_quit_dialog_states_the_cause_and_what_quitting_without_saving_loses`.
- `launch_tests::quitting_without_saving_still_retires_the_service_when_the_profile_is_gone` — real `Service`, pending browsing delta, profile directory removed. `shutdown()` returns `Err("…Writing the last reading position…")` twice; then `retire_without_saving` completes and the service refuses commands.
- Fail-without-fix is inferred (new functions).

Not exercised: the dialog itself, and the handler wiring (`ask_after_failed_quit`, the `exiting`/`authorized` flags, relaunch), which need a live Tauri app. The one-line non-fatal capture in `prepare` has no test for the same reason.

### CORE-7 / LIB-4 — OS-opened archives that fail to import produce no message
Disposition: fixed.

Verification of the finding: Confirmed. `open_paths` returned `Ok(())` after `eprintln!("Archive open errors…")`; the callers reported only `Err`; the macOS `Opened` arm only `eprintln!`ed even hard errors.

Change:
- `open_paths` now returns `Result<Vec<String>, String>`. `Err` means nothing was opened. `Ok` lists one line per unopened archive (`"<file name>: <reason>"`, from `launch_failures`) after the readable ones are queued, emitted and focused exactly as before.
- Second-instance (Unix rendezvous and Windows plugin) goes through `spawn_secondary_paths` → `report_launch_outcome`: a native error dialog for both hard errors and per-file failures.
- Startup uses `note_startup_launch_outcome`: one startup warning per file, the existing `kiwix-open` emit, and show/unminimize/focus of `main` when something failed.
- Headless gets the same warnings plus stderr.
- macOS `Opened` now calls the same compiled `report_launch_outcome(&app, &service, "Import archives", outcome)`. **Not compiled here**: the arm is `cfg(target_os = "macos")`, and the only new lines are `service.clone()` and that call.

Tests: `launch_tests::unreadable_launch_archives_are_reported_and_readable_ones_still_open` — all-fail, secondary launch, mixed launch (one location queued, one failure), startup notice in the snapshot's `startupWarnings`, and the read-only "registered archives only" case. **Observed failing without the fix** (temporarily restored the silent `Ok(Vec::new())`: `left: 0, right: 1`), then restored byte-exact.

Not exercised: the dialogs and window focus; the macOS arm.

### CORE-8 — Crash-report path redaction stops at the first space
Disposition: fixed differently in two details.

Verification of the finding: Confirmed by running: with the old stop rule the test produced `Cannot parse [path] Doe\AppData\Roaming\kiwix\My Library\state.json: …`.

Change:
- `crash.rs: path_end` — a recognised path is redacted up to a closing `"` (not an escaped `\"`), `<`, `>`, a backtick, a line break, `:` followed by whitespace, or end of text.
- Deviation 1: I did not make `'` a stop, as the reviewer listed. Names like `/Volumes/Jane's Backup/…` would then leak everything after the apostrophe. The cost is over-redaction.
- Deviation 2: `replace_urls` had the same flaw for `file:///Users/Jane Doe/…`, so a `file:/…` URL now uses the same end rule.
- Backtrace `at file:line` handling is unchanged (`file_name`).
- The existing exact-match test `local_reader_and_non_http_urls_keep_no_path_or_host_identity` was reordered, because a path now swallows the rest of its clause; its `relative=` and `file:` cases are asserted separately.

Tests: `crash::tests::redaction_keeps_no_part_of_a_path_that_contains_spaces` (Windows profile, `/Volumes/…`, quoted, Debug-escaped, apostrophe, UNC, `~/`, file URL, backtrace frame line). **Observed failing without the fix.** The other four crash tests pass, including the isolated real-panic round trip.

Swift/Kotlin handoff (not edited): both have the same flaw; the path character class `[^\s"<>`]+` stops at the first whitespace.
- `src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/CrashIntake.kt:37` (`pathPattern`).
- `src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Sources/CrashIntake.swift:26-27` (`pathPattern`).
- Their URL patterns (`CrashIntake.kt:29-30`, `CrashIntake.swift:21-22`) likewise stop a `file:` URL at whitespace.

### BUILD-8 — Busy-profile dialog says "Activity Monitor" on Linux and Windows
Disposition: fixed.
Verification of the finding: Confirmed; the string was not platform-gated.
Change: `instance.rs: busy_dialog_message` picks "Activity Monitor" (macOS), "Task Manager" (Windows) or "your system monitor" (elsewhere).
Tests: `instance::tests::busy_message_names_the_profile_and_how_to_choose_another`, extended. **Observed failing without the fix** on Linux.
Not exercised: the Windows and macOS branches.

## What a person should see (not observed here)

(a) Corrupt `state.json` (`{`), launched from Finder / Start menu / launcher: no window; a dialog titled "Kiwix — cannot start" reading "Kiwix could not start." then "Cannot inspect saved policy in …state.json: … The original is untouched."; OK; exit 1.
- macOS: osascript dialog with a stop icon.
- Windows: a message box with an error icon.
- Linux: a zenity error dialog (kdialog if zenity is absent); with neither, or no display, stderr only and an immediate exit 1.
- A valid file from another version takes the setup path instead: the same title and text in the Tauri dialog (NSAlert / TaskDialog / GTK), with the store's "saved by a different version… Your saved file was not changed."

(b) Quit with the profile volume read-only: the windows return from "Quitting…" and show the in-app error banner. A native dialog titled "Quit" says "Kiwix could not save your open tabs and reading positions, so it did not quit.", then the cause, then the retry/loss paragraph, with buttons Retry, Quit Without Saving, Cancel.
- Retry: the same dialog again while the volume stays read-only.
- Cancel or closing the dialog: Kiwix stays open.
- Quit Without Saving: Kiwix exits.
- In private or read-only mode nothing is flushed, so the dialog appears only if a reading position is pending, with the "could not finish saving its work" wording.

(c) Double-clicking a corrupt `.zim` while Kiwix runs: the window comes forward, and after verification a native error dialog titled "Import archives" reads "The operation could not be completed." then "<file name>: <reason>", with one OK button. The library is unchanged. A sandboxed macOS build may show the folder-access panel first.

## Commands run
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- launch_tests quit:: crash:: instance:: launch_target_tests maintenance_cadence_tests navigation_policy_tests` → `test result: ok. 32 passed; 0 failed; 0 ignored; 0 measured; 1410 filtered out`.
- Same filters with three fixes temporarily reverted (crash, instance, launch) → `test result: FAILED. 1 passed; 3 failed`; files restored and verified with `cmp`.
- Linux clippy `--lib --tests --features custom-protocol -- -D warnings` → exit 0, `Finished` (fourth attempt; the first three failed only in `maps*` and `store_tests.rs`).
- Windows-gnu clippy (brief's command): my first run failed on my own `windows::` import, fixed. Five further runs each exited 101 with exactly one error, none in my files:
  - `library_files_tests.rs:1252` — `single_fixture` never used.
  - `sharing_interop_tests.rs:541` and `store_tests.rs:1385` — clippy lints in tests.
  - `acquisition/transfer.rs:748` — `blocks_in_conditions`.
  My `cfg(windows)` code produced no diagnostics in the lib or lib-test units, but a fully clean Windows run was not obtained.
- `node scripts/check-unsafe-census.mjs` → `Unsafe census: 75 matches / 75 blocks / 2 impls in 20 files; every item documented.`

## Files changed
`src-tauri/src/lib.rs`, `src-tauri/src/quit.rs`, `src-tauri/src/instance.rs`, `src-tauri/src/crash.rs`, `docs/STATE_MACHINES.md` (§1.1 only), `scripts/unsafe-census-baseline.json`. Not changed: `main.rs`, `instance/socket.rs`, `service.rs`, frontend.

New user-visible strings, all Rust-side; nothing passed to frontend `t(...)`.

Localized through `Localizer` — these fall back to English until added to `src/locales/extra-en-native.json` and regenerated:
- "Quit Without Saving"
- "Kiwix could not save your open tabs and reading positions, so it did not quit."
- "Kiwix could not finish saving its work, so it did not quit."
- "Retry after fixing the problem, for example by freeing disk space or reconnecting the drive that holds your Kiwix data. If you quit without saving, changes to open tabs and reading positions since the last successful save are lost, and unfinished transfers may have to repeat their most recent progress."

The dialog also reuses the existing keys "Retry", "Cancel", "Quit", "Import archives" and "The operation could not be completed."

English only, pre-locale:
- "Kiwix — cannot start"
- "Kiwix could not start."
- "Task Manager" / "your system monitor" in the busy dialog
- "The archive could not be opened" (fallback reason)

`docs/ERRORS.md` will need regenerating.

## Handoffs / contract changes
- **Mobile lanes:** the `CrashIntake.kt` / `CrashIntake.swift` path and URL patterns above.
- **Desktop lane** (`update.rs`, STATE_MACHINES §1.5): a failed Quit flush no longer clears the relaunch immediately; it is cleared only when the person answers Cancel or closes the dialog. Two texts are now stale: §1.5 "Cancelling the shell flush clears the relaunch", and the comment on `RESTART_AFTER_QUIT` in `update.rs` ("A cancelled prepare clears it…"). The relaunch point is now `lib.rs: finish_authorized_quit`. BUILD-2 (Windows updater exiting inside the plugin) is not addressed here.
- **Integration owner, docs:** `docs/RUST-UNSAFE-REVIEW.md` (~line 446) says "74 blocks in 19 code files"; it is now 75 in 20, with `lib.rs` added. `docs/MEMORY-SAFETY-INVENTORY.md` line 11 says 73, which was already stale.
- `open_paths` and `open_secondary_paths` now return `Result<Vec<String>, String>`; both are private to `lib.rs`. The stderr line "Archive open errors: …" is gone (no consumer found by grep); per-file lines are "Opening startup archives: …" or "<title>: …".
- zenity is now invoked with `--no-markup`; the busy dialog uses it too.

## Doubts
- **Windows non-interactive session:** `MessageBoxW` in an SSH or service session without `--headless` may block invisibly. I found no safe, reliable detection without more unsafe calls, so only `--headless` is exempt.
- **Dialog even from a terminal:** a windowed launch that fails shows the dialog as well as stderr when a display exists. I rejected an "stderr is a TTY" exemption because GUI sessions started from a VT (startx, sway) hand every app a TTY stderr, which would recreate the invisible failure. Scripts that expect an immediate exit 1 from a non-headless launch on a machine with a display and zenity will now wait for OK.
- **Partial setup failure:** if setup fails after the service exists (for example a window build failure), the dialog's OK calls `process::exit(1)` without `Service::shutdown`. This is no worse than the panic it replaces, and nothing has started transfers at that point, but it is not a clean retirement.
- **Hang on forced quit:** if `Service::shutdown` hangs (CORE-6's uncancellable import under the privacy lease), Quit Without Saving hangs too and `exiting` stays set. That is the transfers/core lane's fix.
- **Cancel after a failed retirement** leaves the shells frozen with the service stopping; the next Quit shows the dialog again. That is the pre-existing dismissal behaviour, now behind a button labelled Cancel.
- **Redaction residue:** a Unix file name containing `: `, `<`, `>`, a backtick or an unescaped `"` still ends redaction early. These are the reviewer's stops and were already stops before. The new rule also drops prose after an unquoted path, such as "(os error 2)".
- **Partial-success dialog heading:** it reuses "The operation could not be completed." to avoid a new localized string, although some archives did open; the lines below name the failures.
- **`--no-markup`** is long-standing in zenity 3.x and 4.x but could not be run here; a zenity too old to know it would fall through to kdialog, then stderr.
- **Wayland default socket:** a session with neither `DISPLAY` nor `WAYLAND_DISPLAY` set gets no pre-Tauri dialog (stderr only), including the existing busy-profile dialog.
- **Double reporting on flush failure:** each shell shows its error banner (from the existing `kiwix-quit-cancelled`) under the native dialog. I kept it because the banner is what remains if the person chooses Cancel.
