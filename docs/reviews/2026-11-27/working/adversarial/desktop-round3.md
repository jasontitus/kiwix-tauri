## Verdict for lane desktop, round 3: closed (apart from the R11 service-side handoff, as you said)

I found no new problems. R10, R12 and the store side of R11 are all fixed. I only read the code and ran the tests; I did not edit anything.

**R10 — closed.** `captured_geometry(observed, previous, windows: bool)` skips a minimized window only when `windows` is true. Both callers in `persist_capture` pass `cfg!(windows)`. The test now runs on every host with both values: a parked or moved minimized window is dropped with `true`, a moved one is saved at x = 300 with `false`, and the empty, non-finite and ordinary frames are checked for both values. So the actual BUILD-7 fix for Windows is now covered on the Linux and macOS CI runners.

**R11, store side — correct for the planned consumer.**
- `Inner.notice` starts as the load notice in both constructors (`store.rs:2196`, `:2260`).
- When leaving read-only keeps the copy, `update_inner` replaces it with `kept_copy_notice(&kept)` (`:3101`). That is the same text a writable launch shows.
- The notice is changed only after `preserve_skipped_original` succeeds. If a later step fails (relocation, portable setup, `persist`), the notice still truthfully names a copy that exists on disk. If the copy itself fails, the notice still asks for a backup, and the tests check this.
- `load_notice()` keeps the notice as it was at opening, so `from_store` keeps compiling until the service lane switches.
- `current_load_notice()` takes the store lock briefly and returns a clone. `Store::read()` also returns a clone and holds no guard, so the snapshot handler can call it without risking a re-entrant lock.
- When the service lane makes the change, it should also remove the `#[cfg_attr(not(test), allow(dead_code))]` on `current_load_notice`.

**R12 — closed.**
- The `update.rs:315` doc comment and STATE_MACHINES §1.5 now include the `settle_failed_quit` exception (no window left, or another Quit requested meanwhile, turns Cancel into Retry).
- The §1.1 table rows 31–33 and the invariant at line 43 say the same thing.
- This matches the current `lib.rs` / `quit.rs`.

**Ran:** `cargo test --locked --lib -- store:: update:: windows::` → `test result: ok. 109 passed; 0 failed; 1 ignored`. I did not rerun clippy for this round; the implementer reports host and Windows-gnu clippy clean.

**Remaining items for the lane:**
- **Service lane, R11:** in `from_store`, drop the `load_notice()` extension, and build the snapshot's `startupWarnings` from `startup_warnings` plus `current_load_notice()`. The frontend also needs to reset `startupNoticeDismissed` when the warning text changes.
- **Earlier handoffs still open:**
  - **Library availability pass (R3):** call `crate::sandbox::revive()` in `dispatch.rs` `library_maintenance`. APPLE-2 is not closed until this is done.
  - **Generated files (R7):** regenerate the localization manifest and `docs/ERRORS.md`.
  - **Stale comments (R8):** fix `service.rs:760-762` and `service_tests.rs:5026`.
- **Owner decisions:**
  - Preserved copies are never cleaned up and keep reading history (R9).
  - Shell changes made in the 600 ms before the Windows installer hook are not saved.
  - Whether to keep ⌥⌘T for Table of contents.
  - The comparator does not order two pre-releases of the same version, so a running rc.2 can still mention a recorded rc.1. This is cosmetic only.
- **Never compiled or run here:** all `cfg(target_os = "macos")` code (menus and `sandbox::revive`), the real Windows installer hook, `cleanup_before_exit` and the minimized rectangles, the mobile runtime, and the App Sandbox `lstat` behaviour that `present()` relies on.
