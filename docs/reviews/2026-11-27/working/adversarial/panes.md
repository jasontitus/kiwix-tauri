## Verdict for lane panes: only non-blocking findings

Review base was `d98cac2`. I reviewed `git diff d98cac2` of Library, Downloads, Catalog, Maps, MapBlockProgress and SelectedStorageActions, plus the untracked `scripts/test-pane-actions.mjs` and `scripts/test-pane-surfaces.mjs`.

I read these backend handlers in the current tree:
- `map_forget_recovery`, `map_delete_download` and `map_status` (dispatch.rs:2560-2625, `maps/manager.rs` `status`/`delete_download`/`forget_recovery`, `MapJob.recovery_paths`/`speed`).
- `edition_discard_original` (edition_service.rs:268-298, edition_updates.rs `discard_original`/`remove_original`/`discardable_original`/`rollback_edition`, `library_files::delete_book_impl`).
- `torrent_job` / `run_share` / `apply_worker_error` / `restore_record`.
- `catalog_index::edition_date` / `series_compatible`, and `diagnostics.reportError`.

`impl/features-round3.md` does not exist yet, so the delete-reply contract was checked against round 2.

### PANES-R1 — Catalog hides Download and offers "Open" for a different flavour when the installed copy's flavour is unknown
Severity: Should-fix
Fix under review: ACQ-7
Evidence class: reproduced (I ran the real `installedEdition` in node: `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-panes/flavour.mjs`)
Location: `src/components/Catalog.tsx:131-132` (`installedEdition`)
Trigger:
- The library holds `wikipedia_en_all`, Date 2026-10-16, in a file without the flavour token. For example it was renamed `wikipedia.zim`, or it is a provider document whose display name has no token.
- Real ZIM Tags carry `_pictures:no` but no `_flavour:` (`archive_metadata.rs` reads only `Tags`).
- Open Discover on the maxi entry of the same day.
Causal path:
- `archiveFlavour` gives `null` for the book.
- `.filter(archive => !flavour || !archive.flavour || archive.flavour === flavour)` keeps it.
- The days match, so `exact: true`. The card and the detail dialog show "Installed" and "Open" (which opens the nopic file), and the Download/BitTorrent buttons are not rendered (lines 705, 729, 808-857).
- Reproduced: `renamed nopic install vs maxi entry -> true`.
- At base `exact` was never true for the official catalog, so this hiding is new to the official feed. ACQ-7's own suggested fix required flavour equality. The Rust `series_compatible` that the implementer mirrors also requires `variants` (`_pictures`/`_videos`/`_details`) to be equal, which the JS ignores. Here, the book's `_pictures:no` would have told the two apart.
Smallest correct fix: only set `exact` when the flavours are equal or both unknown. A known-vs-unknown match should give the non-exact badge and keep Download. Also apply the `_pictures/_videos/_details` equality when both sides state them, as `series_compatible` does. Add that case to `test-pane-actions.mjs`.

### PANES-R2 — `editionDay` accepts timestamps with no offset and reads them in local time; Rust rejects them
Severity: Nit
Fix under review: ACQ-7
Evidence class: reproduced
Location: `Catalog.tsx:97-103` (`editionDay`)
Trigger: a third-party OPDS feed with `issued` = `2026-10-16T00:30:00` (no `Z` or offset).
Causal path:
- `Date.parse` treats a form with no offset as local time. Under `TZ=Asia/Tokyo` it gives `2026-10-15`.
- `catalog_index::edition_date` → `timestamps::parse` (strict RFC 3339) gives `None`.
- So the Exact/Installed result depends on the time zone and differs from Rust.
- The official feed uses `Z`, so it is not affected.
Smallest correct fix: require `/(Z|[+-]\d{2}:\d{2})$/i` before `Date.parse`, otherwise return `null`.

### PANES-R3 — A Maps failure from a card or the job panel is painted inside an unrelated area dialog opened in the meantime
Severity: Nit (the inline path already behaved this way at base; the implementer listed it as a doubt)
Fix under review: SHELL-3
Evidence class: source-traced
Location: `Maps.tsx:397-403` (`reportStartError`: `if (!openedRef.current) { onError(e); return; }`)
Trigger:
- Tap "Get map" on card A. `map_prepare` fetches a recipe and can take seconds.
- Meanwhile tap card B's title (`openDetail` is not gated on `working`).
- `map_prepare` fails.
- The same happens for a pending job-panel Pause or Resume (`changeMapState`).
Causal path: any open area dialog takes the error. It renders at `:1210` above B's "Get map" (which reads "Starting…" while `working` is set), so the failure reads as B's. Nothing is silent, and nothing is lost or hidden.
Smallest correct fix:
- Give `reportStartError` the request's `regionId`, and paint inline only when `openedRef.current?.id === regionId`.
- Always route `changeMapState` failures to `onError`. App then raises them in the top-layer alert, because a dialog is open.
- Catalog already does this correctly, by comparing `detailRef.current?.id !== entry.id`.

