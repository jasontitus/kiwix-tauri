## Verdict for lane desktop, round 2: only non-blocking findings

R1, R2, R4, R5 and R6 are closed. Item 7 (the relaunch wording) is closed apart from one imprecise sentence, and R8 is closed for the implementer's parts. I found two new Nits and one wording Nit. The stale `service.rs` / `service_tests.rs` comments were left alone on purpose, so they remain yours.

### Comparator (R2): it cannot hide a newer update
I copied `still_available` and `version_core` into a scratch binary (`adv-desktop/cmp.rs`) and ran edge cases. Results:

| Recorded | Running | Outcome |
|---|---|---|
| `v0.3.0` | `0.2.0` | shown |
| `0.3` | `0.2.0` | shown |
| ` 0.3.0` | `0.2.0` | shown |
| `0.3.0-beta.1` (pre-release of a newer version) | `0.2.0` | shown |
| `0.3.0-beta.1` | `0.3.0-alpha.1` | shown |
| `1.0.0` | `1.0.0-rc.1` | shown |
| `0.2.0` | `0.2.0.1` | shown |
| `0.2.0-` | `0.2.0` | shown |
| `0.3.0` | `v0.2.0` | shown |
| `0.2.0+b2` | `0.2.0+b1` | hidden |
| `0.2.0-rc.1` | `0.2.0-rc.1+x` | hidden |

A recorded version is hidden in only three cases:
- its core is below the running version;
- it is the same string apart from build metadata;
- it is a pre-release of the same core while the matching release runs.

Anything that does not parse is shown, so the comparator errs towards showing. `last_available_version` is used only in `status` and the Settings panel, so no other consumer bypasses the filter. The only cosmetic leftover is that a running `rc.2` still mentions a recorded `rc.1`.

### R1: hook ordering against tauri-plugin-updater 2.14.1 — closed
- `install_inner` runs `extract`, then the hook, then `ShellExecuteW`, then `exit(0)`.
- The closure first runs `run_before_installer_exit`, which waits for the flush to finish or for the 5 s timeout. It then calls `app.cleanup_before_exit()` on the calling thread, also when the `Weak` upgrade fails.
- After a timeout, the flush thread may still be querying windows while cleanup hides them and clears the resource tables. Neither touches the store, so this is harmless.
- The `AppHandle` captured inside the static `PENDING` is the same thing the plugin's default hook held.
- If `ShellExecuteW` fails, the windows have already been hidden. That is also true of the plugin's default hook, and §1.5 says so.

### R5: the read-only exit block in `update_inner` — closed
- **Lock order:** the copy is made under the store's `inner` mutex with plain file I/O only, as `persist` already is. It takes no other lock and does not involve the sandbox.
- **Copy kept, commit fails:** the copy is fsynced before anything else, so the records survive. This holds if `persist` fails (committed or not), and if `mobile_complete_relocation_locked` or portable `prepare`/`initialize` fails with `?` afterwards. The flag is cleared, but the copy exists and was taken from the bytes before relocation.
- **Commit without a copy:** this cannot happen. A copy failure, or a `state_bytes` error, returns before relocation, portable setup and `persist`, so the profile stays read-only and unchanged.
- **Other ways to write:** every other path that writes `state.json` refuses while read-only (`flush_browsing`, `commit_integrity`, `set_runtime_mode`'s pre-private flush), so the first write really goes through this block.
- **Runtime read-only:** a CLI `--read-only` profile never relaxes it (`from_store` passes the same flag), so the flag stays harmlessly unused.
- The tests use a thread-local failure injection, which is safe with parallel tests. Both tests assert the user-visible outcome.

### R4: version gate — closed
`reset_saved_windows` calls `foreign_version_error` after taking the lock and parsing the file, and before any edit. Missing or non-numeric versions behave as before. The test asserts the error and that the bytes are unchanged.

### R6: `cfg!(windows)` — closed, with a Nit (DESKTOP-R10)
On Linux the test now covers the "keep the moved frame" branch.

### Item 7: relaunch comments against the current `lib.rs` / `quit.rs` — accurate
- `take_restart_after_quit` is consumed only in `finish_authorized_quit`.
- That runs after a successful retirement and after "Quit Without Saving".
- Retry keeps the relaunch.
- `clear_restart_after_quit` is called only from `Stay { clear_relaunch }`, which is `failure == Flush`.

One imprecision is DESKTOP-R12 below.

### DESKTOP-R10 — The Windows minimized skip (the core of BUILD-7) is now exercised by no test
- **Severity:** Nit
- **Evidence:** source-traced
- **Location:** `windows.rs:516` (`captured_geometry`) and its test at `:623`
- **Cause:** the parked-window assertions run only under `cfg!(windows)`, and `checks.yml` runs the Rust tests only on ubuntu and macOS.
- **Smallest fix:** pass a `windows: bool` argument (as `platform_accelerator` does), call it with `cfg!(windows)`, and test both values on every host.

### DESKTOP-R11 — After leaving read-only, the startup band still says "Back up that file before you leave read-only mode"
- **Severity:** Nit
- **Evidence:** source-traced
- **Location:** `store.rs:3073-3080`; `Service.startup_warnings`
- **Cause:** the copy is now kept, but `load_notice` and the band are never updated, so the person is not told where the copy is.
- **Smallest fix:** after a successful preserve, add the "copy kept at …" notice with `note_startup_warning` (or return it to the caller).

### DESKTOP-R12 — "Cleared only when the person answers Cancel or closes the dialog after a failed session flush" is not quite right
- **Severity:** Nit (wording)
- **Evidence:** source-traced
- **Location:** `update.rs` `RESTART_AFTER_QUIT` doc comment; STATE_MACHINES §1.5
- **Cause:** `quit::settle_failed_quit` turns that Cancel into Retry when no shell window remains or a Quit arrived while the dialog was open. In that case the relaunch is kept.
- **Smallest fix:** add "…unless no window is left or another Quit was requested meanwhile (then it is a Retry)".

### What I ran
- `cargo test --locked --lib -- store:: update:: windows::` → `test result: ok. 109 passed; 0 failed; 1 ignored`.
- The comparator scratch binary, built with rustc, with the results above.
- I did not rerun clippy for this round.

### What I could not verify
- Real Windows behaviour: `cleanup_before_exit` hiding windows before `ShellExecuteW`, and the minimized rectangles.
- Mobile relocation at runtime.
