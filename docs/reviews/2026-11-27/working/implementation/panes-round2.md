# Lane `panes` — round 2 report (changes since round 1 only)

All five review items (PANES-R1 to R5) and the owner addition (rollback errors in the card) are fixed, in owned files only. There are no handoffs. Every new assertion was seen failing with its fix reverted, always in scratch copies under `scratchpad/panes/{base,mirror}`, never in the shared tree.

Verification:
- `tsc` is clean.
- The full node suite shows only the two known regeneration failures.
- `test-pane-surfaces.mjs` passed 3 of 3 runs (23.0 s, 23.2 s, 26.4 s).
- Other lanes' browser tests `test-frontend-surfaces.mjs` and `test-shell-surfaces.mjs` pass 6/6.

To see the full change: `git diff d98cac2 -- src/components/{Catalog,Library,Maps,Downloads}.tsx`. The two new test files `scripts/test-pane-actions.mjs` and `scripts/test-pane-surfaces.mjs` are not shown by `git diff`.

### PANES-R1 — "Open" offered for a different flavour when the installed copy's flavour is unknown
Disposition: fixed
Verification of the finding: confirmed by reading. Before the fix, a renamed copy (unknown flavour) on the same day as a maxi entry matched exactly.
Change: `Catalog.tsx`
- New `VARIANT_KEYS`, `archiveVariants` and `variantsCompatible`.
- If both sides state `_pictures`, `_videos` or `_details` and the values differ, the entry is a different archive: no badge.
- `exact` (the "Installed" badge with Open, no Download) now requires `archive.flavour === flavour`, that is, equal flavours or both unknown, and the same day.
- When only one side knows the flavour, the card gets the non-exact "Installed edition {date}" badge and keeps Download.
- One difference from Rust: Rust `series_compatible` also treats `_ftindex` as a variant, and also refuses when only one side states a variant. I followed your rule instead: the three named keys, compared only when both sides state them.

Tests: `test-pane-actions.mjs` "the official catalog's RFC 3339 date…" now also covers:
- a renamed copy with `_pictures:no` against the maxi entry: non-exact;
- an unknown-flavour copy against maxi: non-exact;
- `_pictures:yes` against `_pictures:no`: no match;
- the same picture variant with both flavours unknown: exact.

Observed failing: yes, with flavour equality removed from `exact`, and separately with the variant filter removed.

### PANES-R2 — `editionDay` read offset-less timestamps in local time
Disposition: fixed
Change: `editionDay` accepts a timestamp only if it matches `/^\d{4}-\d{2}-\d{2}T.*(?:Z|[+-]\d{2}:\d{2})$/i`; anything else gives `null`.
Tests: the same test adds three cases: `2026-10-16T00:30:00` gives `null`; the `+09:00` form gives `2026-10-15`; lower-case `t…z` is accepted. All pass under `TZ=Asia/Tokyo` too. Observed failing under `TZ=Asia/Tokyo` with the old regex: yes.

### PANES-R3 — a Maps failure painted inside an unrelated area dialog
Disposition: fixed
Change: `Maps.tsx`
- `reportStartError(e, regionId)` paints inline only when `openedRef.current?.id === regionId`. `submitMapRequest` and `start` pass `request.regionId`.
- `changeMapState` (job-panel Pause or Resume) failures always go to `onError(e)`.
- Opening another area's dialog while a card's `map_prepare` is pending already retires that request (`openDetail` → `dismissDecision`, unchanged from base), so its failure is dropped. The case that remains is a `map_start` in flight, and that is what the new browser case drives.

Tests: `test-pane-surfaces.mjs` test 1 delays a failing `map_start` from card 39 and opens Area 00's dialog meanwhile. The failure shows once, in the top-layer alert, not inside Area 00's dialog. Observed failing with the region check reverted: yes, it times out waiting for the alert.

### PANES-R4 — a failed search popped the suggestion list
Disposition: fixed ("searched since last keystroke")
Change: `Library.tsx`
- New state `searchSubmitted`: `performSearch` sets it to true, the field's `onChange` sets it to false.
- `searchFocused = searchInputFocused && !busy && !searchSubmitted`. This replaces `!results`: no suggestions after any search (results, zero hits or failure) until the next keystroke.
- The effect and the focus and blur handlers are untouched, so `test-library-suggestion-focus.mjs` still passes.

Tests: new `test-pane-actions.mjs` "a search that fails does not pop the suggestion list over its error". It runs the real `performSearch` with a rejecting search and re-evaluates the real `searchFocused` expression, which the helper now supports. After the failure no suggestion is requested; after a simulated keystroke one is. Observed failing with `!searchSubmitted` removed: yes.

