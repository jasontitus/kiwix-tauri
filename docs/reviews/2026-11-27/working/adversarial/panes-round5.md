## Verdict for lane panes (round 5): no findings — the lane is closed

The shipped regex is exactly the form I proved last round, it behaves identically in V8 and JavaScriptCore, and it leaks nothing new.

**The two checks you asked for**
- **The shipped regex matches the proven form.** I took my round-4 lookahead-only form, swapped in the `\.(?:part|zim(?:[a-z]{2})?)` gate, and compared it as a string with `bareFilesystemPath` in `src/components/UI.tsx`. They are equal, and `filesystemRoot` is unchanged.
- **No lookbehind in `src/`.** Searching `src/` (`.ts`, `.tsx`, `.js`) for `(?<=` and `(?<!` finds nothing.

**V8/JSC comparison and leak scan:** I re-ran it on the same 3021 inputs as round 4: the named cases plus the seeded fuzz.
- **V8 against JSC 2.52.6:** the shipped regex gave 0 differences.
- **Shipped against the round-4 regex:** 7 differences, all of them improvements. "Notes.v2 (Draft)", "Mr. X.files (Old)", "v1.2 (Old)", "Report.Final (Draft)", "a.b. Smith", "J.R.R. Tolkien" and "Backup.2024. Archive" are now fully redacted. "U.S. History" is also fully redacted.
- **Map sentences still read correctly:**
  - "…kept at [File]. Its folder is gone"
  - "…kept here: [File] (The file there is not…), [File]"
  - "Could not open [File]. Try again."
- **Outputs that still show path text after `[File]`:** only the two nested-parenthesis cases, which are listed under residuals and were the same or slightly worse at base.

**The implementer's upper-case doubt is confirmed and harmless.** `A.ZIM. Try again.` and `a.pdf. Try again.` become `[File]`, so the sentence tail is lost, as at base. `a.Part (Its folder is gone)` becomes `[File] is gone)`. No path text is revealed in any of these. The map recovery names Rust writes are always lower-case `.part` (`partials.rs:139`), so the recovery sentences never reach this case.

**What I ran**
- `npx --no-install tsc --noEmit` → exit 0
- The pane and errorText node tests (`test-pane-actions`, `test-error-visibility`, `test-collections-async`, `test-mobile-voice-search`, `test-provider-recovery`, `test-settings-surface`, `test-library-suggestion-focus`, `test-edition-surfaces`, `test-download-state`) → 116/116 pass
- `KIWIX_TEST_CHROME=…/chrome-nosandbox.sh node --test scripts/test-pane-surfaces.mjs` → 3/3 pass. I ran it once this round; the previous rounds ran it three times each.
- Comparison scripts (all under `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-panes/`):
  - `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-panes/mk.mjs` builds `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-panes/cmp.js` from `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-panes/patterns.json` and `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-panes/patterns-r4.json`.
  - `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-panes/jsc` is the JSC harness that runs `cmp.js`.

**Lane residuals (accepted; none blocks closure)**
1. **Redaction (iOS display only; exports keep the raw text):**
   - Nested parentheses in a folder name still show a tail: `Books ((Old) copy)/…` gives `[File]) copy)/wiki.zim`, and `Books (Old (2))/…` gives `[File]))/wiki.zim`.
   - A path that ends in `)` keeps a stray `)`.
   - An upper-case or unlisted extension before ". Capital" or " (Capital" eats the rest of the sentence. Nothing is revealed.
   - A reason inside the parentheses that itself contains a path is not covered by test 9.
   - The best long-term fix is for the backend to send the path in its own field rather than inside the sentence.
2. **Catalog matching:**
   - The `exact` match ("Installed", which hides Download) follows the Rust `series_compatible` rule, or is stricter.
   - The frontend lets two empty languages match exactly, where Rust refuses.
   - When an OPDS entry has no language, the parser falls back to a two-letter code, so `exact` is withheld and Download stays offered. That is the safe direction.
   - The related "Installed edition {date}" badge deliberately uses the looser rule.
3. **Suggestions:**
   - After any search they come back only on the next keystroke, not when the field is refocused.
   - A successful rollback replaces its button with "Rolled back…" and focus drops to the page body, as at base.
   - The pending "focus the results" request is now cleared when no search starts.
4. **Maps:**
   - The kept-file paths appear both in the `role=alert` error text and in the recovery list.
   - A Get map started from a card is dropped silently if the user opens another area's dialog while it is being prepared. That is base behaviour, triggered by the user's own navigation.
   - The share poll has no cap and runs every 750 ms while its dialog is open. It is retired when the dialog closes, the pane unmounts, privacy changes, or the backend reports an error.
5. **Not verified on real hardware or engines:**
   - Android rendering and TalkBack for the Move checkbox.
   - The iOS display and the phone soft keyboard.
   - WebKitGTK 2.40, iOS 16.4 and macOS 13.3 webviews. With the lookbehind gone, these no longer matter for the regex.
   - The edition-original outcomes "trashed", "missing" and "external", and the discard failure path, which no test renders.
   - A real map Delete that fails and keeps a file in a `.streetzim-cleanup-<uuid>/` folder.
   - A real long torrent hash.
   - The Rust tests for these handlers. I traced the contracts by reading the code and did not run cargo.

This closes the review of the lane. It is not certification for a release or for parity with other platforms.