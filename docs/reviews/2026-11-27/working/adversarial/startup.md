## Verdict for lane startup: only non-blocking findings

Reviewed base `d98cac2`. I read the full diff of `src-tauri/src/{lib,quit,instance,crash}.rs`, `scripts/unsafe-census-baseline.json` and `docs/STATE_MACHINES.md` §1.1. The diff did not change while I worked (two snapshots compared byte for byte). I also read every caller of the changed functions; `Service::shutdown` (`service.rs:3517-3652`); `update.rs` RESTART_AFTER_QUIT; the frontend quit and startup-notice handlers (`App.tsx:2070`, `2700-2776`, `3720`); the plugin source `tauri-plugin-dialog-2.8.4/src/{desktop,lib}.rs`; the rfd 0.17.2 backends (GTK, Windows TaskDialog, macOS NSAlert and CFUserNotification); and tao 0.37.2 session-end handling.

### STARTUP-R1 — The failed-Quit dialog can leave a running Kiwix with no window, holding the profile, after Quit requests were silently ignored
Severity: Should-fix
Fix under review: CORE-1
Evidence class: source-traced
Location: `src-tauri/src/lib.rs:1566-1568` (ExitRequested gate); `lib.rs` `ask_after_failed_quit`, `FailedQuitAction::Stay` arm; `lib.rs:561-570` (`activate_instance`)
Trigger, flush case (Windows or Linux):
1. The profile volume becomes unwritable, and the person chooses Quit.
2. The flush fails, so the shells resume and the Retry / Quit Without Saving / Cancel dialog opens with no parent window.
3. The person closes the Kiwix window (or picks Quit again) while the dialog is still open.
4. The person then presses Cancel or Escape.
Causal path:
- `prepare` cancelled and resumed the shells, and `on_window_event` does not intercept CloseRequested, so the last window is destroyed.
- Tauri raises ExitRequested. `api.prevent_exit()` runs, then `if !exiting.swap(true, …)` finds `exiting` already true, so the request is dropped with nothing shown.
- Cancel maps to `Stay { clear_relaunch: true }`, which runs `exiting.store(false, …)` and nothing else.
- The process now has no window and no tray. It still holds `store.lock` and the service.
- A second launch reaches `activate_instance`. There, `desktop_launch_target` returns "No native window is available…", so nothing comes forward. On Unix the person then gets the busy-profile dialog. The only way out is Task Manager or a system monitor; on macOS the Dock still works.
How this differs from base:
- In base, a failed flush reset `exiting` at once, so closing the window started a new attempt.
- The retirement-failure variant (close the last window → shutdown fails → Cancel) also existed in base, through dismissing the old Retry-only dialog. The new explicit Cancel button now invites it.
- Related: while the dialog is open, Quit from the menu silently does nothing. An unparented TaskDialog or GTK dialog can sit behind the Kiwix window.
Smallest correct fix:
- In the `Stay` arm, if no window with `quit::shell_label` remains, do not stay. Call `ask_after_failed_quit` again, or take the Retry path (`exiting=false; app.exit(code)`), so the person must choose Retry or Quit Without Saving.
- Optionally, also record an ExitRequested that arrives while the dialog is open and re-request exit after Stay.
- Consider `.parent(&main_window)` when a shell window exists.

### STARTUP-R2 — The forced-quit regression test cannot fail on the claim it makes
Severity: Nit
Fix under review: CORE-1
Evidence class: source-traced
Location: `lib.rs` `launch_tests::quitting_without_saving_still_retires_the_service_when_the_profile_is_gone`
Causal path:
- The test calls `service.shutdown()` twice before `retire_without_saving`. The first call already sets `stopping=true` (`service.rs:3519`).
- So `assert!(service.ensure_running().is_err())` and the refused `snapshot` both pass even if `retire_without_saving` were an empty function.
- Nothing in the test shows that the forced path re-runs the retirement stages, which is the implementer's "every stage runs" claim. That claim does hold by code reading: on desktop `shutdown` has no early return; the only `?` is behind `cfg(target_os = "ios")`.
Smallest correct fix: assert an effect only the forced call can produce. Either keep one resource live until the forced call (for example a started sharing server or listener, then check it is released), or drop the claim from the test comment.

### STARTUP-R3 — A collision between the Retry and Cancel labels also disables Quit Without Saving
Severity: Nit (latent)
Fix under review: CORE-1
Evidence class: source-traced
Location: `quit.rs` `failed_quit_action` (`distinct` requires all three labels to differ)
Trigger: a translation where "Retry" and "Cancel" render the same.
Causal path:
- With `distinct == false`, the explicit Quit Without Saving button also maps to `Stay`.
- That brings back CORE-1 (no way to quit) for that locale, although the destructive button would still be unambiguous.
- No locale collides today. I checked all 74 `src/locales/messages/*.json`: no collisions, and none translates "Quit Without Saving" yet.
Smallest correct fix:
- Require only `quit_without_saving != retry && quit_without_saving != cancel` for the destructive mapping.
- Map Retry only when `retry != cancel`.

