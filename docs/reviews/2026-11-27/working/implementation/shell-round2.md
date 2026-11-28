# Lane `shell` — follow-up report (review fixes only)

All six items from the adversarial review are done in owned files, plus `src/diagnostics.ts` as assigned. `tsc` is clean, the lane's 54 node tests pass, and both browser suites passed 6/6 three times in a row. Nothing ran on a device. Every "fails without the fix" check below was made in a scratch mirror at the pre-review checkpoint (`9f93810~1`), not in the shared tree.

### SHELL-R2 — typed download directory showed "Saving…"; Quit dropped it silently
Disposition: fixed
Verification of the finding: confirmed in `SettingsAutosave.tsx`. A waiting, confirmation-gated edit counted as `pending`; on Quit, `flushDrafts` promoted it to `confirming` and the unavailable-prompt effect then deleted it.
Change (`src/components/SettingsAutosave.tsx`):
- **Status line:** a waiting edit that needs confirmation is no longer "pending". The line reads "This change is not saved yet. Leave the field or press Enter to review it." (`data-state="confirm"`). A different preference that really is saving still shows "Saving…".
- **Enter:** `input()` now returns an `onKeyDown` that commits a confirmation-gated edit on Enter. The Settings form has no submit control, so Enter previously did nothing and the new hint would have been false. Other fields are untouched, including the newline in the OPDS list.
- **Quit / window close:** the typed directory is neither applied nor discarded. `flushDrafts` skips confirmation-gated edits while the App-level prompt cannot be shown (tracked in a layout effect, so it is already false when the Quit flush runs). The edit stays in its field, marked not saved. If Quit completes, it ends with the process, after the status line had already said it was not saved. If Quit is cancelled, the text is still there to review.
- **Why not block Quit:** failing the flush now goes to the new failed-Quit native dialog, whose text is about open tabs and which offers "Quit Without Saving". A prompt also cannot be awaited inside the 10 s flush window.
- A prompt that is already open when Quit starts is still withdrawn, as before.

Tests:
- `scripts/test-settings-surface.mjs`: "a typed download directory is shown as not saved, never as being saved"; "Enter in the download directory field opens the review, and only there"; "Quit or close with an unreviewed download directory neither applies nor discards it".
- `scripts/test-shell-surfaces.mjs` (real browser): "Settings hides desktop-only copy without windows, and a typed download directory waits for review".
- Observed failing pre-review: yes, all four.

Not exercised: a real desktop Quit.

### SHELL-R3 — a recoverable diagnostic replaced an open blocking alert
Disposition: fixed
Change: `src/App.tsx` `onError` uses a functional `setError`. An incoming non-blocking entry leaves a current `surface === "dialog"` entry in place; it is still recorded in the diagnostics log. A newer blocking failure still replaces the alert's message, and the band keeps "newest wins".
Tests: `scripts/test-error-visibility.mjs` "the App reporter owns one surface per failure and paints no cancellation", extended with the current-state cases. Observed failing pre-review: yes.

### SHELL-R4 — focus fell to `<body>` when the alert's origin was gone or disabled
Disposition: fixed
Change: `App.tsx` focus-return effect. If the origin control is disconnected, disabled, or outside the top-most open dialog, focus goes to that dialog's first enabled control. With no dialog open and no usable origin, nothing is forced.
Tests: `test-shell-surfaces.mjs` "a failure inside a modal is shown, announced and dismissed above it, with one owner" now disables the originating button before Dismiss and asserts focus lands on the editor's Close button with the draft intact. Observed failing pre-review: yes (timed out at that step).

### SHELL-R5 — two code shapes still reached the user raw
Disposition: fixed
Change: `src/components/UI.tsx` `nativeCodeText`:
- After a sentence prefix, a self-identical `[CODE] - CODE` of any family is a code. `Creating the Wi-Fi hotspot failed: [ACTIVITY_DESTROYED] - ACTIVITY_DESTROYED` now ends "…That action did not finish. Try again. (ACTIVITY_DESTROYED)".
- A `{…}` detail next to a code is treated as no detail.
- Still left alone: `Saving failed: DOCUMENT_PROVIDER_ERROR`, `Saving failed: [X_Y]`, `[X_Y] - A_B`, and a sentence that merely contains braces.

