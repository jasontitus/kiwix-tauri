## Verdict for lane shell (round 2): only non-blocking findings. R2 is not closed.

R2 still loses the typed download directory on Quit and close. I reproduced it with the real module and a browser blur check; details below. R3, R4, R5 and the `diagnostics.ts` change are closed. Two small new findings (IME Enter, the browser-test fail-fast budget) and one leftover NET-3 nit.

### SHELL-R2b — Quit or close still drops the typed download directory, because the blur bypasses the new gate
Severity: Should-fix (it fails to close R2, which was Should-fix)
Fix under review: R2 / SHELL-6
Evidence class: reproduced
- **Chromium (headless 158):** I focused an input inside a container and set the container to `display:none`, as `hidden={quitting}` does. `blur` and `focusout` fire on the next frame (`adv-shell/blurhide.mjs`).
- **Real `SettingsAutosave` module** (`adv-shell/settings-blur.test.mjs`): a copy of the lane harness with `useEffect` actually run. Sequence: type a path, `prompt(false)`, `flush()`, then the `onBlur()` that hiding the shell causes. Result: the field reverts to `/home/reader/Downloads` and the status says "Changes are saved automatically." Then `prompt(true)` (Quit cancelled) gives no prompt, the field still shows the old directory, and there are 0 writes.

Location: `src/components/SettingsAutosave.tsx`. Only `flushDrafts` checks `confirmationAvailable`; `input().onBlur` and `onKeyDown` call `commit()` directly. `commit()` sets `phase = "confirming"` whenever `needsConfirmation`. Then `SettingsPolicyConfirmation`'s `useEffect`, with `available` false, runs `edits.delete(key)`.

Trigger, on every path:
- **Native Quit by keyboard shortcut**, through the `kiwix-quit-flush` handler. macOS Dock quit and logoff probably reach the same handler; I did not trace them. `flushSync(setQuitting(true))` hides the shell, which blurs the focused field.
- **Window close button and Dock quit:** the window, and so the field, normally loses focus first, so the prompt opens while still available. The close or Quit that follows then withdraws it and deletes the edit.
- **The failed-Quit dialog**, all three branches:
  - Cancel: the shell comes back with the typed text gone and no prompt.
  - Quit Without Saving: lost, which is acceptable because it was labelled "not saved yet".
  - Retry: the edit is already gone.

So the claim "if Quit is cancelled, the text is still there to review" does not hold. Nothing is ever applied without confirmation, but a value typed into the field disappears without any notice.

Smallest correct fix:
1. In `commit()`, when `needsConfirmation(key, edit) && !confirmationAvailable`, return and leave the edit `waiting`.
2. In the `available === false` effect, set a `confirming` edit back to `phase = "waiting"` instead of deleting it.

The prompt then reappears or stays reviewable after a cancelled Quit. Add a regression in `test-settings-surface.mjs` that runs `useEffect` and calls `onBlur()` after `prompt(false)`.

### SHELL-R7 — Enter that commits an IME composition opens the review on macOS
Severity: Nit
Fix under review: R2 (the new `onKeyDown`)
Evidence class: inference (WebKit behaviour, not run here)
Location: `SettingsAutosave.tsx` `input().onKeyDown`: `event.isComposing || event.nativeEvent?.isComposing`.
Trigger: macOS (WKWebView), a CJK IME, typing a directory name and pressing Enter to commit the composition.
Causal path: WebKit sends that Enter as `keydown` with `isComposing === false` and `keyCode === 229`. `awaitsReview` is true, so `commit()` opens the prompt over a half-typed path, which is the original SHELL-6 symptom.
Smallest correct fix: also return when `nativeEvent.keyCode === 229`.

The OPDS textarea is unaffected: `awaitsReview` is false for it, so Enter is not prevented. None of the nine `{...input()}` spreads has its own `onKeyDown` that would conflict.

### SHELL-R8 — A startup regression now takes about 2 minutes per browser test to fail and can hit the CI job limit
Severity: Nit
Fix under review: item 6 (browser-test waits)
Evidence class: source-traced
Location:
- `scripts/test-shell-surfaces.mjs:157,160`: 120 s for `goto` and boot.
- `:156,168`: 45 s per wait.
- Per-test timeout: 420 s.
- `.github/workflows/checks.yml`, "Frontend build and tests": `timeout-minutes: 20`.

Causal path:
- An ordinary defect still fails fast, because the first unmet `until` throws after 45 s.
- A defect that stops boot costs 120 s in each of the 5 shell tests plus the frontend test, about 12 minutes, inside a 20-minute job that also builds and runs the other JS tests. The likely result is a cancelled job with no assertion message.
- A page-side hang in `page.evaluate` is bounded by puppeteer's protocol timeout (180 s default), not by the 45 s waits.

