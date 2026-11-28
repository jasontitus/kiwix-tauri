# Lane `panes` — round 5 report (changes since round 4 only)

PANES-R9 and PANES-R10 are fixed: the redaction regex in `UI.tsx` no longer uses lookbehind, and its extension check now accepts only `.part`, `.zim` and `.zimaa`-style names. `tsc` is clean, and the pane and errorText node tests pass 116/116. `test-pane-surfaces.mjs` passed 3/3 in each of three runs (23.1 s, 38.9 s, 30.0 s). All 46 test-9 inputs give the same redacted output in V8 and in JSC (the reviewer's harness), with no folder text left showing. The regression was seen failing in a scratch copy only.

To see the change: `git diff d98cac2 -- src/components/UI.tsx`. `scripts/test-pane-actions.mjs` is untracked, so `git diff` does not show it.

### PANES-R9 — remove the lookbehind
Disposition: fixed, using the form adopted from the review.
Verification of the finding: as the review says, this was the only lookbehind in `src/`. `grep -c "(?<="` on `UI.tsx` now gives 0.
Change: `src/components/UI.tsx` `bareFilesystemPath`.
- The end of a path is now matched by `(?:(?=delimiter|")" not followed by a separator|space+verb|$)|EXT(?=[ \t]+\((?=[A-Z][a-z])|\.(?:[ \t]+(?=[A-Z])|$)))`.
- So the extension is consumed inside the match and checked with a lookahead, instead of with a lookbehind.
- The prefix group and the replace callback are unchanged.
- The "Lookbehind needs Safari 16.4" comment is deleted, and the remaining comment is updated.
Tests: the round-4 assertions still pass, now also in JSC (below).

### PANES-R10 — narrow the extension check
Disposition: fixed
Change: `EXT` is now `\.(?:part|zim(?:[a-z]{2})?)`, in place of `\.[A-Za-z0-9]{1,12}`.
Tests: `test-pane-actions.mjs` test 9 adds:
- "U.S. History", "J.R.R. Tolkien", "Notes.v2 (Draft)" and "Report.Final (Draft)". Each is checked fully redacted in the same three sentence shapes as the earlier folders.
- `Could not open [File]. Try again.` for a `.zimaa` name.
- The map list form for a "U.S. History" folder.
- Still passing: the five reasons in both map forms, `.zim. Try again.`, "(see [File])", and the older shapes.
Observed failing with the round-4 gate restored (in the lookahead form): yes. It printed `Could not open [File]. History/wikipedia.zim right now`.

### JSC check
- `scratchpad/panes/jsc-gen.mjs` takes the shipped `filesystemRoot`, `quotedFilesystemPath` and `bareFilesystemPath` from `UI.tsx`, builds the 46 test-9 inputs and their V8 outputs, and writes `scratchpad/panes/jsc-case.js`.
- `scratchpad/adv-panes/jsc` (JSC 2.52) ran it: `{"cases":46,"mismatches":[],"leaks":0}`. In V8 the same scan gives `leaks: 0`.

## Commands run
- `npx --no-install tsc --noEmit` → exit 0
- `node --test scripts/test-pane-actions.mjs test-library-suggestion-focus.mjs test-error-visibility.mjs test-edition-surfaces.mjs test-download-state.mjs test-collections-async.mjs test-mobile-voice-search.mjs test-provider-recovery.mjs test-settings-surface.mjs` → tests 116, pass 116, fail 0
- `node --test scripts/test-*.mjs` → tests 531, pass 515, fail 2 (the known `docs/ERRORS.md` and localization-manifest gates), skipped 14
- `KIWIX_TEST_CHROME=…/chrome-nosandbox.sh node --test scripts/test-pane-surfaces.mjs` ×3 → 3/3 each
- JSC harness, as above.

## Files changed in this round
- `src/components/UI.tsx` (the `bareFilesystemPath` regex and its comment only)
- `scripts/test-pane-actions.mjs`

## New or changed user-visible strings
None.

## Doubts
- **Upper-case extensions.** The check matches lower-case extensions only, as adopted. A `.ZIM` or `.Part` name before ". Try again." or " (Reason" eats the rest of the sentence, as base did. Nothing is revealed, but the tail of the sentence is lost.
- **Other extensions.** Any other extension in that position (for example `.pdf. Try again.`) also eats the sentence tail, as base did. The reviewer noted this.