Tests: `test-error-visibility.mjs` "native code families map…" and "a sentence is never mistaken for a native code" extended; the source-scan test now also checks every native literal in the prefixed-and-wrapped and envelope forms. Observed failing pre-review: yes.

### SHELL-7 handoff 1 — `reportError` ignores a user cancellation
Disposition: fixed
Change:
- `src/diagnostics.ts` `reportError` now returns false, without recording, for `isUserCancelledError`, exactly like `isSuspendedError`.
- I removed the now-redundant check from `App.onError`, so there is one gate.
- Effect: Catalog and Maps (which call `reportError` directly) no longer paint or record a declined low-storage prompt.
- Still open, for the panes lane: `Downloads.start` paints "Download cancelled." inline because it calls `errorText` unconditionally (handoff 2).

Tests: `test-error-visibility.mjs` "a cancellation the user chose is neither recorded nor rendered" (the proposed line, plus the sentence-alone counter-case); the App reporter test now also asserts an empty log. Observed failing pre-review: yes.

### R1 — does it need anything from `App.onError`?
No. The existing third parameter is enough: `onError(e, "blocking", false)` records without painting. I widened the type so callers can use it: `SurfaceProps.onError` in `UI.tsx` now declares the optional `showGlobally`, which covers `Library.tsx` and `Maps.tsx`. `Downloads.tsx` declares its own `onError: (error: unknown) => void` and must widen that itself.

### Item 6 — browser test robustness
Disposition: fixed
Change (`scripts/test-shell-surfaces.mjs`):
- The one fixed sleep is gone; that step now waits for the shortcut's lifecycle check to answer.
- Condition waits allow 45 s; navigation and startup allow 120 s, because each test starts a cold dev server; per-test limit is 420 s.
- One bounded wait remains by design: a 1.5 s negative check that the review prompt does not open while typing. It can only pass late, never fail from load.

Evidence: with two copies of the suite running beside three passes of the node suite, the first attempt reproduced a load failure (`Navigation timeout of 30000 ms`). After raising the limits, both copies passed 5/5.

## Commands run (final)
- `npx --no-install tsc --noEmit` → exit 0. One earlier run showed three errors in `Maps.tsx` (another lane mid-edit); the retry was clean.
- `node --test scripts/test-*.mjs` → `tests 514, pass 501, fail 2, skipped 11`. The two failures are the regeneration gates (`docs/ERRORS.md matches…`, `every UI string … localization manifest`).
- My five node files plus `test-provider-recovery.mjs` → `tests 54, pass 54, fail 0`.
- `KIWIX_TEST_CHROME=… node --test scripts/test-shell-surfaces.mjs scripts/test-frontend-surfaces.mjs`, three times → `tests 6, pass 6, fail 0` each.

## Files changed in this round
`src/App.tsx`, `src/components/UI.tsx`, `src/components/SettingsAutosave.tsx`, `src/diagnostics.ts`, `scripts/test-error-visibility.mjs`, `scripts/test-settings-surface.mjs`, `scripts/test-shell-surfaces.mjs`.

New user-visible string: `This change is not saved yet. Leave the field or press Enter to review it.`

## Handoffs
- **Panes lane:**
  - R1 call sites in `Library.tsx`, `Downloads.tsx`, `Maps.tsx`: pass `false` as the third `onError` argument. Add the reviewer's `remove_book` regression.
  - Handoff 2: `Downloads.start` should skip the inline text for `isUserCancelledError` and drop its second owner.
  - Widen `Downloads`' own `onError` prop type.
- **Not addressed:** SHELL-R6 (iOS `*_RETIRED` codes are rewritten by Rust `user_error::plain` before the shell sees them). It is in Rust and was not assigned to me.

## Doubts
- **Quit semantics for R2** are a judgment call: the unreviewed directory survives a cancelled Quit and is lost with a completed one, having been labelled "not saved yet" beforehand. If you would rather Quit stop and ask, that needs a failed-Quit reason of its own on the Rust side.
- **Orphan edit:** if Settings were unmounted while the prompt is unavailable, a gated edit would stay waiting with no field to leave until the next flush. I found no path that does this on desktop, the only platform with the field.
- **Review prompt focus (pre-existing):** the browser run showed initial focus on the header Close button, not Cancel; `autoFocus` loses to `showModal()`. Both decline, so I left it.
- **R4** uses the dialog's first enabled control, which is normally its Close button.