### STARTUP-R4 — The unsafe documentation is now stale
Severity: Nit (documentation claim)
Fix under review: CORE-2
Evidence class: source-traced
Location:
- `docs/RUST-UNSAFE-REVIEW.md:446` still says "74 blocks in 19 code files", and about line 437 says "one prose `unsafe {}` in `lib.rs`".
- `docs/MEMORY-SAFETY-INVENTORY.md` (B8/B10) does not list the new `MessageBoxW` site.
Cause: `node scripts/check-unsafe-census.mjs` now reports 75/75/2 in 20 files. The implementer handed this off, but the docs are wrong as of this change.
Smallest correct fix: update both documents in the same batch.

### STARTUP-R5 — Remaining redaction gaps: one introduced by the new file-URL branch, the rest pre-existing
Severity: Nit
Fix under review: CORE-8
Evidence class: reproduced. I copied the current `crash.rs:405-720` into a scratch crate (`.../adv-startup/redact`) and ran it.
Location: `crash.rs` `replace_urls` (the `tail.starts_with('/')` gate) and `path_at` (start and boundary rules)
Results:
- `open file:C:/Users/Jane Doe/a.zim now` becomes `open [url] Doe/a.zim now`. The new path-end rule is used only when the tail starts with `/`.
- Pre-existing detection gaps, unchanged by this lane:
  - `open //nas/Family Share/a.zim failed` is not redacted at all (a forward-slash UNC path).
  - `failed->/home/jane doe/x.zim` and `path|/home/…` are not redacted, because `>` and `|` are not boundaries.
  - `~\Documents\…` is not redacted.
- Over-redaction (accepted by design): `invalid route /A/Main_Page for book 12 and the len is 3…` becomes `invalid route [path]`.
- Correct:
  - quoted, JSON and Debug-escaped forms;
  - `\\?\C:\…: Access is denied. (os error 5)`, which becomes `[path]: Access is denied. (os error 5)`;
  - tabs;
  - mixed `file:///…` URLs.
Smallest correct fix: use `path_end` for any `file:` tail (not only `/`-prefixed ones), and start a path at `//` followed by a non-slash. The other gaps are optional.

### STARTUP-R6 — Dismissing zenity with Escape or the close button opens a second kdialog copy
Severity: Nit
Fix under review: CORE-2 and BUILD-8
Evidence class: inference. Zenity's exit-code semantics are from memory; nothing was run here.
Location: `instance.rs` `show_native_message` (Linux): `status.success()`
Causal path:
- Zenity exits 1 when its dialog is closed or cancelled.
- That counts as "not shown", so kdialog is tried next. On a desktop with both installed, the same message appears twice.
- The pattern predates this lane (busy dialog); it is now also on the startup-error path.
Smallest correct fix: treat any exit status from a successfully spawned helper as shown, except an option-parse failure (for example 255).

### STARTUP-R7 — `open_paths` contract wording; some per-file failures are dropped
Severity: Nit
Fix under review: CORE-7
Evidence class: source-traced
Location: `lib.rs` `open_paths`, around lines 963-975
Causal path:
- When `queue_launch_locations` or `window.emit(...)` fails after a successful import, the function returns `Err`, and the `failures` list is discarded.
- Books may already be registered in the library, so the doc's "`Err` means nothing was opened" is inaccurate.
- An error is still shown, so nothing reports false success.
Smallest correct fix: correct the comment. Optionally append `failures` to the `Err` text.