Smallest correct fix: after the first test fails at boot, fail the rest immediately (for example, a module-level `bootFailed` flag checked in `open`). Or keep 120 s only for the first, cold test and use about 45 s for the rest.

### SHELL-R5b — A JSON envelope after a Rust prefix is still shown raw
Severity: Nit
Fix under review: R5
Evidence class: reproduced (`adv-shell/errortext.mjs`)
Location: `UI.tsx` `nativeCodeText`: `else if (prefix && !(bracketed && detail === code)) return null;`. The `{…}` rule applies only in the earlier check.
Trigger: `Native background checkpoint unavailable: [NATIVE_DOCUMENT_ERROR] - {"code":…}` is returned unchanged. That prefix is in `mobile_service.rs:1676` (iOS). I found no producer of this exact shape today, so it is latent.
Smallest correct fix: treat a `{…}` detail as `detail === code` in that condition as well.

The implementer's report also says `[X_Y] - A_B` is "left alone"; it actually becomes "That action did not finish. Try again. (X_Y)". That output is acceptable; only the report is inaccurate.

### Earlier findings
- **R2:** not closed, because the hide- or window-triggered blur commits the edit and the unavailable effect deletes it (R2b). The "Saving…" status itself is fixed.
- **R3:** closed.
  - The functional `setError` keeps a `surface === "dialog"` entry against any non-blocking entry, in either order within a burst, because the updater sees the latest queued state.
  - A blocking failure still replaces the alert's message, and the band keeps "newest wins".
- **R4:** closed.
  - With the origin removed, disabled, or in a lower dialog, focus goes to the top-most dialog's first enabled control.
  - If `elementFromPoint` returns null or a non-dialog, it falls back to the last open dialog in document order.
  - Minor: the selector can match an `input[type=hidden]` or a hidden `[tabindex]`, in which case `focus()` does nothing and focus stays on `<body>`. Not worth a finding.
- **R5:** closed, apart from the nit above.
  - Adversarial inputs behave: `[ZIM] - ZIM`, `Saving failed: DOCUMENT_PROVIDER_ERROR`, `Saving failed: [X_Y]`, `Use {name}: HTTP_PROXY`, `[Use {braces}] - HOTSPOT_FUTURE` and `Error: {} [A_B] - A_B` stay unchanged. `Time: 10:00: [A_B] - A_B` maps with its prefix kept.
  - Combined with the new `plain`: bare shell-family codes now pass Rust before `has_internal_terms`, so `BLE_OWNER_RETIRED` reaches the shell, which maps it to "Nearby sharing is no longer active…". This closes R6 on iOS.
  - Unmapped codes come back bare and become "That action did not finish. Try again. (CODE)". Nothing is mapped twice and no instruction is lost. Codes containing internal words, such as `(JOIN_LEASE_RETIRED)`, now appear in the parentheses: owner decision.
- **SHELL-7 `diagnostics.ts`:** closed.
  - `reportError` returns false without recording for `isUserCancelledError`, and `App.onError` relies on it, so there is exactly one gate.
  - Only `api.request` creates `UserCancelledError`, and only for `"declined"`. No Rust or TS producer emits `KIWIX_USER_CANCELLED`.
  - The direct `reportError` callers (Collections, Catalog, Maps, CrashReport) lose only the declined-prompt case. No real failure is suppressed.
- **Item 6 (browser waits):** mostly fine; see R8. The one remaining fixed 1.5 s check (`:362`) is a negative check: it can only miss a late prompt, never fail because of load.

### What I ran
- Node tests:

  | File | Result |
  |---|---|
  | `test-error-visibility.mjs` | 21/21 |
  | `test-shell-dismiss.mjs` | 11/11 |
  | `test-settings-surface.mjs` | 7/7 |
  | `test-updates-settings.mjs` | 5/5 |
  | `test-camera-window.mjs` | 4/4 |
  | `test-provider-recovery.mjs` | 6/6 |

- `npx --no-install tsc --noEmit`: exit 0.
- `node --test scripts/test-*.mjs`: `tests 520, pass 507, fail 2, skipped 11`. The two failures are the known regeneration gates `docs/ERRORS.md matches…` and `every UI string … localization manifest`.
- `KIWIX_TEST_CHROME=… node --test scripts/test-shell-surfaces.mjs scripts/test-frontend-surfaces.mjs`, three times: `tests 6, pass 6, fail 0` each (49 s, 33 s, 38 s).
- Scratch reproductions, all in `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-shell/`: `blurhide.mjs`, `settings-blur.test.mjs` and `errortext.mjs`.

### What I could not verify
- A real desktop Quit, window close, Dock quit or logoff, and the native failed-Quit dialog. The R2b path is shown with the real module plus Chromium's blur-on-hide; WKWebView and WebKitGTK blur timing was not run.
- WebKit IME `keyCode 229` behaviour (R7) is inference.
- No Rust tests were run; the `plain` behaviour is traced from source.