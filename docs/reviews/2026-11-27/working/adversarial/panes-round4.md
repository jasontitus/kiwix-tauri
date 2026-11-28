## Verdict for lane panes (round 4): only non-blocking findings, both with a proven fix

PANES-R8 is not yet closed. The lookbehind should be replaced with an equivalent that I checked is identical to it. One extension-gated terminator still lets part of a realistic folder name through.

I re-reviewed `git diff d98cac2` of `UI.tsx` (`bareFilesystemPath`), `Library.tsx` (`searchCorrection`) and `MapBlockProgress.tsx`, plus `scripts/test-pane-actions.mjs`.

### PANES-R9 — The new lookbehind is the only thing that could take the whole UI down on an old engine; an identical form without it exists
Severity: Should-fix. The failure would be catastrophic, but the platform analysis below gives it low probability, and the fix costs nothing.
Fix under review: PANES-R8
Evidence class: reproduced for the equivalence; inference for WebKitGTK 2.40.
Location: `src/components/UI.tsx:41-50`

The regex is built with `new RegExp(string)` at module evaluation. An engine without lookbehind throws a `SyntaxError` when `UI.tsx` loads, and every pane imports `UI.tsx`. Vite (`build.target: 'es2022'`) does not rewrite regexes. Platform by platform:
- **iOS:** the deployment target is 16.4 (`tauri.ios.conf.json`, `project.yml`). Safari/WKWebView 16.4 supports lookbehind. OK.
- **macOS:** `minimumSystemVersion` is 13.3, which ships Safari 16.4, and the system WebKit can only be newer. OK.
- **Android:** `minSdk = 24`, and the app checks no WebView version. The bundle already needs optional chaining and other ES2020+ syntax at parse time (Chromium 80 or later), and uses `.at(-…)` and `Object.hasOwn` (Chromium 92/93). Lookbehind arrived in Chromium 62, so it adds no new floor.
- **Windows:** evergreen WebView2 is Chromium-based. OK.
- **Linux, WebKitGTK 2.40 (`docs/PACKAGING.md:17`):** I could not verify this. 2.40 shipped in the same cycle as Safari 16.4, so it very likely includes JSC lookbehind, but I cannot run 2.40 here. The container has WebKitGTK/JSC 2.52.6, which runs the current regex correctly.
- **Nothing else needs 16.4.** I searched `src/` for `toSorted`/`toReversed`/`toSpliced`, `groupBy`, `withResolvers`, `Array.fromAsync`, the regex `v` flag and other lookbehinds. None is used. `Promise.withResolvers` is avoided on purpose, with comments.

Smallest correct fix: replace the lookbehind gate with "consume the extension, then look ahead". Exactly, replace `(?<=\.[A-Za-z0-9]{1,12})(?:A|B)` with `\.[A-Za-z0-9]{1,12}(?=A|B)`, as its own branch of the end group:

```
(^|[\s(=:\[])ROOT(?:[^\r\n:;,"'\x60)\]]|\)(?=[\\/]))*?(?:(?=[:;,"'\x60\]\r\n]|\)(?![\\/])|[ \t]+(?:is|was|…|failed)\b|$)|\.[A-Za-z0-9]{1,12}(?=[ \t]+\((?=[A-Z][a-z])|\.(?:[ \t]+(?=[A-Z])|$)))
```

- The prefix group and the replace callback do not change, because the extension stays inside the match.
- I compared it with the current regex on 3021 inputs: the named cases plus seeded fuzz over folder and abbreviation atoms. It gave 0 differences in node (V8) and 0 in JSC 2.52. I ran JSC through a small C harness on `libjavascriptcoregtk-4.1`.
- The current regex also gave the same output in V8 and JSC on all of them.
- Scripts: `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-panes/gen.mjs`, `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-panes/mk.mjs`, `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-panes/cmp.js`, `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-panes/jsc`.
- Then delete the comment "Lookbehind needs Safari 16.4".

### PANES-R10 — The extension gate still fires on abbreviation-like folder names ("U.S. History", "J.R.R. Tolkien", "Notes.v2 (Draft)")
Severity: Should-fix. Display only; exports keep the raw text.
Fix under review: PANES-R8
Evidence class: reproduced with the shipped regex (`/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-panes/mk2.mjs`)
Location: `UI.tsx` `bareFilesystemPath`, the `\.[A-Za-z0-9]{1,12}` gate
Trigger and output for paths under the iOS app container:

