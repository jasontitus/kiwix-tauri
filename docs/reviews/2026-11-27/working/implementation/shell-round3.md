# Lane `shell` — round 3 report (these four items only)

All four items are fixed in owned files. `tsc` is clean, the lane's 57 node tests pass, and both browser suites passed 6/6 three times. Fail-without-fix checks ran in a scratch copy at `842cdc1~1`, never in the shared tree. Not run: a real desktop Quit or window close, and WebKit's IME behaviour.

### SHELL-R2b — Quit or close still dropped the typed download directory
Disposition: fixed, with one deliberate deviation for switches.
Verification of the finding: confirmed. `onBlur` and Enter called `commit()` directly, bypassing the gate I had put only in `flushDrafts`, and the unavailable-prompt effect then deleted the edit. The reviewer's `adv-shell/settings-blur.test.mjs`, run against the fixed module, now shows the field keeping `/media/reader/zim` and the "not saved yet" status after the hide-triggered blur and after the cancel.
Change (`src/components/SettingsAutosave.tsx`):
- `commit()`: a typed edit that needs confirmation returns and stays `waiting` while the prompt is unavailable. This is now the single gate; the separate check in `flushDrafts` is removed.
- Unavailable effect: a typed `confirming` edit goes back to `waiting` instead of being deleted. After a cancelled Quit the text is still in the field and the prompt reappears on the next blur or Enter, not automatically.
- **Deviation:** both rules apply to typed text only (`edit.raw !== undefined`). A toggled switch (private mode, read-only) is still withdrawn and deleted as before. Demoting it would leave the switch showing a state that was never applied, with no blur or Enter to re-open the prompt, and it would resurface later from an unrelated flush. Say if you want switches demoted too.

Tests (`scripts/test-settings-surface.mjs`; the harness now runs `useEffect`):
- "hiding the shell for Quit blurs the field, and the typed directory survives a cancelled Quit" — the reviewer's sequence (`prompt(false)`, `flush()`, `onBlur()`), then cancel, then review by blur and by Enter.
- "a review prompt that Quit or close withdraws puts typed text back in its field".
- "a withdrawn prompt for a switch still reverts the switch" — guards the deviation.
- Observed failing pre-fix: the first two, yes; the third is a guard and passes either way.

Not exercised: real Quit, close, Dock quit, or the native failed-Quit dialog.

### SHELL-R7 — IME composition Enter opened the review on WebKit
Disposition: fixed
Change: the `onKeyDown` in `SettingsAutosave.tsx` also returns when `nativeEvent.keyCode === 229`.
Tests: "Enter in the download directory field opens the review, and only there" has the 229 case. Observed failing pre-fix: yes.
Not exercised: a real IME on WKWebView (the finding itself was inference).

### SHELL-R8 — a startup regression could burn the CI job
Disposition: fixed
Change (`scripts/test-shell-surfaces.mjs`):
- A module-level `startupFailure` is set when `goto` or the boot wait fails. Every later `open()` fails at once with "Not run: the shell did not start in an earlier test (…cause…)".
- `protocolTimeout: 120000` bounds a page that stops answering.

Tests: in the copy, with `main.tsx` made to throw and the boot wait shortened to 8 s, test 1 failed with the boot state and tests 2–5 failed immediately; the whole file took 15 s. With the real limits the worst case is one 120 s wait in this file.
Not covered: the flag is per file. `test-frontend-surfaces.mjs` is unchanged; its single test uses puppeteer's 30 s defaults.

### SHELL-R5b — JSON envelope after a Rust prefix
Disposition: fixed
Change: `src/components/UI.tsx` `nativeCodeText` — after a sentence prefix, `[CODE] - {…}` counts as a code, the same as `[CODE] - CODE`. `Native background checkpoint unavailable: [NATIVE_DOCUMENT_ERROR] - {"code":…}` now ends "…That action did not finish. Try again. (NATIVE_DOCUMENT_ERROR)". A sentence with braces after a prefix is still left alone.
Tests (`scripts/test-error-visibility.mjs`): new table rows, and the source-scan test now checks every native literal in the prefixed-with-envelope form. Observed failing pre-fix: yes.

**Correction to my previous report:** `[X_Y] - A_B` (two different codes, no prefix) is not left alone. It becomes "That action did not finish. Try again. (X_Y)". Only the prefixed form `Saving failed: [X_Y] - A_B` stays unchanged. Both are now asserted.

## Commands run (final)
- `npx --no-install tsc --noEmit` → exit 0.
- `node --test scripts/test-*.mjs` → `tests 528, pass 512, fail 2, skipped 14`. The two failures are the regeneration gates (`docs/ERRORS.md matches…`, `every UI string … localization manifest`).
- My five node files plus `test-provider-recovery.mjs` → `tests 57, pass 57, fail 0`.
- `KIWIX_TEST_CHROME=… node --test scripts/test-shell-surfaces.mjs scripts/test-frontend-surfaces.mjs`, three times → `tests 6, pass 6, fail 0` each.

## Files changed this round
`src/components/SettingsAutosave.tsx`, `src/components/UI.tsx`, `scripts/test-settings-surface.mjs`, `scripts/test-error-visibility.mjs`, `scripts/test-shell-surfaces.mjs`. No new user-visible strings.

## Doubts
- After a cancelled Quit the field no longer has focus, so the person must click into it and press Enter (or leave it) to get the prompt; the status line says so. I did not re-open the prompt automatically because you specified "next blur or Enter".
- If the window loses focus while the path is half typed (for example switching apps to copy a path), the blur opens the review prompt. That is the base `onBlur` behaviour and unchanged.
