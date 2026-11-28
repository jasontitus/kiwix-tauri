Lane `desktop` round 3: all three nits are done. R10 and R12 are fully closed. R11's store side is done, but the startup band only shows the new notice once the service lane applies a two-line handoff (below). Tests: 109 passed. Host clippy and Windows-gnu clippy are clean. Both changed tests fail against a mutated copy.

### DESKTOP-R10 — The Windows minimized skip was exercised by no test
Disposition: fixed.
Change:
- `windows.rs` `captured_geometry(observed, previous, windows: bool)` skips a minimized window only when `windows` is true. The zero-size and non-finite guards still apply to both values.
- Both callers in `persist_capture` pass `cfg!(windows)`.
Tests:
- `windows::tests::a_minimized_window_keeps_its_last_saved_frame` now runs on every host with both values:
  - parked frame with `true` → `None`;
  - moved minimized frame with `true` → `None`;
  - moved minimized frame with `false` → saved at x = 300;
  - empty, non-finite and ordinary frames under both values.
- The maximized test passes `cfg!(windows)`.
- It passes, and fails in a mutated copy with the skip disabled.

### DESKTOP-R11 — The startup band kept asking for a backup after the copy was kept
Disposition: store side fixed; service side is a handoff, since the band is copied once in `service.rs` and rendered from `dispatch.rs`.
Change (`store.rs`):
- New `Inner.notice: Option<String>`. It starts as the load notice.
- When leaving read-only keeps the copy, `update_inner` replaces it with `kept_copy_notice(&kept)`. That is the same text a writable launch shows, now factored out of `skipped_records_notice`.
- If the copy cannot be kept, the notice is left unchanged.
- New accessor `Store::current_load_notice() -> Result<Option<String>, String>`. `load_notice()` (the notice as of opening) is unchanged, so `service.rs` still compiles.
- `current_load_notice` carries `#[cfg_attr(not(test), allow(dead_code))]` with a comment naming this handoff. Remove that attribute once the service lane uses it.
**Handoff (service lane), exact edits:**
1. `service.rs` `from_store`: delete `startup_warnings.extend(store.load_notice().map(str::to_owned));`.
2. `service/dispatch.rs` `"snapshot"`: build `startupWarnings` as the existing `self.startup_warnings` clone followed by `self.store.current_load_notice()?`, appended when `Some`.

The band then always shows the current text, and the stale "Back up that file before you leave read-only mode" line disappears. Not checked: the frontend's `startupNoticeDismissed` keeps a band the person already dismissed hidden. The new text only reappears if the frontend resets that flag when the content changes.
Tests:
- `leaving_saved_read_only_mode_keeps_the_skipped_records_first`: before, the notice contains "Back up". After, it contains the copy's `display_path`, with neither "Back up" nor "read-only". `load_notice()` stays as it was at opening. It fails in a mutated copy without the update.
- `leaving_read_only_mode_is_refused_while_the_copy_cannot_be_kept`: the notice still asks for a backup.
- `skipped_records_survive_the_next_save_in_a_preserved_copy`: `current_load_notice()` equals `load_notice()` on a writable launch.

### DESKTOP-R12 — Relaunch wording
Disposition: fixed (comment and doc only).
Verification: `quit::settle_failed_quit` turns `Stay` into `Retry` when no shell window remains, or when a Quit was requested while the dialog was open.
Change: the `RESTART_AFTER_QUIT` doc comment in `update.rs` and the matching sentence in STATE_MACHINES §1.5 now add "unless no window is left or another Quit was requested meanwhile (then it is a Retry, `quit::settle_failed_quit`)".

**Commands run:**
- `cargo test --locked --lib -- store:: update:: windows::` → `test result: ok. 109 passed; 0 failed; 1 ignored`.
- Mutated copy (scratch copy of the tree, shared target dir, copy deleted afterwards) → `2 failed`: `a_minimized_window_keeps_its_last_saved_frame` and `leaving_saved_read_only_mode_keeps_the_skipped_records_first`.
- Host `cargo clippy --lib --tests --features custom-protocol -- -D warnings` → `Finished`. Earlier attempts failed only on other lanes' in-progress files (`localswarm/cleanup.rs`, `maps/manager.rs`, `crash.rs`), plus one dead-code error in my new accessor, which the attribute above fixes.
- Windows-gnu `cargo clippy --target x86_64-pc-windows-gnu --lib --tests --features custom-protocol -- -D warnings` → `Finished`.

**Files changed this round:** `src-tauri/src/store.rs`, `src-tauri/src/store_tests.rs`, `src-tauri/src/windows.rs`, `src-tauri/src/update.rs` (comment only), `docs/STATE_MACHINES.md` (§1.5 sentence).

**New strings:** none. The kept-copy notice reuses the existing writable-launch text, now in `kept_copy_notice`.
