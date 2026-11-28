## Verdict for lane shell (round 3): no findings

All four round-2 findings are closed. A typed download directory is no longer lost or applied without confirmation on any Quit or close path that can be simulated off-device. Lane node tests, `tsc` and one browser run all pass.

### R2b and the switch deviation
- **Single gate.** `commit()` now returns early, leaving the edit `waiting`, when `typed(edit) && !confirmationAvailable`. Blur, Enter and `flushDrafts` all go through it.
- **Demote instead of delete.** The unavailable effect sets a typed `confirming` edit back to `waiting`. A switch edit is still deleted.
- **Nothing is applied without confirmation.** A save happens only after `consented`, which is set only by the prompt's trusted "Apply change" click.
- **Reruns of my real-module reproduction** (`adv-shell/settings-blur.test.mjs`, harness with `useEffect` running):
  - Quit, flush, then the blur caused by hiding the shell: the field keeps `/media/reader/zim`, the status says "This change is not saved yet. Leave the field or press Enter to review it.", 0 writes.
  - Quit cancelled: the text is still there. Enter while still unavailable opens no prompt. Enter once available opens the prompt with `/srv/a`, 0 writes.
  - A prompt already open when Quit starts is withdrawn and the text goes back to waiting, status "not saved yet", 0 writes.
- **These cover every Quit and close path, including the failed-Quit dialog's three branches.** Each one either hides the shell, which blurs the field, or withdraws an open prompt:
  - Cancel: text kept.
  - Quit Without Saving: lost, after being labelled "not saved yet".
  - Retry: the flush skips the gated edit.
- **Switch deviation.**
  - While the prompt is open, the switch shows the requested state. That is unchanged and visible next to the prompt.
  - When Quit withdraws the prompt, the edit is deleted and the switch shows the confirmed state again. 0 writes, and no prompt comes back after a cancelled Quit.
  - The only moment a switch shows an unapplied state with no prompt is the frame before the effect runs, while the shell is hidden. So a person is never shown a switch state that was not applied. I accept the deviation.

### Per-finding lines
- **R2b:** closed.
- **R7:** closed. `nativeEvent.keyCode === 229` returns before `commit`; I checked that it opens no prompt. A normal Enter still opens the review.
- **R8:** closed.
  - `startupFailure` is set only in the `goto` or boot `catch`. Later tests then use `assert.fail("Not run: …")`, so they show as failures, never as passes or skips, and the first failing test keeps the real cause.
  - `protocolTimeout: 120000` bounds a page that stops answering.
  - Residual, acceptable: the flag is per file, and `test-frontend-surfaces.mjs` keeps puppeteer's 30 s defaults.
- **R5b:** closed.
  - `Native background checkpoint unavailable: [NATIVE_DOCUMENT_ERROR] - {…}` now becomes "…That action did not finish. Try again. (NATIVE_DOCUMENT_ERROR)".
  - Left unchanged, as they should be: `Saving failed: [X_Y] - A_B`, `Saving failed: [X_Y] - {name} was busy` and `Saving failed: DOCUMENT_PROVIDER_ERROR`.
  - `[X_Y] - A_B` maps to the generic sentence, as the implementer's corrected report says.
  - Negligible: any `{…}` detail after a prefix is treated as an envelope.

### What I ran
- Lane node tests:
  - error-visibility 21/21
  - shell-dismiss 11/11
  - settings-surface 10/10
  - updates-settings 5/5
  - camera-window 4/4
  - provider-recovery 6/6
- `npx --no-install tsc --noEmit`: exit 0.
- `KIWIX_TEST_CHROME=… node --test scripts/test-shell-surfaces.mjs scripts/test-frontend-surfaces.mjs`, once: `tests 6, pass 6, fail 0` (70 s).
- Scratch checks in `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-shell/`: `settings-blur.test.mjs` (my earlier sequence plus two new ones, all passing) and `errortext.mjs`.

### What I could not verify
A real desktop Quit, window close, Dock quit or logoff; the native failed-Quit dialog; and the blur timing and IME `keyCode 229` behaviour in WKWebView and WebKitGTK.