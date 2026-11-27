## BUILD review — Windows/Linux desktop, updater, connector, build/CI/docs (base d98cac2, read-only)

No repository file was modified. Nothing was compiled or run on Windows or Linux desktop; evidence is source tracing plus two Node test runs. Dependency behaviour was read from the locked sources in `~/.cargo/registry` (`tauri-plugin-updater 2.14.1`, `tauri 2.12.5`, `tao 0.37.2`).

### BUILD-1 — Fatal desktop startup errors are never shown to the user; the app just does not open
Severity: High        Category: usability
Platforms: Windows (always, release builds have no console), Linux and macOS when launched from a launcher/Finder
Evidence class: source-traced
Location: src-tauri/src/lib.rs:1399-1404 (`run`)  [+ lib.rs:888, 931, 951 (early returns in `run_application`), lib.rs:1113 (setup closure), src-tauri/src/main.rs:6, src-tauri/src/instance.rs:117-171, src-tauri/src/store.rs:3284-3296]
Trigger: `state.json` is unparseable or unreadable; a portable copy is on write-protected media; `--data-dir` points at a missing drive; a shortcut passes an unknown option. The user double-clicks Kiwix (or a `.zim`) and nothing appears.
Causal path:
- `run()` is `if let Err(error) = parse_options(..).and_then(run_application) { eprintln!("Kiwix: {error}"); std::process::exit(1); }`.
- `main.rs` sets `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`, so that stderr goes nowhere on Windows.
- A malformed profile returns at lib.rs:888 (`hint.read_only.clone()?`) with the text built in `store::startup_hint`: "Cannot inspect saved policy in …state.json: … The original is untouched."
- Portable failures return at lib.rs:931 (`profile.prepare(..)?`); a non-busy lock failure returns at lib.rs:951.
- Inside setup, `Err(error) => return Err(error.to_string().into())` (lib.rs:1113) becomes `panic!("Failed to setup app: {e}")` in `tauri-2.12.5/src/app.rs:1444`. Windows are created with `create = false`, so none exists yet.
- The only native startup dialog is the busy-profile one (`instance::show_native_message`, `profile_busy_in_setup`). The recovery text in `store.rs` ("restore a known-good backup or select another profile, not an empty replacement") can never reach a GUI user.
- `--help` and `--version` also print nothing on Windows release builds, for the same reason.
Smallest correct fix:
- In `run()`, show a native message before `exit(1)`: reuse `instance::show_native_message` on Unix (make it `pub(crate)`); on Windows call `MessageBoxW` (the `windows` crate's `Win32_UI_WindowsAndMessaging` feature is already enabled; this adds one `unsafe` block to the census baseline).
- At lib.rs:1113, handle `StoreOpenError::Fatal` like `profile_busy_in_setup`: close windows, show `app.dialog()` with the error, exit 1 from the callback, return `Ok(())` so Tauri does not panic.
- No contract changes; exit codes stay the same.
Regression test: in the `lib.rs` tests, factor a `report_startup_failure(error, reporter)` helper and assert the reporter receives the exact store message for a profile whose `state.json` is `{`. Assert the setup path maps `Fatal` to the dialog branch (same shape as the existing busy-profile test).

### BUILD-2 — Windows self-update exits inside the plugin, skipping Quit flush, `Service::shutdown` and the post-install cleanup
Severity: Medium        Category: reliability
Platforms: Windows
Evidence class: source-traced
Location: src-tauri/src/update.rs:972-1001 (`install`)  [+ update.rs:451-470 (`updater_for`), tauri-plugin-updater-2.14.1/src/updater.rs:1009-1050, src/components/UpdatesSettings.tsx:318-324, src-tauri/src/service.rs:3512 and its final `flush_browsing`, src-tauri/src/quit.rs:306]
Trigger: on Windows, Settings → Updates → Install update, with tabs open and a reading position changed in the last few seconds.
Causal path:
- `update.install(&bytes)` reaches the plugin's Windows `install_inner`, which extracts the installer, calls `ShellExecuteW`, then `std::process::exit(0)`.
- The plugin runs an `on_before_exit` hook first, but `updater_for` never registers one.
- Everything after the call in `install` is unreachable on Windows: `set_progress(None)`, clearing `PENDING`, and `stored.last_available_version = None; save_state(..)`.
- The ordinary Quit path never runs: no shell session flush (`quit::prepare`), no `windows::capture`, no `save_session_snapshot`, no `Service::shutdown`. The shutdown comment says its last step exists because "The last reading-position refresh may still be coalesced in memory".
- This is the outcome `docs/STATE_MACHINES.md` F-2 rated High and fixed for macOS/Linux ("unsaved sessions, transfers not checkpointed"); the code comment at update.rs:986 only notes "Windows starts its installer and exits inside `install`".
- Visible after-effect: the relaunched new version still has `lastAvailableVersion` set, so the panel shows "You have the latest version." together with "Version {new} was available at the last check." until the next check.
Smallest correct fix:
- In `updater_for`, register `.on_before_exit(..)` (a no-op off Windows). `check()` builds the `Update` kept in `PENDING` through this function, so the hook travels with it.
- In the hook: (a) run `quit::prepare(&app)` on a helper thread with `tauri::async_runtime::block_on`, joined with `FLUSH_TIMEOUT`; (b) call `store.flush_browsing()`; (c) clear `last_available_version`/`last_error` in `update-state.json`.
- Do not call `Service::shutdown` from the hook: it takes `privacy_gate.write()` while `install` holds a privacy read lease and would deadlock.
- Add the Windows path to STATE_MACHINES §1.5.
Regression test: in the `update.rs` tests, extract the hook body as `before_installer_exit(service)` and assert that after it runs, a pending coalesced browsing change is on disk and `load_state_at(..).last_available_version` is `None`.

### BUILD-3 — The only two real-browser UI tests are skipped in every workflow and still report green
Severity: Medium        Category: maintainability
Platforms: CI (all)
Evidence class: reproduced
Location: scripts/test-frontend-surfaces.mjs:9, scripts/test-startup-navigation.mjs:9  [+ .github/workflows/checks.yml:51-52]
Trigger: any PR or push; the "JavaScript tests" step runs `node --test scripts/test-*.mjs`.
Causal path:
- Both tests declare `skip: !process.env.KIWIX_TEST_CHROME && "set KIWIX_TEST_CHROME to a Chromium executable"`.
- No workflow, `package.json` script or doc sets `KIWIX_TEST_CHROME` (grep over `.github`, `package.json`, `docs`, `README.md` returns nothing).
- Running both files here without the variable gave `ok 1 … # SKIP`, `ok 2 … # SKIP`, `# pass 0 # fail 0 # skipped 2`, exit 0.
- The frontend job has already run `npm run build`, which `test-startup-navigation` needs, so only the browser path is missing.
- The 2026-11-25 record ran them only by hand on a Mac.
Smallest correct fix:
- In the `frontend` job's "JavaScript tests" step add `env: KIWIX_TEST_CHROME: /usr/bin/google-chrome`, with a preceding `test -x` guard or a pinned setup-chrome action. I did not verify Chrome's sandbox behaviour on the hosted image.
- In both tests, fail instead of skip when `process.env.CI` is set and the variable is missing.
Regression test: the CI guard is the test; optionally a `checks.yml` step that fails if the test summary reports any skip for these two files.

### BUILD-4 — Rust unit tests never run on pull requests, and Windows-only tests are never executed by any workflow
Severity: Medium        Category: maintainability
Platforms: CI (all); Windows most affected
Evidence class: source-traced
Location: .github/workflows/build.yml:27-29, 82-83, 199-200, 292-365  [+ .github/workflows/checks.yml:3-4, 73-74, 116-117, 152-157, 188, 54]
Trigger: a PR that breaks a Rust test, or any change to `cfg(windows)` logic.
Causal path:
- `checks.yml` is the only PR workflow. For the app crate it runs only `cargo clippy`; the only `cargo test` there is for `kiwix-mcp`.
- The app's `cargo test --lib` exists only in `build.yml`, which has no `pull_request` trigger: Linux runs on push to `main`, macOS only on manual dispatch.
- The Windows job has no clippy or test step at all (checkout → node → toolchain → "Build NSIS installer" → stage/sign).
- So `cfg(windows)` tests are only type-checked through the MinGW clippy job and never run: `location_windows.rs:259-320`, the Windows DACL tests in `private_sync_identity.rs`, `store_tests.rs` `windows_lock_violation_…`, `kiwix-mcp/src/ipc.rs` `pipe_name_is_per_user_and_per_profile`, and 14 `cfg(windows)` sites in `*_tests.rs`.
- Linux and Android clippy on PRs use `--lib` without `--tests`, so non-compiling test code on those targets is also first seen after merge.
- `scripts/test-linux-appimage.py` is a pure in-memory unittest, but its hyphenated name misses `discover -p 'test_*.py'` (checks.yml:54); it only runs in `build.yml`.
Smallest correct fix:
- Add a Linux `cargo test --locked --lib --features custom-protocol` job to `checks.yml`, reusing the `linux-typecheck` apt list plus `xvfb`, and add `--tests` to that job's clippy.
- Add `python3 scripts/test-linux-appimage.py` to the frontend job.
- In the trusted Windows job, before "Build NSIS installer", run the two `cargo test … --target x86_64-pc-windows-msvc` commands already documented in `docs/WINDOWS-BUILD-PROVISIONING.md`.
Regression test: not applicable (workflow change); a one-off PR with a deliberately failing Rust test proves the gate.

### BUILD-5 — Linux and Android location providers are never compiled or type-checked, yet `zbus` is built into every Linux build for them, and DEPENDENCIES.md says the opposite
Severity: Medium        Category: maintainability
Platforms: Linux, Android
Evidence class: source-traced
Location: src-tauri/build.rs:16-26  [+ src-tauri/src/lib.rs:54-59, src-tauri/Cargo.toml:178-181, src-tauri/src/permissions.rs:610-613, docs/DEPENDENCIES.md:96 and 127, .github/workflows/build.yml:103-118]
Trigger: every Linux build; any refactor of `permissions.rs` types (`Position`, `LocationError`, the `start`/`cancel` signatures).
Causal path:
- `build.rs` sets `native_location` only for `Ok("macos" | "ios" | "windows")`, with the comment "Linux (GeoClue) and Android providers are written but not yet compiled".
- `lib.rs` declares `#[cfg(all(target_os = "linux", native_location))] mod location_linux;` and the Android equivalent. So `location_linux.rs` (401 lines) and `location_android.rs` (250 lines) are compiled by no build and no CI job; nothing detects drift.
- `Cargo.toml` still declares `zbus = { version = "5.19", … }` for `cfg(target_os = "linux")` "for location_linux.rs". The lockfile shows only `kiwix-workbench` and the Windows-only `tauri-plugin-single-instance` depend on `zbus`.
- So Linux builds compile `zbus` and its tree for a module that is cfg'd out.
- `docs/DEPENDENCIES.md:96` claims "`cargo tree --target <triple>` shows no zbus on any of the nine shipping targets", and line 127 says zbus "is no longer compiled for any shipping target". Both are false for Linux.
- CI and `PACKAGING.md` still gate releases on a "GeoClue desktop identity" for a feature that is not built.
Smallest correct fix:
- In `build.rs`, also emit the cfg for `linux | android` when an opt-in env var is set (for example `KIWIX_EXPERIMENTAL_LOCATION=1`, with `rerun-if-env-changed`), and set it in the `linux-typecheck` and `android-typecheck` jobs so both files are type-checked without shipping.
- Until the provider ships, make `zbus` an optional dependency behind a cargo feature enabled by the same job.
- Correct DEPENDENCIES.md lines 96 and 127.
Regression test: the two typecheck jobs with the opt-in set.

### BUILD-6 — Confirmed documentation contradictions in the release/updater and developer docs
Severity: Low        Category: maintainability
Platforms: all (docs)
Evidence class: source-traced
Location: docs/PACKAGING.md "Signed updater feed" and "Artifacts, channels, and release acceptance"  [+ docs/UPDATE.md:203, 228; docs/DEV-ITERATION.md:19; docs/PLATFORM-GAPS.md "Docs that don't match the code"]
Trigger: a release manager following PACKAGING.md instead of UPDATE.md.
Causal path (each pair quoted from the current tree):
1. PACKAGING: the script "reads an existing `.sig` or signs with `TAURI_SIGNING_PRIVATE_KEY_PATH`". `scripts/make-update-manifest.mjs:330` and 449 say "Never trust a staged .sig" and "Always sign these newly staged bytes; never reuse any .sig"; a source `.sig` is read only with `--no-sign --public-key` (line 409). The script's own error at line 322 ("or place a .sig file next to every artifact") has the same flaw.
2. PACKAGING: "The checked-in public key is a placeholder". `tauri.conf.json` has `"pubkey": ""`, and UPDATE.md says "no placeholder or generated throwaway key is a release trust root".
3. PACKAGING: "Put the base64 contents of updater.key.pub into plugins.updater.pubkey in src-tauri/tauri.conf.json. Change plugins.updater.endpoints…". UPDATE.md and `stage-update-release.mjs --configure` generate `.updater-release.json` from `KIWIX_UPDATER_PUBLIC_KEY` and derive the endpoint.
4. PACKAGING: "No workflow here creates a release". `build.yml:400` runs `gh release create "v$KIWIX_RELEASE_VERSION" --draft …`.
5. PACKAGING: "packages a `.app` into `Kiwix.app.tar.gz`". The script names it `${target}-${version}-Kiwix.app.tar.gz` (line 401).
6. UPDATE.md:203 and 228 refer to "the final `app.restart()` in the production command". `update.rs:989` says "Never `restart()`/`request_restart()` here" and calls `app.exit(0)`.
7. DEV-ITERATION.md: "`scripts/dev.sh` resolves the patched desktop SDK". `dev.sh` resolves no desktop SDK.
8. PLATFORM-GAPS.md: "README offers a Linux `.flatpak`". README contains no "flatpak".
Smallest correct fix: replace the PACKAGING "Signed updater feed" body with a pointer to UPDATE.md plus the three `stage-update-release.mjs` modes; fix the six other sentences; reword the line-322 error.
Regression test: none practical.

### BUILD-7 — Closing or quitting while a window is minimized on Windows overwrites its saved geometry; it reopens at 760×520, centred
Severity: Low        Category: usability
Platforms: Windows
Evidence class: source-traced (Kiwix and tao code read; the Win32 minimized-window rectangle is documented OS behaviour, not executed here)
Location: src-tauri/src/windows.rs:461-478 (`capture`)  [+ windows.rs:421-452 (`restore`), src-tauri/src/lib.rs:1241-1245, src-tauri/src/quit.rs:306, tao-0.37.2/src/platform_impl/windows/window.rs:177-181 and 219-225]
Trigger: minimize Kiwix, then close it from the taskbar (or close another Kiwix window, or Quit) while it is still minimized; relaunch.
Causal path:
- `capture` runs for every window on any `CloseRequested` and in `quit::prepare`. It stores `outer_position()` and `inner_size()` unconditionally.
- tao returns raw `GetWindowRect`/`GetClientRect`; Windows parks a minimized window at (-32000, -32000) with an icon-sized or empty client area.
- `restore` then applies `width.clamp(760.0, 6000.0)` and `height.clamp(520.0, 4000.0)`, finds no intersecting monitor and calls `window.center()`. The user's size and position are lost.
- With several windows, a minimized one is overwritten each time any other closes.
Smallest correct fix: in `capture`, `if window.is_minimized().unwrap_or(false) { continue; }` so the last good geometry stays; optionally keep the previous x/y/size when `is_maximized()` and update only the flags. The stored schema is unchanged.
Regression test: split a pure `captured_geometry(position, size, scale, minimized, maximized) -> Option<Value>` out of `capture` and test it in `windows.rs`; assert `None` when minimized, and that `restore`'s clamp is not fed a zero size.

### BUILD-8 — Busy-profile dialog on Linux and Windows tells the user to use "Activity Monitor"
Severity: Low        Category: usability
Platforms: Linux, Windows
Evidence class: source-traced
Location: src-tauri/src/instance.rs:101-114 (`busy_dialog_message`)  [+ instance.rs:156-171, src-tauri/src/lib.rs:508-530, lib.rs:953-966]
Trigger: on Linux, Kiwix is running with a read-only profile (which publishes no rendezvous by design) and the user double-clicks a `.zim`; also any profile whose socket path exceeds 100 bytes. On Windows, any `StoreOpenError::Busy` reaching `profile_busy_in_setup`.
Causal path: the message is not platform-gated: "If no Kiwix window is visible, quit the running instance from its window or Activity Monitor; do not delete store.lock." It is shown through zenity/kdialog on Linux and the Tauri dialog on Windows. It is also English-only regardless of the saved locale.
Smallest correct fix: pick the tool name with `cfg!`: "Activity Monitor" on macOS, "Task Manager" on Windows, "your system monitor" elsewhere. Keep the rest unchanged; the existing test at instance.rs:537 compares against the same function.
Regression test: instance.rs tests — assert the non-macOS message does not contain "Activity Monitor" and still contains the profile path and "do not delete store.lock".

### BUILD-9 — A `lastChecked` timestamp in the future suppresses automatic update checks until that time passes
Severity: Low        Category: reliability
Platforms: Windows, Linux, macOS
Evidence class: source-traced
Location: src-tauri/src/update.rs:370-379 (`check_due`)  [+ update.rs:381-384 (`next_check_at`), 729-731]
Trigger: a check runs while the clock is ahead (dual-boot RTC local/UTC mismatch, manual mis-set, VM resume) and the clock is later corrected.
Causal path: `(now - checked).whole_seconds() >= CHECK_INTERVAL_SECONDS` is negative for `checked > now`, so `automatic_check` returns at line 730 on every hourly tick. Settings shows `nextCheckAt` at the bogus future time plus 24 h. A manual check repairs it; no test covers a future timestamp.
Smallest correct fix: in `check_due`, treat `checked > now` as due. Apply the same guard in `next_check_at` so Settings does not show a far-future time.
Regression test: update.rs tests — `assert!(check_due(Some(&format(now + 2 h)), now))`.

### BUILD-10 — `docs/ERRORS.md` is a generated inventory that is stale, and nothing checks it
Severity: Low        Category: maintainability
Platforms: docs/CI
Evidence class: source-traced
Location: docs/ERRORS.md (header "Generated from current sources by `node scripts/generate-error-docs.mjs`")  [+ scripts/generate-error-docs.mjs:73, package.json:25]
Trigger: any reader using the file to find an error string's source.
Causal path:
- The file cites `src-tauri/src/storage.rs:413` for the FAT/FAT32 message; that string is now at storage.rs:495 (line 413 is a `use`).
- It cites `update.rs:358 | KIWIX_UPDATER_PUBKEY`; that is now line 412.
- "The update download was cancelled." (update.rs:775, added 2026-11-25) is absent.
- The last commit touching the file is 2026-11-19 (`35e220b`).
- The generator only writes; there is no `--check` mode and no workflow calls it.
Smallest correct fix: add `--check` to `generate-error-docs.mjs` (build the document, compare with the file, exit 1 on difference), run it in the `checks.yml` frontend job, and regenerate once. If per-line churn is unwanted, drop the `:line` suffix from the Source column.
Regression test: the `--check` CI step.

### BUILD-11 — Floating Rust toolchain and mutable third-party action refs, including in the jobs that receive signing secrets
Severity: Low        Category: maintainability
Platforms: CI/release
Evidence class: source-traced
Location: .github/workflows/build.yml:58, 176, 310  [+ checks.yml:70, 97, 133, 171; dependencies.yml:79, 86; no `rust-toolchain.toml`; src-tauri/Cargo.toml `rust-version = "1.90"`]
Trigger: a new stable Rust release, or a moved `stable` branch / `v2` tag of a third-party action.
Causal path:
- Every Rust job uses `dtolnay/rust-toolchain@stable` and gates on `clippy … -D warnings`, so each stable release can turn unrelated PRs red; commit `3a2f603` ("Satisfy clippy arity, range, and multiple-of lints on Linux CI") is an example of clippy lint repairs landing on main.
- Release artifacts are built with whatever stable is current, so they cannot be rebuilt identically.
- The `desktop-release` jobs execute that third-party branch ref in the same job that later exposes `TAURI_SIGNING_PRIVATE_KEY` and the Apple certificate. A step that altered PATH or cargo earlier in the job could read them.
- `EmbarkStudios/cargo-deny-action@v2` is likewise a mutable tag.
Smallest correct fix: add a root `rust-toolchain.toml` pinning an exact channel with the needed components and targets, bumped deliberately. Pin `dtolnay/rust-toolchain` and `cargo-deny-action` to commit SHAs; first-party `actions/*@v4` can stay.
Regression test: not applicable.

### BUILD-12 — Windows `state.json` replacement has no retry for transient ACCESS_DENIED/SHARING_VIOLATION, although the helper exists
Severity: Low        Category: reliability
Platforms: Windows
Evidence class: inference (code path traced; the third-party handle that triggers it was not reproduced)
Location: src-tauri/src/store.rs:3551-3555 (`replace_file`, Windows)  [+ src-tauri/src/storage.rs:410-436 (`move_file`), storage.rs:438-455 (`retry_transient_sharing`), src-tauri/src/update.rs:345-358]
Trigger: another process (third-party antivirus, backup or sync client, an editor) briefly holds `state.json` or the temporary file open without delete sharing while Kiwix commits a transaction.
Causal path:
- Every store transaction ends in `persist` → `replace_file` → one `MoveFileExW(REPLACE_EXISTING | WRITE_THROUGH)`.
- A failure maps to `PersistError { committed: false }`, so the user's action (settings save, bookmark, adding a finished download) is rejected with "Cannot save …state.json: Access is denied… The previous state was retained."
- `storage.rs:438-443` already documents that "a third party (antivirus scanning just-written files, the search indexer, a sync client) often holds one for a moment" and provides `retry_transient_sharing` (5 attempts, about 1.5 s). It is used only at `publication.rs:451` and `localswarm/storage.rs:480`, not for `state.json` or `persist_session`, the files rewritten most often.
- Related: `update.rs:351-355` has a fallback commented "Windows refuses to replace an existing destination". `std::fs::rename` already replaces on Windows, so the branch is dead; if ever taken it would remove then rename, which is not atomic.
Smallest correct fix: in the Windows `replace_file`, wrap the `move_file` call in `crate::storage::retry_transient_sharing`. Make `update::save_state_at` call `store::replace_file` and delete the remove-then-rename branch.
Regression test: `store_tests.rs` (`cfg(windows)`) — hold `state.json` open with share mode read|write (no delete) on a thread that releases after 200 ms, then assert `store.update(..)` succeeds; assert it still fails once the handle outlives the retry budget.

**Covered** (read end to end unless noted)
- CI and manifests: `checks.yml`, `build.yml`, `dependencies.yml`, `package.json`, `src-tauri/Cargo.toml`, `build.rs`, `tauri.conf.json` and the linux/windows/macos overlays, `capabilities/main.json`, `windows-app-manifest.xml`, `windows/installer-hooks.nsh`, `linux/AppRun`, `linux/main.desktop`, `.gitattributes`, `.gitignore`, `bacon.toml`, `vite.config.ts`, `tsconfig.json`.
- Rust: `update.rs` (all production code, tests partly), `update_admission.rs`, `agent_connector.rs`, `kiwix-mcp/src/main.rs` and `ipc.rs`, `protocol.rs` (first 260 lines), `win-security/src/lib.rs`, `location_windows.rs`, `native_pdf/windows.rs`, `native_pdf/linux.rs`, `windows.rs`, `instance.rs` (to line 385), `instance/socket.rs`.
- Rust, partial: `lib.rs` startup, quit and single-instance paths (140-240, 300-330, 470-530, 800-1150, 1215-1330, 1399-1405); `store.rs` open/persist/startup-hint paths; `storage.rs` `move_file` and retry; the Windows ACL call sites in `private_sync_identity.rs`, `kokoro/install.rs`, `spelling.rs`.
- Scripts: `tauri-cli.mjs`, `build-mcp.mjs`, `windows-bundle-resources.mjs`, `stage-update-release.mjs`, `linux-appimage.py`, `test-linux-appimage.py`, `dev.sh`, `fuzz.sh`, `check-dependency-audit.sh`, `test-native-sanitizers.sh`, `check-windows-build.ps1`, `provision-windows.ps1`, `generate-error-docs.mjs`; `check-unsafe-census.mjs` (run: passes, 74 blocks); `test-tauri-cli`, `test-update-manifest`, `test-archive-packaging` (run: 23 pass, 1 platform skip).
- Docs: `UPDATE.md`, `AGENT-CONNECTOR.md`, `PLATFORM-GAPS.md`, `DEV-ITERATION.md`, `WINDOWS-BUILD-PROVISIONING.md`, `PACKAGING.md` (lines 1-20 and 678-871), README download/build sections, `docs/reviews/2026-11-25` in full, `2026-11-23` and `STATE_MACHINES.md` by targeted reads. An automated pass over the listed docs found no reference to a source file that no longer exists.
- Nothing met the bar in: connector lifecycle and transport (enable/disable, stale socket, peer checks, timeouts, error text), `win-security`, `location_windows.rs`, the NSIS hooks, the release staging script, the AppImage assembler.
- Not re-reported, already a documented residual: Windows ACL checks fail closed with a message (not at startup) on FAT/exFAT; the automatic update check holds the privacy read lease for up to its 60 s timeout.

**Not covered**
- `native_pdf/windows_print.rs`; the body of `location_linux.rs` (header only); `kiwix-mcp/src/protocol.rs` after line 260; the `update.rs` test module.
- `scripts/make-update-manifest.mjs` (grep and targeted reads only), `update-e2e.mjs`, `bundle-macos.mjs`, `build-desktop-sdk.py`, `localization.mjs`, `error-inventory.mjs`, `check-archive-libraries.mjs`, `test-linux-appimage-runtime.py`; `upstream/appimage/`; `src-tauri/deny.toml`.
- `docs/FEATURES.md`, `ARCHITECTURE.md`, `PLATFORM_COMPARISON.md`, `DEPENDENCIES.md` (grep and spot checks only); the mobile sections of `PACKAGING.md`; `docs/ERRORS.md` beyond three spot checks.
- `cfg(windows)` and `cfg(target_os = "linux")` branches in `acquisition.rs`, `publication.rs`, `library_files.rs`, `localswarm/storage.rs`, `fs_ext.rs`, `archive.rs`, `torrent/publication.rs`, `service.rs`, `service/dispatch.rs`, `portable.rs` (only `detect`/`distribution_root` read).
- Long-path, UNC and reserved-name handling, WebView2-missing first launch, DPI/high-contrast, Wayland vs X11 and Flatpak/Snap behaviour were not traced far enough to support a finding.
- The Linux Save-as-PDF path fails if GTK printer enumeration takes longer than 500 ms (`native_pdf/linux.rs`); I could not check GTK's CUPS backend timing offline, so it is not reported.