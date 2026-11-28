# Lane `reader` — round 3 report (R4 and R5 only)

Both items are done. `tsc --noEmit` is clean and the lane tests are 242/242.

### READER-R4 — photo secret scoped to where the photo chooser exists
Disposition: fixed

Verification of the finding: `initializeDocument` sent `photoSecret` unconditionally, and the `photo-request` handler checked only `isTrusted`, the secret, and that no request was open.

Change (`src/components/Reader.tsx`):
- New module helper `photoChooserOffered(location)` returns `isWindows && location.restricted !== true`. It is the single expression now used for the `photoChooser` flag, for issuing the secret, and for honouring a request.
- `initializeDocument` puts `photoSecret` in `reader-init` only when that helper is true. When it is false the field is absent and the shell's secret map is cleared, so entering restricted mode drops the secret the frame had.
- The `photo-request` handler replies `photo-cancel` unless the helper is true for the admitted session, even when the secret matches.
- No bridge change was needed: it already stores `""` when the field is missing and refuses without `photoChooser`.

Tests (`scripts/test-reader-frame-admission.mjs`, 39 pass):
- "only the session being initialised keeps a photo secret" — now runs as Windows, unrestricted, and also asserts `photoChooser: true`.
- "no photo secret is issued where the photo chooser is not offered" — new. Covers off Windows, restricted mode, and a Windows frame that re-sessions into restricted mode.
- "a photo request is honoured only where the photo chooser is offered" — new. Covers wrong secret refused, right secret accepted on Windows, and a matching secret refused off Windows and in restricted mode with no dialog opened.
- The two new tests fail on a scratch copy of the pre-change tree (`353712c`) and pass now.

Not exercised: a real Windows webview. The residual (forged hello plus `document.open()`) is now confined to Windows with restricted mode off, as you intended; I did not re-run `attack2.mjs`, since its stand-in shell does not model the platform flag.

### READER-R5 — reload failure wording
Disposition: fixed

Verification of the finding: the text asserted a loss the shell cannot know.

Change: the last sentence of the `reopenFreshDocument` failure message now ends "may be lost."

Tests: `scripts/test-reader-fresh-document.mjs` asserts the message ends with "may be lost." This fails on the pre-change copy.

Not exercised: n/a.

## Commands run
- `npx --no-install tsc --noEmit` → no output.
- `node --test scripts/test-reader-*.mjs scripts/test-sensor-policy.mjs scripts/test-media-policy.mjs scripts/test-speech-preferences.mjs scripts/test-reference-transitions.mjs` → `# tests 242 # pass 242 # fail 0`.
- `node --test scripts/test-*.mjs` → `# tests 525 # pass 509 # fail 2 # skipped 14`. The two failures are the generated `docs/ERRORS.md` check and the localization manifest check, as before.
- Fail-without-fix check ran in a scratch copy under `scratchpad/reader/round2`, not the shared tree → 3 failures there (the two new R4 tests and the R5 assertion).

## Files changed this round
- `src/components/Reader.tsx`
- `scripts/test-reader-frame-admission.mjs`
- `scripts/test-reader-fresh-document.mjs`

Changed user-visible string (replaces the round-2 one; collect with `node scripts/localization.mjs generate`):
- `"Kiwix could not confirm that this page finished saving, so it was closed. Choose Reload article to open it again. Anything the page had not finished saving may be lost."`

## Handoffs / contract changes
- `reader-init` no longer carries `photoSecret` off Windows or in restricted mode. Only `Reader.tsx` and `reader-bridge.js` speak this protocol.

## Doubts
- None new. The round-2 doubts about the size of the admission state machine still stand.