| Folder in the path | Base | Now |
|---|---|---|
| `…/U.S. History/x.zim` | `[File]` | `[File]. History/x.zim now` |
| `…/J.R.R. Tolkien/x.zim` | `[File]` | `[File]. Tolkien/x.zim now` |
| `…/Notes.v2 (Draft)/wiki.zim`, and likewise `Mr. X.files (Old)`, `v1.2 (Old)`, `Report.Final (Draft)` | `[File])/wiki.zim` | `[File] (Draft)/wiki.zim` |

Causal path: `.S`, `.R`, `.v2` and `.files` all satisfy "any 1–12 letters or digits after a dot". The rest of the path has no filesystem root, so the global replace never redacts it. The first two rows are new leaks of realistic names; the third shows a little more than base.

Smallest correct fix: gate on the extensions these sentences actually carry. The recovery names are `.streetzim-<uuid>.part` (`partials.rs:139`) and `.zim`/`.zimaa` publications. Use `\.(?:part|zim(?:[a-z]{2})?)` in place of `\.[A-Za-z0-9]{1,12}`, in the lookbehind-free form above. I checked it:
- every row above is fully redacted again;
- both map sentence forms and "Could not open [File]. Try again." still work.

The only cost: a sentence ending in another extension (for example `.pdf. Try again.`) loses its tail, as it did at base. Add "U.S. History" and "Notes.v2 (Draft)" to test 9.

### Other shapes I tried
- **Nested parentheses:** `Books ((Old) copy)/` gives `[File]) copy)/wiki.zim`, and `Books (Old (2))/` gives `[File]))/wiki.zim`. The `)` before a space or another `)` ends the path. Base was the same or slightly worse. Accept.
- **A path ending in ")":** `…/Books (Old)` at the end of the text gives `[File])`; `… (Old). Try again.` gives `[File]). Try again.`; `wiki (English).zim. Try again.` gives `[File]).zim. Try again.`. Cosmetic: no folder name shows. Accept.
- **"(1)/":** fully redacted.
- **`Mr. X.files/wiki.zim`:** fully redacted.
- **`(see …/Books (Old)/wiki.zim)`:** gives `(see [File])`, which is correct.

### `searchCorrection` id check: closed
`performSearch` increments `searchId.current` before its first `await`. Its only early returns (an empty query, `snippetSaving`, no privacy context or request) all come before that increment. So `searchId.current === before` holds exactly when no search started, and nothing else changes `searchId` in between.

A search that started but was then superseded or retired clears the flag through the effect: `busy` goes false, and with no results region there is nothing to focus. Rapid taps are covered by the round-3 analysis.

**Per-item status**
- **PANES-R8:** not closed. Two edits remain: swap the lookbehind for the identical lookahead form (PANES-R9) and narrow the extension gate (PANES-R10).
- **The `)`-before-separator rule:** closed. "Books (Old)/…" and "St. Louis Guide (Draft)/…" are now fully redacted, and `(see [File])` still works.
- **Focus nit (`searchCorrection`):** closed.
- **Recovery-note wording:** closed. It now reads "Press Delete to retry…".

**What I ran**
- `npx --no-install tsc --noEmit` → exit 0
- `node --test scripts/test-pane-actions.mjs scripts/test-library-suggestion-focus.mjs scripts/test-error-visibility.mjs scripts/test-edition-surfaces.mjs scripts/test-download-state.mjs scripts/test-collections-async.mjs scripts/test-mobile-voice-search.mjs scripts/test-provider-recovery.mjs scripts/test-settings-surface.mjs` → 116/116 pass
- `KIWIX_TEST_CHROME=…/chrome-nosandbox.sh node --test scripts/test-pane-surfaces.mjs`, three times → 3/3 each run (22.9 s, 25.9 s, 25.1 s)
- The regex equivalence and leak comparisons above, in V8 (node) and JSC 2.52.6 (C harness against `libjavascriptcoregtk-4.1`)

**What I could not verify:** WebKitGTK 2.40 itself, and real iOS 16.4 or macOS 13.3 webviews. Their lookbehind support comes from release-generation knowledge, not from running them. The lookbehind-free form makes that question irrelevant.