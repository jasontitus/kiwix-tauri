Round 4: both items are fixed, and the lane tests, host clippy, the Android clippy script, tsc and the node test all pass.

### SERVICE-R3-1 — a withdrawn notice that came back stayed hidden
Disposition: fixed.
Change:
- `src/startup-notice.ts`:
  - `visibleStartupNotices(warnings, dismissed)` returns the current notices that are not dismissed.
  - `dismissStartupNotice(warnings)` records exactly what is shown.
  - `pruneStartupNoticeDismissals(warnings, dismissed)` drops dismissals of notices that are no longer current. It returns the same array when nothing changes, so React skips the update.
- `src/App.tsx`:
  - A `useEffect` on `snapshot?.startupWarnings` prunes the dismissed list. It does nothing while there is no snapshot, so a reload does not reopen closed notices.
  - The band renders only the visible notices, so reopening it no longer lists ones already closed.
- No new `t()` strings.
Tests: `scripts/test-startup-notice.mjs` → `pass 3, fail 0`.
- The assertion that a withdrawn notice stays dismissed is replaced.
- New case: a notice is withdrawn and then noted again → visible again.
- Also covered: reopening shows only `[CANCELLED]`, and an unchanged snapshot keeps the same array.
- With the prune disabled in a scratch copy, the withdraw-then-re-add test failed (`not ok 3`).

### SERVICE-R3-2 — an unrelated import withdrew the notice
Disposition: fixed.
Change (`service.rs`):
- New `cancelled_imports: Mutex<HashSet<String>>`. Each cancelled manual import is keyed by `cancelled_import_key(path)`: the archive UUID if the file still opens, otherwise its canonical path. The key is taken before any staged copy is discarded, so a picker retry that makes a fresh copy still counts as the same archive.
- Every admitted book removes its UUID and its path from the set. The notice is withdrawn only when the set has just become empty.
- An unrelated import, or the LIB-3 reopen of a different registered book, leaves the notice in place.
Tests: `a_privacy_switch_cancels_a_running_import_and_later_imports_still_work` now imports an unrelated archive and then reopens it through the registered-path shortcut; the notice stays both times. Importing the cancelled archive withdraws it. It passes. That this extension would fail on the old code is inferred, not run: the old code withdrew the notice on any successful import.

**Commands run** (with `SHERPA_ONNX_ARCHIVE_DIR` set)
- `cargo test --lib -- service:: server:: spelling:: search:: archive:: imports:: integrity:: watch:: native_pdf:: store::` → `ok. 484 passed; 0 failed; 6 ignored`
- Host clippy `--lib --tests --features custom-protocol -D warnings` → `Finished`
- Android clippy script → exit 0, `Finished`
- `npx --no-install tsc --noEmit` → exit 0
- `node --test scripts/test-startup-notice.mjs` → `pass 3`
- Windows-gnu clippy was not re-run; the round-4 changes have no Windows-specific code.

**Files changed:** `src-tauri/src/service.rs`, `src-tauri/src/service_tests.rs`, `src/startup-notice.ts`, `src/App.tsx` (startup band only), `scripts/test-startup-notice.mjs`.

**Strings:** none added or changed.

**Doubts:**
- The cancelled-archive set lives only in memory and is cleared on restart. So are the startup warnings it guards, so the two stay in step.
- If a cancelled file can no longer be opened, it is keyed by its path. A retry from a different copy then withdraws the notice only if that copy has the same path.