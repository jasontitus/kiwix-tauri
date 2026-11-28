# Lane `panes` — report

All 14 items in this lane are addressed in files I own. There are no handoffs. `tsc` is clean, and the full node suite has only the two known regeneration failures. My new browser test passed 3 out of 3 runs, each 3/3 tests (about 23–27 s per run). Every new behaviour test was seen failing, either on base sources or with the fix reverted in a scratch mirror. That mirror is under `scratchpad/panes/` and the shared tree was never mutated. Nothing ran on a device. ANDROID-6 is `isAndroid`-only, so it was exercised only by running the `relocate` code in node, not by rendering the Android UI.

To see the full change: `git diff d98cac2 -- src/components/{Library,Downloads,Catalog,Maps,MapBlockProgress,SelectedStorageActions}.tsx`. The two new tests are untracked additions, so `git diff` does not show them: `scripts/test-pane-actions.mjs` and `scripts/test-pane-surfaces.mjs`.

---

### SHELL-3 — card-level failures painted off-screen at the top of the scrolled page
Disposition: fixed (routed to the shell's surface)
Verification of the finding: confirmed in Catalog (`{!detail && actionNotice}` above the header) and in Maps (`{!openedRegion && startError}`). The band is reproduced as missing on base: the browser test times out waiting for it.
Change:
- `Catalog.tsx`:
  - `reportActionError` paints inline only while the detail dialog for that same entry is open (`detailRef`).
  - Otherwise it calls `onError(error)`. App puts that in the always-visible `.global-error` band, or in the top-layer alert if some dialog is open.
  - The page-top notice is removed.
- `Maps.tsx`:
  - New `reportStartError`, used by `submitMapRequest`, `start` and `changeMapState`.
  - It paints inline only while the area dialog is open; otherwise it calls `onError(e)`.
  - The page-top notice is removed.
- Declined low-storage prompts are still silent, because `reportError` and `App.onError` ignore `UserCancelledError`.

Tests: `test-pane-surfaces.mjs` "a card failure far down Discover or Maps is shown where it can be seen" uses 40 cards and scrolls `#main-content` to the bottom. For Catalog, clicking the last card's Download shows the band inside the viewport, painted exactly once, with no alertdialog, and the page stays scrolled. Maps "Get map" behaves the same. Observed failing on base: yes, it times out waiting for the band.
Not exercised: phone viewport.

### SHELL-5 — BitTorrent share reported failure after 180 s
Disposition: fixed
Verification of the finding: the cap of 240 × 750 ms is confirmed in `Library.shareBook`.
Change: `Library.tsx`:
- New `shareSettled(job)` (has a magnet, or status is error/completed/paused/cancelled).
- The poll is now `while (!shareSettled(job))`. It is still retired by `alive`, by `shareRequest` (bumped when the dialog closes) and by a privacy change.
- "Not created" is shown only for a settled job with no magnet.
- The spinner renders only while the job is unsettled and there is no `shareError`.
- Also fixed a double owner found here: a `torrent_job` failure after the share dialog opened used to paint inline AND call `onError`. It now uses `onError(e, "blocking", false)`.

Tests: `test-pane-actions.mjs`, all running the real `shareBook`:
- 300 `creating` polls, then a magnet: no error.
- `error` status: the error is shown, the job counts as settled, and no shell report.
- A failing poll: one inline copy, recorded with `showGlobally: false`.

Observed failing: yes, with the 240 cap restored, and with the shell report restored.

### SERVE-2 — Enter within 120 ms let the suggestion timer cancel the search
Disposition: fixed (differently from the suggested `busyRef`). `scripts/test-library-suggestion-focus.mjs`, which I do not own, runs these exact functions in a vm context with fixed bindings. Any new identifier in them would break it, so I used existing names and a derived value.
Change: `Library.tsx`:
1. The debounce callback captures `suggestionId.current` and skips if it has changed. `performSearch` → `closeSuggestions` bumps it synchronously, so a pending timer is retired before React re-renders.
2. The focus state is renamed to `searchInputFocused`, with `const searchFocused = searchInputFocused && !busy && !results;`.
   - No suggestion is scheduled while a search or "Show more results" is running. That covers the refocus case.
   - No suggestion opens over fresh results. Typing clears the results and brings suggestions back.
   - Side effect: refocusing the field while results are shown no longer pops suggestions until you type.

Tests:
- `test-pane-actions.mjs` "Enter before the suggestion debounce elapses…" runs the real effect, `closeSuggestions`, `loadSuggestions` and `performSearch`.
- Browser test: type, then Enter, with search delayed 600 ms. It asserts no `suggest` or `cancel_search` after the `search` call, and no error surface. With the fix reverted it saw `['cancel_search','suggest']`.
- The existing `test-library-suggestion-focus.mjs` still passes.

### SERVE-5 — spelling corrections never shown
Disposition: fixed (corrections only; I did not add the optional spelling-index pointer)
Change: `Library.tsx` zero-hit branch. Each correction is a button reusing "Search instead for “{query}”" inside `role="group"`, labelled with the existing string "Spelling suggestions". The button does `setSearch(...)` then `performSearch(...)`.
Tests: browser test — clicking the button issues `search` with "animal" and updates the input. Not separately reverted; base has no such button.

### ACQ-7 — Catalog "installed" detection
Disposition: fixed
Change: `Catalog.tsx`:
- `editionDay` takes `YYYY-MM-DD` as is and converts an RFC 3339 timestamp to its UTC day, as Rust `edition_date` does.
- `archiveFlavour` reads the `_flavour:` tag, otherwise a `maxi|mini|nopic` token in the name or file name. For a book that is `name` plus the basename of the path or provider display name; for an entry, `name` plus the URL basename.
- A known, different flavour is a different archive. A flavour unknown on either side does not separate them, mirroring `series_compatible`. A strict match would have hidden every badge, because most ZIMs' `Tags` carry no `_flavour:`.
- The per-library index precomputes flavour and day.

Tests: `test-pane-actions.mjs` covers RFC 3339 vs date, a UTC offset that crosses midnight, a different day, nopic by tag and by file name, an unknown flavour, and a provider display name. Observed failing with either half reverted.

### ACQ-8 — interrupted/failed rows hide progress
Disposition: fixed
Change: `Downloads.tsx` — new `kept` = no live summary, not ready, no import error, status paused/failed/error, and `downloaded > 0`. It renders a static bar (only when the total is known) plus "{downloaded} of {total}". There is no rate or ETA.
Tests: browser test — a paused row with the interruption error shows "32.0 MB of 40.0 MB" and the bar reads 0.8. Observed failing on base: yes.

### ANDROID-6 — Move can never succeed for provider-backed archives
Disposition: fixed
Change: `SelectedStorageActions.tsx`:
- When any selected book has `sourceReference`, the Move checkbox is disabled and unchecked, with an explanation linked by `aria-describedby`.
- `relocate` re-derives `removeOld && !books.some(sourceReference)` at call time, because the selection can change while the dialog is open.
- The old "Provider originals are removed only when…" hint is replaced.

Tests: `test-pane-actions.mjs` — a mixed selection with Move chosen sends `removeOld:false` and the "copied" notice; a managed-only selection still moves. Observed failing on base: yes.
Not exercised: Android rendering and TalkBack.

### Double error owners (SHELL-R1) and Shell handoff 2
Disposition: fixed. The current signature is confirmed: `SurfaceProps.onError(error, severity?, showGlobally?)`.
Change:
- `Library.tsx`: new `removalFailed(error, shownInline)`.
  - The removal dialog always paints inline.
  - `remove()` and the privacy-expired path paint inline only when the detail dialog for that installation is open.
  - From a swipe with no dialog, `onError(e)` is the one owner. Painting inline there would have been invisible.
- `Downloads.tsx`:
  - Cleanup dialog: `onError(value, "blocking", false)`.
  - `continueMap`: `onError(value)` only. I dropped `setMapError`: that page-top notice is wiped by the next successful status poll and sits above a card that may be scrolled away.
  - `start()`: `onError(e, "blocking", false)`, then `if (!isUserCancelledError(e)) setStartError(...)`.
  - The `onError` prop type is widened.
- `Maps.tsx` cleanup dialog: `onError(e, "blocking", false)`.

Tests: browser test — a failing `remove_book` from the detail dialog is painted exactly once with no alertdialog (base: two copies plus an alertdialog, reproduced). A failing `map_delete_download` in the Downloads cleanup dialog is also painted once. The fixed sleeps were removed: the two state updates commit together, so the counts are checked as soon as the inline copy appears.

### Maps UI (features contract)
Disposition: fixed
Change:
- (a) New `MapRecoveryPaths` in `MapBlockProgress.tsx`, used by Maps and by the Downloads map card.
  - Each kept path is listed: the display path, or the file name on iOS.
  - Each has a "Dismiss (keeps the file)" button whose `aria-describedby` points to the path and an explanatory note.
  - The button calls `map_forget_recovery { path, privacyGeneration }` (`forgetRecovery` / `forgetMapRecovery`), then re-reads the status.
- (b) After `map_delete_download` succeeds, Maps re-reads `map_status`, guarded by job revision and privacy, instead of `setJob(null)`. If the read fails, the restarted poll reports it. On an error (including the "kept at …" message) the error is shown in the dialog, and `noteJobUpdate()` makes the card show the cancelled job with its paths.
- (c) New `MapTransferRate`: "{speed}/s" when `speed > 0` and the job is neither terminal nor paused.
- (d) The copy now reads "Delete this download, then get the map again."

Tests: browser test — Delete returns Ok while the status still lists the job with `recoveryPaths`; Dismiss appears with the path in its description and sends `{path, privacyGeneration:1}`; the list then disappears.
Honest note on (b): base already called `noteJobUpdate()`, so the poll restores the job. With the re-read reverted the test still passes. The real gain is that the job no longer vanishes and reappears; the test does not detect that flicker.

### Edition originals UI
Disposition: fixed. `privacyGeneration` is passed explicitly, so `api.ts` is not edited.
Change: `Downloads.tsx` new `discardOriginal`:
- The confirmation names the title and the file: the full display path for a local original (an original outside Kiwix's storage is deleted too), the provider display name otherwise.
- Three confirmation wordings: permanent delete, system trash (following `settings.trashDeletedFiles`, which is what the backend uses), and a provider original ("Stop keeping…", Kiwix cannot delete it).
- Rollback and Delete are hidden once `originalDiscarded`, and a static line says why.
- Each of the four `originalFile` outcomes has its own notice, shown in that transaction's card. Errors show there as an alert.
- Focus moves to the outcome when the pressed button disappears.

Tests: browser test — Escape on the confirmation sends nothing; confirming sends `{transactionId, privacyGeneration:1}`, focus lands on the outcome, no buttons remain, and both status lines are shown. The "trashed", "missing" and "external" branches are not rendered by any test.

### Networking contracts
Nothing to change: I found no affected consumer in my files.

---

## Commands run
- `npx --no-install tsc --noEmit` → exit 0
- `node --test scripts/test-*.mjs` → `tests 528, pass 512, fail 2, skipped 14`. The failures are the known `docs/ERRORS.md` and localization-manifest gates. My 3 browser tests are among the skips when no browser is set.
- `node --test scripts/test-pane-actions.mjs test-library-suggestion-focus.mjs test-error-visibility.mjs test-edition-surfaces.mjs test-download-state.mjs` → `tests 62, pass 62`
- `KIWIX_TEST_CHROME=…/chrome-nosandbox.sh node --test scripts/test-pane-surfaces.mjs` ×3 → `tests 3, pass 3, fail 0` each
- Same harness: `test-frontend-surfaces.mjs test-shell-surfaces.mjs test-nearby-sharing-surfaces.mjs` → 9/9 ok
- Base/mutation runs in the scratch mirror: on base, all 3 browser tests and all 6 node tests fail. With only the SERVE-2, ACQ-7, SHELL-5 or double-owner fix reverted, the matching test fails.
- Not run: `cargo`, `npm run build` (not needed: the tests use the Vite dev server).

## Files changed
- `src/components/Library.tsx`
- `src/components/Downloads.tsx`
- `src/components/Catalog.tsx`
- `src/components/Maps.tsx`
- `src/components/MapBlockProgress.tsx`
- `src/components/SelectedStorageActions.tsx`
- New: `scripts/test-pane-actions.mjs` (node), `scripts/test-pane-surfaces.mjs` (browser)

Not touched: `api.ts`, `types.ts`, `styles.css`.

## New user-visible strings (need localization regeneration)
1. "This download cannot be resumed. Delete this download, then get the map again." (replaces "…Get the map again to start a new download.")
2. "Kiwix kept these files because it could not prove they were its own to delete. Remove them yourself if you no longer need them. Dismissing a warning never deletes the file."
3. "Dismiss (keeps the file)"
4. "Archives opened from another app or storage provider can only be copied: Kiwix has no permission to remove the original there. After copying, delete the original in that app if you no longer need it."
5. "The originals are removed only after the managed copies are verified." (the provider-permission hint is no longer used)
6. "Stop keeping the retained original “{title}” ({file})? Kiwix cannot delete it: the file stays where it is, in the app or storage that provides it. This edition can then no longer be rolled back."
7. "Move the retained original “{title}” ({file}) to the system trash? This edition can then no longer be rolled back."
8. "Permanently delete the retained original “{title}” ({file})? This cannot be undone, and this edition can then no longer be rolled back."
9. "Stop keeping original"
10. "Delete retained original"
11. "The retained original was deleted. This edition can no longer be rolled back."
12. "The retained original was moved to the system trash. This edition can no longer be rolled back."
13. "The retained original was already gone, so nothing was deleted. This edition can no longer be rolled back."
14. "Kiwix no longer keeps the retained original. Its file was not deleted; remove it in the app or storage that provides it if you no longer need it."
15. "The retained original is no longer kept, so this edition cannot be rolled back."

Reused strings: "{speed}/s", "Search instead for “{query}”", "Spelling suggestions", "Progress", "{downloaded} of {total}".

## Handoffs / contract changes
- None required.
- `Downloads` `onError` prop now takes `(error, severity?, showGlobally?)`, which App already passes.

## Doubts
- **SERVE-2: no suggestions while results are shown.** A deliberate UX change: refocusing the field over results no longer pops suggestions until you type.
- **SHELL-3: the error follows whatever dialog is open at failure time.** If someone opens an area dialog while a job-panel Pause is pending, that failure is shown inside the unrelated dialog.
- **ACQ-7 reads flavour from file names.** The filename fallback is a heuristic (`maxi|mini|nopic` only). Other flavours are matched only by tag; if unknown, they stay compatible, as before.
- **Discard: off-screen top band for rollback errors.** Discard errors are shown in the transaction card, but rollback errors still go to the page-top `editionActionError`, which can be scrolled out of view. That is pre-existing and was not changed.
- **`continueMap`: "Download settings changed" can vanish.** The `setMapError(t("Download settings changed…"))` messages there are also cleared by the next status poll. That is pre-existing and was left alone.
- **Maps Delete: brief stale card.** After a successful delete the dialog closes only once the `map_status` re-read returns. If that read fails, the card stays stale until the poll answers.