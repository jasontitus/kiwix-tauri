## Verdict for lane panes (round 3): one should-fix finding

The iOS path-redaction change (FEATURES-R13.1) fixes the map sentences, but it now shows part of the path for ordinary folder names that contain an abbreviation. I reproduced that and propose a one-line fix below. PANES-R6, PANES-R7 and FEATURES-R13.2 are closed, with small notes.

I reviewed `git diff d98cac2` of `Catalog.tsx`, `Library.tsx`, `MapBlockProgress.tsx` and `UI.tsx`, plus the untracked `scripts/test-pane-actions.mjs`. I compared them against Rust `catalog_index::series_key`/`series_compatible` and the `catalog.rs` language parsing.

### PANES-R8 — The new redaction stops early and shows the rest of the path for folders like "Dr. Who", "A. Smith", "Vol. II" or "St. Louis"
Severity: Should-fix
Fix under review: FEATURES-R13.1
Evidence class: reproduced. I ran the current and base `UI.tsx` regexes on the same strings: `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-panes/redact.mjs`.
Location: `src/components/UI.tsx:41-47` (`bareFilesystemPath`), the new terminator `\.(?:[ \t]+(?=[A-Z])|$)`

What the old and new regexes print for the same input (`<P>` is the app container path):

| Input | Base | Now |
|---|---|---|
| `Could not open <P>/Dr. Who/wikipedia.zim right now` | `Could not open [File]` | `Could not open [File]. Who/wikipedia.zim right now` |
| `The archive <P>/A. Smith/notes.zim is not available` | `The archive [File] is not available` | `The archive [File]. Smith/notes.zim is not available` |
| `Import failed for <P>/Vol. II/b.zim` | `Import failed for [File]` | `Import failed for [File]. II/b.zim` |
| An iCloud path `…/St. Louis Guide (Draft)/x.zim` | `[File])/x.zim` | `[File]. Louis Guide (Draft)/x.zim` |

Causal path:
- The `. ` + capital terminator fires on any abbreviation inside a folder name.
- The rest of the path no longer starts with a filesystem root, so the global replace never redacts it.
- Folder names like these are realistic in iCloud Drive and Files: personal names, "Vol.", "St.".

The ` (` + capital terminator is not a new class of leak. Base already stopped at `)`: "Books (Old)" printed `[File])/wikipedia.zim`, and now prints `[File] (Old)/wikipedia.zim`. It shows a little more than before.

How serious this is: the redaction is display-only. The comment says "Keep original errors/export bytes intact", so diagnostics and exports still hold the full path, and what shows is the user's own folder and file names on their own screen. It is still a visible regression for realistic names, and those names can be personal.

Smallest correct fix: let both new terminators fire only right after a file extension. Wrap them in `(?<=\.[A-Za-z0-9]{1,12})(?:…)`. I checked this in a scratch copy (`/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-panes/redact2.mjs`):
- the map list form and the `kept at ….part. Its folder…` form still redact correctly;
- "Could not open [File]. Try again." keeps the sentence;
- "Dr. Who", "A. Smith" and "Vol. II" are fully redacted again.

Add those three folder names to test 9. The more robust option is for the backend to send recovery paths in a separate field so the sentences carry no path. The UI already has `recoveryPaths` for map recovery, and the backend could use the same idea for the `kept at` sentence.

### Notes on PANES-R6 (Nit, safe direction, no change needed)
`exact` now equals Rust's rule or is stricter, with two small differences:
- **Variants.** `seriesVariants` compares raw values over the four keys. A variant stated on one side refuses the match, and an empty or conflicting value gives `null`. This matches `series_key`/`series_compatible`.
- **Flavour.** The frontend requires the flavours to be equal, which is stricter than Rust's "compatible when one side has none". That is the safe direction.
- **Language: Rust really does compare it.** It uses `left.language == right.language`, normalised only by `trim().to_ascii_lowercase()`, with no `en`→`eng` mapping. The frontend's `trim().toLowerCase()` gives the same result for ASCII.
- **Difference 1, empty language.** Rust's `series_key` returns `None` when the language is empty. The frontend lets two empty languages match exactly. This is unlikely in real catalogs.
- **Difference 2, language fallback.** When an OPDS entry has no `<language>`, `catalog.rs:879-885` falls back to `xml:lang` or the feed's language, which can be a two-letter code. That code never equals a ZIM's three-letter `Language`, so `exact` is withheld and Download stays offered. That is the safe direction. Official Kiwix entries carry `<language>eng</language>`, the same value as the ZIM.

### Notes on PANES-R7 focus (closed)
I traced the effect, keyed on `[results, busy]`:
- **Success:** the click sets the flag. `setBusy(true)` and `setResults(null)` happen in one render, and the effect returns while `busy`. `setResults(found)` and `setBusy(false)` are batched after the await, so the effect then focuses the results region once and clears the flag.
- **Failure:** the effect runs with `busy` false and no region rendered. It clears the flag and focuses nothing; the shell's band shows the error.
- **Rapid taps:** the button is disabled while `busy` and unmounts with the old results. A second call in the same frame supersedes the first search, and the single flag still yields a single focus move.
- **Nit:** if `performSearch` returns early (no privacy context or request), the flag stays set. A later search that settles would then move focus to the results without a tap. The fix is to clear the flag in `searchCorrection` when `performSearch` does not start, or to have `performSearch` return whether it started.

### Note on FEATURES-R13.2 (closed)
The reassurance is still in the accessible name: the button reads "Dismiss (keeps the file)", and the note is in its `aria-describedby`. Dropping the sentence from the note loses nothing.

Minor wording nit: "Press Delete again" assumes a Delete already happened. Recovery records can also come from `start()` on an earlier job, and Delete is not offered while a job is active. Consider "Press Delete to retry…".

**Per-item status**
- **PANES-R6:** closed. The two small differences above are in the safe direction, apart from empty languages.
- **PANES-R7:** closed. The lingering-flag edge is a nit.
- **FEATURES-R13.1:** not closed, because the new terminators leave parts of realistic paths unredacted (PANES-R8).
- **FEATURES-R13.2:** closed.

**What I ran**
- `npx --no-install tsc --noEmit` → exit 0
- `node --test scripts/test-pane-actions.mjs scripts/test-library-suggestion-focus.mjs scripts/test-error-visibility.mjs scripts/test-edition-surfaces.mjs scripts/test-download-state.mjs` → 65/65 pass
- `TZ=Asia/Tokyo node --test scripts/test-pane-actions.mjs` → 9/9 pass
- The other node tests that use `errorText` (`test-collections-async`, `test-mobile-voice-search`, `test-provider-recovery`, `test-settings-surface`) → 51/51 pass
- `KIWIX_TEST_CHROME=…/chrome-nosandbox.sh node --test scripts/test-pane-surfaces.mjs`, three times → 3/3 each run (27.6 s, 26.7 s, 46.0 s)
- Same harness, `test-frontend-surfaces.mjs` and `test-shell-surfaces.mjs` → 6/6 pass
- The redaction comparisons in `redact.mjs` and `redact2.mjs` (paths above)

**What I could not verify:** the iOS rendering and the phone soft-keyboard behaviour. The iOS case was checked only by running the regex.