### PANES-R4 — Library search: a failed search now pops the suggestion list
Severity: Nit
Fix under review: SERVE-2
Evidence class: source-traced
Location: `Library.tsx:316` (`searchFocused = searchInputFocused && !busy && !results`) and the effect at `:398-420`
Trigger: type a query and press Enter, and the search rejects (for example an archive error). The input keeps focus.
Causal path:
- `busy` goes true→false while `results` stays `null`, so `searchFocused` flips false→true.
- The effect's dependencies change, it schedules `loadSuggestions`, and the list opens under the global error band.
- At base, Enter never re-armed suggestions.
- A zero-hit search sets `results` to an object, so it is not affected.
Smallest correct fix: none needed if this is acceptable. Otherwise drop the focus flag when a search settles, or gate on a "searched since last keystroke" ref.

### PANES-R5 — Small accessibility and copy gaps in the new controls
Severity: Nit
Fix under review: SERVE-5, Edition originals UI, Maps UI
Evidence class: source-traced
- **Spelling corrections (`Library.tsx:1512-1523`).** Clicking a "Search instead for…" button runs `performSearch`, which calls `setResults(null)`. That unmounts the focused button, so keyboard focus drops to `<body>`. Fix: `searchInput.focus()` (or focus the results region) before searching.
- **Edition discard (`Downloads.tsx:1011-1066`).**
  - When it fails, the outcome is shown as `role=alert` but is never recorded in diagnostics. Every other inline owner calls `onError(e, "blocking", false)`. Add that call.
  - A failure that `load()` shows as already-discarded (for example "already deleted") also removes the button without moving focus, because `originalOutcomeFocus` is false on that path.
  - The confirmation names only `original.path`. For a split archive, `delete_book` removes the whole `.zimaa…` part set.
- **Discard button visibility.** The button is offered when `discardable_original` would refuse because the original is in the library again. The reply is an actionable error, so this is acceptable.
- **Map recovery paths are listed twice.** The `job.error` warning, shown as `role=alert`, already lists every kept path in full ("…The files were kept here: …"), and `MapRecoveryPaths` lists them again. On iOS the first copy is the full sandbox path, while the list deliberately shows only the file name.

**Per-fix assessment**
- **SHELL-3:** fixes it. Catalog routes failures to the inline notice by entry. Maps works, with the misattribution in R3. The global path is always wired: every pane is mounted only from `App.tsx` with `onError`, and `reportError` drops `UserCancelledError`/`SuspendedError`.
- **SHELL-5:** fixes it. The poll ends on settle (magnet, error, completed, paused or cancelled). It is retired by:
  - dialog close (`shareRequest++`);
  - unmount (`alive`), since panes unmount on switch;
  - `privateMode`/`readOnly` change (the effect at :171 closes the dialog);
  - a privacy-generation change (it returns);
  - a backend error, or a vanished job (`torrent_job` → `record()` returns `Err` and never null, so the catch fires).

  Stuck in `creating` forever: one cheap IPC every 750 ms only while the modal is open, and the user can close it. That is acceptable. Pre-existing residual: a privacy-generation bump with no mode change returns silently and leaves the spinner in the still-open dialog. A share restored as `queued` (network policy) spins under "Checking the shared item…".
