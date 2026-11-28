## Verdict for lane panes (round 2): only non-blocking findings

All five round-1 items and the rollback change are closed. One thing is left for your decision: R1's variant rule can still produce a wrong exact match in one narrow case (PANES-R6).

I re-reviewed `git diff d98cac2` of `Catalog.tsx`, `Maps.tsx`, `Library.tsx` and `Downloads.tsx`, plus `scripts/test-pane-actions.mjs` and `scripts/test-pane-surfaces.mjs`. I checked them against Rust `catalog_index::series_compatible`/`series_key` and `library_files::read_part_names`/`archive_suffix`.

### PANES-R6 — The variant rule can still mark a different archive as exact when neither side states a flavour
Severity: Should-fix (low impact)
Fix under review: PANES-R1
Evidence class: reproduced. I ran the current `installedEdition` in node: `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-panes/flavour2.mjs`.
Location: `Catalog.tsx` `installedEdition` (the `exact` find) and `variantsCompatible`.
Trigger: the entry and the book have the same name and day, and no `_flavour:` tag or `maxi|mini|nopic` token on either side. Then either:
- the entry says `_ftindex:yes` and the book `_ftindex:no`; or
- only one side states a variant, for example the entry says `_pictures:no` and the book has no `_pictures` tag.
Causal path:
- `archive.flavour === flavour` holds because both are `null`.
- `VARIANT_KEYS` leaves out `_ftindex`, and a variant stated on one side only is skipped.
- So the result is `exact: true`, and the card and detail show Open with no Download. Both cases print `true`.
- Rust `series_compatible` compares the whole `variants` map with `==`, which includes `_ftindex` and refuses a one-sided entry. Rust says "not the same series" for both.
- The reported case (maxi entry against a renamed or nopic copy) now prints `false`, so R1 itself is fixed.
- Realistic exposure is small. Official Kiwix archives that differ in content always carry a flavour token in their file name, and OPDS tags come from the ZIM's own Tags. So the remaining case needs a third-party feed or hand-named files.

**Recommendation:** the two possible errors are not equally costly, so use a different rule for each decision.
- **`exact`**, which removes Download: use the Rust rule exactly. All four keys must be equal, and a variant stated on only one side refuses the match.
- **The related "Installed edition {date}" badge:** keep the looser rule.

Why this split:
- A wrong `exact` removes the only way to download a different archive from Discover.
- A missed `exact` only shows the related badge with Download still offered. At worst that Download fails with the existing reserved-destination error, which is exactly the base behaviour.
- The change is one extra predicate inside the `exact` `find`, plus two test lines.

### PANES-R7 — Tapping a spelling correction opens the soft keyboard on phones
Severity: Nit
Fix under review: PANES-R5
Evidence class: inference (not run on a device)
Location: `Library.tsx:1524` (`searchInput.current?.focus()`)
Trigger: on a phone, tap "Search instead for …".
Causal path: focusing a text input programmatically after a tap shows the iOS/Android keyboard, which then covers the new results.
Smallest correct fix: put `tabIndex={-1}` on the results region (it already has `aria-live`) and focus that instead, or focus the input only when `!isMobile`.

**Per-item status**
- **R1:** closed for the reported flavour case. The variant rule leaves the narrow wrong-exact case in PANES-R6; your call on matching Rust, and I recommend the Rust rule for `exact` only.
- **R2:** closed.
  - The regex requires `Z` or `±hh:mm`.
  - An offset-less timestamp gives `null`, the same result as Rust `timestamps::parse`.
  - The test passes under `TZ=Asia/Tokyo` (7/7).
- **R3:** closed.
  - Failures from `start` and `submitMapRequest` are painted inline only when `openedRef.current?.id === request.regionId`.
  - `changeMapState` failures always go to `onError`, and App puts them in the top-layer alert when a dialog is open.
  - A card's pending `map_prepare` is dropped when another area dialog opens, because `openDetail` → `dismissDecision` retires `selectionRequest`. That was already the case at base, and the user's navigation is what abandons the request, so this is acceptable.
- **R4:** closed. Suggestions are not suppressed in any case where you would expect them:
  - `searchSubmitted` is set only inside `performSearch`, after its early returns. A refused search therefore never sets it.
  - It is cleared on every `onChange`, including paste, IME composition and mobile autocorrect.
  - It stays set after Cancel, a scope or mode change, or a privacy invalidation, until the next keystroke. That is the stated design (the implementer's second doubt), and it is acceptable.
  - The focus test still passes.
- **R5:** closed.
  - Discard failures are now recorded with `onError(value, "blocking", false)`.
  - The outcome is set after `load()`, and the focus effect decides once.
  - The split-archive detection is right. `/\.zimaa$/i` on the primary path matches Rust `archive_suffix(primary, ".zimaa")`, which is also case-insensitive and is how `read_part_names` → `verified_parts` → `delete_book` select the whole `aa…` part set.
  - Operator precedence keeps the split-archive paragraph out of the provider "Stop keeping" wording, so it is added only to the delete and trash confirmations.
  - The mobile keyboard nit is PANES-R7.
- **Rollback errors in the card:** closed.
  - One owner: the in-card `role=alert` plus a record with `showGlobally: false`.
  - `rollbackEdition` now only clears `editionActionError` and never sets it. The page-top band is still used for intent actions only.
  - Focus moves to the error once the disabled button can no longer hold focus. The browser test asserts this.
  - Residual, unchanged from base: after a successful rollback, the button is replaced by "Rolled back…" and focus drops to `<body>`.

**What I ran**
- `npx --no-install tsc --noEmit` → exit 0
- `node --test scripts/test-pane-actions.mjs scripts/test-library-suggestion-focus.mjs scripts/test-error-visibility.mjs scripts/test-edition-surfaces.mjs scripts/test-download-state.mjs` → 63/63 pass
- `TZ=Asia/Tokyo node --test scripts/test-pane-actions.mjs` → 7/7 pass
- `KIWIX_TEST_CHROME=…/chrome-nosandbox.sh node --test scripts/test-pane-surfaces.mjs`, three times → 3/3 each run (24.2 s, 22.7 s, 28.8 s)
- The same harness on `test-frontend-surfaces.mjs`, `test-shell-surfaces.mjs` and `test-nearby-sharing-surfaces.mjs` → 9/9 pass
- `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-panes/flavour2.mjs`, the evidence for PANES-R6

**What I could not verify**
- The phone keyboard behaviour (PANES-R7) and the Android/iOS rendering.
- The discard failure path in a browser; no test renders it.
- A real split-archive deletion through `edition_discard_original`. I traced it by reading the Rust; I did not run cargo.