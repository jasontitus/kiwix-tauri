# Lane `panes` — round 3 report (changes since round 2 only)

PANES-R6, PANES-R7 and FEATURES-R13 (both parts) are fixed. `tsc` is clean, the full node suite shows only the two known regeneration failures, and `test-pane-surfaces.mjs` passed 3 runs in a row (3/3 each: 32.1 s, 37.2 s, 27.0 s). Every new test was seen failing with its fix reverted in a scratch copy (`scratchpad/panes/base`), never in the shared tree. There are no handoffs.

To see the full change: `git diff d98cac2 -- src/components/{Catalog,Library,MapBlockProgress,UI}.tsx`. The test file `scripts/test-pane-actions.mjs` is untracked, so `git diff` does not show it.

### PANES-R6 — `exact` must follow Rust `series_compatible`
Disposition: fixed (the owner's split: the Rust rule for `exact`, the looser rule for the badge)
Verification of the finding: I compared against `catalog_index::series_key` and `series_compatible` in Rust:
- Variants are `_pictures`, `_videos`, `_details` and `_ftindex`, compared as a whole with `==`. A variant stated on one side only is a different series.
- An empty or conflicting variant value makes `series_key` return `None`, so nothing matches.
- The language must also be equal.

Change: `Catalog.tsx`
- New `SERIES_VARIANT_KEYS` and `seriesVariants(tags)`. It returns a sorted `key:value;…` string, or `null` for an empty or conflicting value, as Rust does.
- `InstalledArchive` now also stores `series` and `language` (trimmed, lower-case).
- `exact` (which hides Download) now requires the same flavour, the same `series`, a non-null entry series, the same language and the same day.
- The related "Installed edition {date}" badge keeps the looser rule: the same name, no conflicting stated flavour, and no conflicting variant among the three keys that both sides state.
- Adding the language check is my own reading of "follow Rust exactly"; you did not ask for it.

Tests: added to `test-pane-actions.mjs` test 1. Each of these is matched but not exact, so the badge is still shown:
- `_ftindex:yes` against `_ftindex:no`;
- `_pictures:no` against no `_pictures` tag;
- no tag against `_videos:no`;
- a different language.

Also added: equal variants in a different tag order are exact; conflicting duplicates are never exact. Observed failing with the old `exact` predicate: yes.

### PANES-R7 — a correction tap raised the phone keyboard
Disposition: fixed
Change: `Library.tsx`
- The inline click handler became the named function `searchCorrection(query)`.
- On desktop it focuses the search field, as before. With `isMobile` (already imported in the file) it sets `focusResults` instead.
- A new effect, keyed on `[results, busy]`, focuses the results region once the search has settled, then clears the flag. After a failure the region is absent and the shell's surface reports the error.
- The results region (`aria-live="polite"`) now has `ref={resultsRegion}` and `tabIndex={-1}`.

Tests: new test 8 in `test-pane-actions.mjs`. It runs the real `searchCorrection` and the real effect: desktop focuses the input; mobile focuses the results only after they arrive, and only once. Observed failing with the mobile branch removed: yes.
Not exercised: a real phone and its soft keyboard. The browser test runs on desktop, where its existing input-focus assertion still passes.

### FEATURES-R13.1 — iOS path redaction swallowed the start of the reason
Disposition: fixed (the redaction now stops at the path's end)
Verification of the finding: reproduced with the shipped `errorText` on iOS. "…kept here: [File] is not available right now… again), [File]".
Change: `UI.tsx` `bareFilesystemPath`, edited for this item only. Two more points now end a path:
- before ` (` followed by a capitalised word (a parenthesised reason);
- at a full stop followed by spaces and a capital letter, or at the end of the text (the end of a sentence).

Every earlier terminator is kept. No function was added, because `test-error-visibility.mjs` extracts `errorText`'s parts by name.

I did not show the map sentences unredacted. Both sentence forms still contain the full path ("kept here: <path>", "kept at <path>").

Tests: new test 9 in `test-pane-actions.mjs`, the shipped `errorText` with `isIOS: true`. For all five reasons (the three current `partials.rs` sentences and the two round-4 sentences) it checks:
- the list form: `…kept here: [File] (<reason>), [File]`;
- the `Err` form: `…kept at [File]. <reason>`, with a path inside `.streetzim-cleanup-…/`.

It also checks four older shapes still redact: a path before `:`, a path with spaces before "is", a path in quotes, and a path at the end of a sentence. That last one now keeps the final full stop, where the old regex swallowed it.

Observed failing with only the regex line reverted: yes, with the reviewer's exact output.

Residual: a path that itself contains ` (Capitalised` or `. Capital` would be cut there. The remainder of that path would then show unredacted on iOS. Kiwix-owned map paths do not take that shape.

### FEATURES-R13.2 — the recovery note contradicted the alert
Disposition: fixed
Change: `MapBlockProgress.tsx` `MapRecoveryPaths`. The note now reads, as you suggested: "Kiwix kept these files. Press Delete again to retry the ones it can prove are its own; remove any others yourself, then dismiss this warning." The doc comment was updated to match. The note is still part of each Dismiss button's `aria-describedby`.
Tests: none new. The browser test does not assert this text.

## Commands run
- `npx --no-install tsc --noEmit` → exit 0
- `node --test scripts/test-pane-actions.mjs scripts/test-library-suggestion-focus.mjs scripts/test-error-visibility.mjs scripts/test-edition-surfaces.mjs scripts/test-download-state.mjs` → tests 65, pass 65, fail 0
- `node --test scripts/test-*.mjs` → tests 531, pass 515, fail 2, skipped 14. The failures are the known `docs/ERRORS.md` and localization-manifest gates.
- `KIWIX_TEST_CHROME=…/chrome-nosandbox.sh node --test scripts/test-pane-surfaces.mjs` ×3 → 3/3 each
- Mutation runs in scratch: with the old `exact` predicate, without the mobile branch, and with the old redaction regex, tests 1, 8 and 9 each fail.

## Files changed in this round
- `src/components/Catalog.tsx`
- `src/components/Library.tsx`
- `src/components/MapBlockProgress.tsx`
- `src/components/UI.tsx` (the `bareFilesystemPath` regex and its comment only)
- `scripts/test-pane-actions.mjs`

## New user-visible strings
- "Kiwix kept these files. Press Delete again to retry the ones it can prove are its own; remove any others yourself, then dismiss this warning."
- Removed: "Kiwix kept these files because it could not prove they were its own to delete. Remove them yourself if you no longer need them. Dismissing a warning never deletes the file."

## Doubts
- **Language check on `exact`.** An OPDS `language` written differently from the ZIM `Language` (for example `en` against `eng`) now gives only the related badge, so Download stays offered.
- **Redaction and the round-4 wording.** The round-4 sentences contain "If it is on a drive…" inside the parentheses. They are redacted correctly only because they contain no filesystem root. A future reason that names a path inside the parentheses is not covered by test 9.