- **SERVE-2:** fixes it. The guard is retired in every path, because `closeSuggestions` bumps `suggestionId` synchronously on each one: Enter or button (`runSearch` → `performSearch`), voice (`performSearch`), suggestion click or Enter (`openSuggestion`), correction click, blur. `busy` also blocks the refocus case. One side effect is R4, plus the accepted change that refocusing over results does not show suggestions.
- **SERVE-5:** fixes it. `corrections[].correctedQuery` matches dispatch.rs:773-821, which de-duplicates. The group is labelled "Spelling suggestions" (focus nit in R5).
- **ACQ-7:** fixes it but the date fix is right only for RFC 3339 with an offset (R2), and the flavour rule is too loose for hiding Download (R1).
- **ACQ-8:** fixes it. `kept` and `live` cannot both be set. A paused row with no error still uses the existing live block. Nothing is drawn twice.
- **ANDROID-6:** fixes it. `move`, the checkbox, the hint, the button label and `relocate`'s `moveOriginals` all derive from the same `books` of the same render (the onClick closure). They cannot disagree, and the earlier choice comes back only when the selection becomes all-managed. `aria-describedby` points at an element that is always rendered. I did not render it on Android.
- **SHELL-R1 / shell handoff 2:** fixes it. Every site listed now has one owner: Library removal and share, Downloads cleanup/`start`/`continueMap`, and the Maps cleanup dialog. I grep'd the six files for any remaining `setXError(errorText(e)); onError(e)` pair and found none.
- **Maps UI contract:** matches.
  - `map_forget_recovery {path, privacyGeneration}` takes a path captured at call time (`capturePrivacyContext()` at click). It requires `normal_writable`, `ensure_window_active` and `ensure_generation`.
  - The path string goes back exactly as `recovery_paths` gives it, including `.streetzim-cleanup-<uuid>/`. Only the display uses `displayPath`/`pathFileName`.
  - Delete: an `Ok` reply re-reads `map_status`. An `Err` reply ("kept at…") stays in the dialog, and `noteJobUpdate` shows the listed job.
  - `speed` is shown only when it is above 0 and the job is neither terminal nor paused.
- **Edition originals UI:** matches the backend.
  - The arguments `transactionId` and `privacyGeneration` are checked as current after the confirmation.
  - The wording follows the backend's branches: `sourceReference` gives "Stop keeping" (External); otherwise `trashDeletedFiles` gives trash or permanent. The backend reads the same setting at dispatch time.
  - Mobile with trash enabled gets the backend's actionable error, the same as Library removal.
  - Rollback is hidden exactly when `originalDiscarded` or rolled back. The replacement-missing case is disabled.
  - Focus moves to the outcome after the button unmounts. Only the "deleted" branch is rendered in a test.

**Implementer's doubts**
- **SERVE-2, no suggestions over results:** acceptable as a UX choice. See R4 for the related failed-search case.
- **SHELL-3, error follows the open dialog:** real but minor and pre-existing (R3). It is confusing rather than harmful; the fix is small.
- **ACQ-7 filename heuristic:** the heuristic is fine. The problem is the "unknown flavour = compatible" rule applied to `exact`, not the heuristic (R1, should fix now).
- **Rollback errors in the off-screen page-top band:** pre-existing. That is a SHELL-3-class issue outside this lane's claims, so it needs an owner decision.
- **`continueMap` "Download settings changed" vanishing:** pre-existing and acceptable for now.
- **Maps Delete leaves a stale card briefly:** acceptable. The restarted poll reads the status as well.

**What I ran**
- `npx --no-install tsc --noEmit` → exit 0.
- `node --test scripts/test-pane-actions.mjs scripts/test-library-suggestion-focus.mjs scripts/test-error-visibility.mjs scripts/test-edition-surfaces.mjs scripts/test-download-state.mjs` → tests 62, pass 62, fail 0.
- `KIWIX_TEST_CHROME=…/chrome-nosandbox.sh node --test scripts/test-pane-surfaces.mjs`, three times → each run 3/3 passed (25.9 s, 21.7 s, 30.6 s).
- The same harness on `test-edition-surfaces.mjs test-frontend-surfaces.mjs test-nearby-sharing-surfaces.mjs test-shell-surfaces.mjs` → tests 19, pass 19, fail 0.
- A scratch reproduction of `installedEdition`/`editionDay`: `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-panes/flavour.mjs` (with `TZ=Asia/Tokyo` and `TZ=America/New_York`).

The new tests extract and run the production declarations; they do not test mocks. Reading them, the SERVE-2 node test fails without the `suggestionId` guard, the share tests fail with the cap or the second owner restored, and the relocate test fails without the re-derivation. I did not mutate the tree to confirm the implementer's base-failure runs.

**What I could not verify**
- Android rendering and TalkBack for the Move checkbox; iOS file-name display.
- Phone viewport for the SHELL-3 band.
- The trashed, missing and external edition outcomes, which no test renders.
- A real Maps Delete that returns `Err` with a `.streetzim-cleanup-<uuid>/` path, a real long torrent hash, and the `map_status`/`edition_*` Rust tests (I traced the contracts by reading them, not by running cargo).