**Per-fix assessment**
- **CORE-1: fixes it but …** see R1 (the Cancel path with no window) and R2.
  - I checked every backend: a closed or failed dialog arrives as the Cancel label, never the "No" slot.
    - GTK (the plugin's default `gtk3` feature): Escape or close gives `DELETE_EVENT` → `Cancel` → the plugin maps it to `Custom(cancel)`.
    - Windows (`common-controls-v6`, `TDF_ALLOW_DIALOG_CANCELLATION`): Escape or Alt+F4 gives `IDCANCEL` → `Cancel` → `Custom(cancel)`; a failed `TaskDialogIndirect` also gives `Cancel`.
    - macOS without a parent uses CFUserNotification: it has no close button, and a display failure or cancel response gives `Cancel`.
  - "No" is produced only by the No-slot button: `GTK_RESPONSE_NO`, `ID_CUSTOM_NO`, or `kCFUserNotificationAlternateResponse`.
  - The XDG-portal backend is not compiled (the plugin enables only `gtk3`).
  - Unverified: which button the macOS UserNotificationCenter alert binds to Escape.
  - Other checks:
    - Relaunch branches match the new §1.1.
    - The updater's `app.exit(0)` during the dialog is absorbed safely.
    - Windows logoff (`WM_ENDSESSION` → `exit(0)` in tao) and SIGTERM are unchanged from base.
  - A hang inside `shutdown` still leaves the app needing a force-kill. That is the same as base, not worse.
- **CORE-2 / BUILD-1: fixes it.**
  - Every `Err` from the setup closure is wrapped. The `MessageBoxW` call uses owned `HSTRING`s that live past the call, a `None` owner, and runs on the main thread from `run()`. The `// SAFETY:` comment is accurate.
  - AppleScript quoting escapes `\` and `"` and turns newlines into `\n`, which is safe.
  - Zenity and kdialog get the text as separate argument values, after a fixed leading sentence.
  - No display means no Linux dialog; `--headless` means no dialog (except the pre-existing busy-profile path in headless, `lib.rs:1196`, which can still show zenity).
  - After a setup failure the process exits from the dialog callback without `Service::shutdown`. That is equal to the panic it replaces.
  - Windows `--help`/`--version` print as before when stdout is redirected.
  - See R4 (docs) and R6.
- **CORE-7 / LIB-4: fixes it.**
  - All callers report failures:
    - the Unix rendezvous;
    - the Windows plugin callback (`activate_instance` still focuses first, as before);
    - the pending-launch drain;
    - startup, through the notice plus a focus only when something failed;
    - headless;
    - macOS `Opened`, which I read character by character: `&service` deref-coerces from `Arc`, and `app` is the cfg-gated clone.
  - File names in the dialog are plain text on every backend. Spoofing text through a crafted name is a local-only concern.
  - See R7.
- **CORE-8: fixes it** for the reported space-containing shapes. The existing crash tests still assert their originals (two cases split out). See R5.
- **BUILD-8: fixes it.** The strings are correct for each platform.

**Implementer's doubts**
- Windows non-interactive `MessageBoxW`: acceptable; `--headless` is the documented exemption.
- Dialog shown even when launched from a terminal: acceptable, for the stated reason (VT-started sessions).
- Partial setup failure exits without `shutdown`: acceptable; equal to base.
- Hang on forced quit: acceptable for this lane; not worse than base. The real fix belongs to CORE-6.
- Cancel after a failed retirement: real problem when no window remains. See R1; fix now.
- Redaction residue: acceptable. Additional residue is listed in R5.
- Partial-success heading and mixed-language buttons: acceptable for now; the owner decides on new strings.
- `--no-markup`: acceptable; the fallback is safe.
- Wayland default socket: acceptable.
- Double reporting (banner plus dialog): acceptable.

**What I ran**
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- launch_tests quit:: crash:: instance::` → `test result: ok. 29 passed; 0 failed; … 1422 filtered out`.
- Host clippy `--lib --tests --features custom-protocol -- -D warnings` → `Finished`, exit 0.
- `node scripts/check-unsafe-census.mjs` → `Unsafe census: 75 matches / 75 blocks / 2 impls in 20 files; every item documented.`
- Windows-gnu clippy (the brief's command), run twice, exit 101 both times. Every error was in other lanes' files, none in this lane's:
  - Run 1:
    - `src/update.rs:1101` E0433 `semver` (twice);
    - `src/localswarm/mod.rs:24` unused import `reachable_addresses`;
    - `src/update.rs:662` E0382 moved `current_version`.
  - Run 2:
    - `src/localswarm/mod.rs:24` (same);
    - `src/update.rs:662` (same);
    - `src/store.rs:3074` E0425 `state_bytes` plus E0277 `[u8]` size;
    - (lib: 4 errors; lib test: 2).
  - So lane lints were not fully exercised on Windows. Type-check evidence for the `cfg(windows)` code is partial.
- Scratch redaction probe (R5) and a label-collision scan over 74 locale catalogs.

**What I could not verify**
- No native dialog was shown on any OS. This covers GTK and zenity/kdialog behaviour, Windows `MessageBoxW` and TaskDialog, and macOS osascript and CFUserNotification (including Escape mapping and button layout).
- The macOS code is uncompiled.
- The live Tauri wiring is unexercised: the setup wrapper, the ExitRequested and dialog interplay, relaunch, and window focus.
- No clean Windows-gnu clippy run was obtained.