### PANES-R5 — focus, recording and copy gaps
Disposition: fixed
Change:
- **Spelling correction:** `Library.tsx` clicking a correction now focuses the search field (new `searchInput` ref) before the results that contain the button are replaced.
- **Discard failure recorded:** `Downloads.tsx` `discardOriginal` failure now calls `onError(value, "blocking", false)`.
- **Focus when a discard removes the button:**
  - On both success and failure, the outcome is now set after `await load()`, so it renders together with the card's final buttons.
  - The focus effect, keyed on `[originalOutcome]`, decides once: if focus has dropped to `<body>` because the button went away, focus moves to the outcome.
- **Split archives:**
  - `delete_book` → `verified_parts` → `read_part_names` treats a primary ending in `.zimaa` as the first part and removes the whole part set.
  - The status exposes only `original.path` (the primary), so `/\.zimaa$/i` detects a split archive.
  - For a split archive, the local confirmation (permanent or trash) adds a paragraph saying every part is removed. A provider original is not deleted, so that wording is unchanged.

Tests:
- Browser test 2 asserts focus is on "Search articles" after the correction click. Observed failing without the focus call: yes, focus was on `<body>`.
- Browser test 3 now uses a `.zimaa` original and checks for the split paragraph. Observed failing with that paragraph disabled: yes.
- Not covered by any test: the discard failure path itself, which includes the new diagnostics recording and the case where the refreshed status removes the button.

### Owner addition — rollback errors in the transaction's card
Disposition: fixed
Change: `Downloads.tsx`
- The `rollbackEdition` catch now records with `onError(value, "blocking", false)` and shows the error in that transaction's card (`#edition-original-<id>`, `role=alert`), using the same outcome slot and focus handling as discard. It no longer writes the page-top `editionActionError`.
- The button now calls a new wrapper, `rollbackFromCard`, which clears the previous outcome.
- The success path and the early-return path of `rollbackEdition` are unchanged, because `test-edition-surfaces.mjs` (not mine) runs them with fixed bindings.
- The button disables itself while working, so on failure focus lands on the error text. A successful rollback still replaces the button with "Rolled back…" without moving focus. That is unchanged from base and has no handling yet.

Tests: browser test 3 makes `edition_rollback` fail. The error is painted once, in the card, with no alertdialog, and focus is on it. Observed failing with the old page-top owner restored: yes.

### PANES-R5 map path duplication
Disposition: left as is, as you asked.

## Commands run
- `npx --no-install tsc --noEmit` → exit 0
- `node --test scripts/test-pane-actions.mjs test-library-suggestion-focus.mjs test-error-visibility.mjs test-edition-surfaces.mjs test-download-state.mjs` → tests 63, pass 63
- `TZ=Asia/Tokyo node --test scripts/test-pane-actions.mjs` → pass 7, fail 0
- `node --test scripts/test-*.mjs` → tests 529, pass 513, fail 2, skipped 14. The failures are the known `docs/ERRORS.md` and localization-manifest gates. The browser tests are skipped when no browser is set.
- `KIWIX_TEST_CHROME=…/chrome-nosandbox.sh node --test scripts/test-pane-surfaces.mjs` ×3 → 3/3 each
- Same harness, `test-frontend-surfaces.mjs test-shell-surfaces.mjs` → 6/6
- Mutation runs in scratch copies:
  - node: the R1 exact rule, the R1 variant filter, the R2 regex and the R4 guard each fail their test;
  - browser: R3, the correction focus, the rollback owner and the split wording each fail.
- Your reproduction script `adv-panes/flavour.mjs` no longer runs as is: it does not extract the new helper names (`VARIANT_KEYS`, `archiveVariants`, `variantsCompatible`). The same case is now in `test-pane-actions.mjs`.

## Files changed in this round
- `src/components/Catalog.tsx`
- `src/components/Maps.tsx`
- `src/components/Library.tsx`
- `src/components/Downloads.tsx`
- `scripts/test-pane-actions.mjs`
- `scripts/test-pane-surfaces.mjs`

## New user-visible strings
- "This is the first part of a split archive: every part of it is removed."

## Doubts
- **Variant rule is looser than Rust.** It differs from `series_compatible` on `_ftindex` and on a variant stated by only one side (see R1).
- **No suggestions after any search.** After any search, including a zero-hit one, suggestions return only after a keystroke, not on refocus.