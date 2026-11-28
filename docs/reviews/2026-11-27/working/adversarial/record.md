## Verdict for the record lane (final record, generated outputs, DEPENDENCIES table, RELIABILITY section): only non-blocking findings

None of the findings is Blocking. The record makes no claim that anything ran on a device, a Mac, Windows, a simulator, an Apple compiler or GitHub Actions. Line 283's "Fails on simulator/macOS" describes how the code is designed, not a run. Every Blocking finding is listed and closed. Every partial, accepted-residual and rejected row matches its lane's final report. The §5.4 numbers match the logs. The Rust sources are unchanged since `fbcb420`. The generated locales and `docs/ERRORS.md` regenerate byte for byte, and no translation was lost. What needs fixing is stale or contradictory wording, missing residuals, how §7 keeps the evidence, one pre-existing localization defect that the record presents as fixed, and one misleading omission in RELIABILITY.md.

### RECORD-R1: "the commit that adds this file" is already a wip commit that does not remove `working/`
Severity: Should-fix
Evidence class: reproduced (`git log`, `git ls-tree`)
Location: `docs/reviews/2026-11-27/PROJECT-REVIEW-REMEDIATION.md:7,10` (header) and §7 (lines 607–608)
Trigger: someone reads the record at `c12c231`, or at any later commit.
Causal path:
- `c12c231` ("wip: … (unreviewed)") is the commit that adds this file, and it still contains `working/` (125 files).
- So "removed in the commit that adds this file" is false.
- "Every commit after d98cac2 and before the one that adds this file is titled wip" excludes `c12c231`, which is itself a wip commit holding an unreviewed draft.
- "The reviewed state is the commit that adds this file" therefore names an unreviewed commit.
Smallest correct fix: name the reviewed state as "the commit that removes `working/` (the child of EVIDENCE_COMMIT)", or give its hash once it exists. Apply the same wording to the header row "Tree this record describes".

### RECORD-R2: the evidence commit can become unreachable, and §5.4's logs are not in it
Severity: Should-fix
Evidence class: source-traced
Location: §7; `scratchpad/sync-working.sh`
Causal path:
- EVIDENCE_COMMIT survives only if the branch is merged without squashing or rebasing. This repository has squashed history before: adv buildhygiene r2 says the historical "31 packages" figure "cannot be verified, because the git history is squashed".
- `sync-working.sh` copies only `*.md`. So none of `final-*.log`, `final-rev.txt` or this review's report reach `working/`.
- After the removal commit, nothing durable supports §5.4.
- The full Kotlin test command lives only in `working/ANDROID-ENV.md`. No other doc mentions `testDebugUnitTest` except historical lines in RELIABILITY.md. That file goes away too, although the ANDROID-R11 owner decision was to document the local command.
Smallest correct fix:
- Before EVIDENCE_COMMIT, copy `final-*.log`, `final-rev.txt` and this report into `working/`.
- Tag the evidence commit, or state in §7 that the branch must not be squash-merged.
- Put the Kotlin unit-test command in a durable doc, or say plainly that it is not documented.

### RECORD-R3: stale and contradictory statements inside the record
Severity: Should-fix
Evidence class: reproduced (read against adv reports and the code)
Location and causal path:
- **§3.3 SERVE-6** ("Round-2/3 doc follow-ups not re-reviewed"), **§3.13 BUILDHYGIENE-R1** ("**not re-reviewed**") and **§5.3** ("**No adversarial run for rounds 2 and 3.**"): all false. `adv/buildhygiene-round2.md` reviewed rounds 2 and 3, says "R1: fixed", and ran `test-build-hygiene` (5/5) plus `cargo tree`. §2.1 and §8.1 already say so.
- **§6.9** says "Host clippy across all lanes on the final tree is not in the record". §5.4 now has it.
- **§6.9** also says "buildhygiene rounds 2–3 … need independent review". This is stale.
- **§6.4** says the `dispatch.rs:1701` comment was "reported stale … no report confirms the edit". §8.12 and the code say otherwise: `src-tauri/src/service/dispatch.rs:1704-1709` now says the move is cancelled rather than retried.
- **§6.9** says "the record does not show that the command was documented". §8.13 says it is documented in §5.4.
- The §5.4 Kotlin row is abbreviated (`gradle --offline …`); see RECORD-R2.
Smallest correct fix: delete or rewrite the six stale lines so they match §2.1, §5.4 and §8.12/§8.13.

### RECORD-R4: closure is overstated for the owner, apple and buildhygiene lanes
Severity: Should-fix
Evidence class: reproduced
Location: §1, line 28 ("All fifteen lanes and both crash ports closed"); the §2.1 definition at line 43
Causal path:
- §2.1 defines "Closed" as "the lane's last adversarial report states closure".
- `adv/owner-round2.md`: "R4 still open". The owner lane is not closed.
- `adv/apple-round2.md` gives the verdict "only non-blocking findings" and says "Two things still stand between this and calling APPLE-1 and APPLE-4 fully done". It never states that the lane is closed; the closure is recorded only in LEDGER line 222.
- `adv/buildhygiene-round2.md` closes only conditionally: "can close once DEPENDENCIES.md:3 is reworded".
- §1 also says OWNER-R4 "closes with the independent review of this record", which assumes the outcome of that review.
Smallest correct fix:
- Say that 13 lanes and both ports closed in their reports.
- Say apple was closed by owner decision, with APPLE-R7 open as a device gate.
- Say buildhygiene was closed conditionally, and that the condition is now verified (item 3 below).
- Say the owner lane closes once this review's findings are dispositioned and re-checked.

### RECORD-R5: §6 leaves out residuals that final adversarial reports list
Severity: Should-fix
Evidence class: reproduced
Location: §6 (it has no buildhygiene subsection)
Causal path: these residuals are missing.
- **adv buildhygiene r2:**
  - the Android opt-in check covers aarch64 only (armv7 was never checked);
  - no runtime test of the GeoClue or LocationManager providers;
  - a docs-only PR touching Markdown other than FORKS.md skips the libzim-claim check until the next PR that is not docs-only;
  - the historical "31 packages" figure cannot be verified.
- **adv panes r5:**
  - a Get map started from a card is dropped silently if another area's dialog opens while it is being prepared (base behaviour);
  - a reason containing a path is not covered by test 9;
  - a real long torrent hash was never rendered.
- **adv owner r2:** `VARIABLE_SOURCES` cannot find new variable-source call sites.
- **adv files r3:** the R2 proxy keeps showing a warning for a library whose only book was removed mid-move. §4.3 names the proxy but not this effect.
- **adv service r4:** the notice band was never seen rendered in a real shell.
Smallest correct fix: add a §6 buildhygiene subsection and add the other items to §6.4 and §6.8.

### RECORD-R6: Rust-only plural strings never reach the runtime catalogs, but OWNER-R1 is recorded as fixed
Severity: Should-fix (pre-existing at `d98cac2`, not a regression)
Evidence class: source-traced; the catalog contents were reproduced
Location: `scripts/localization.mjs:254-258` (`inventory`: sources are the `extra-en-*.json` keys plus TS `t()`/`tPlural()` literals) and `:658-669` (`runtimeCatalogs` is filtered to `activeSources`); `src-tauri/src/localization.rs:16` (Rust embeds only `messages/`) and `render` (`unwrap_or(source)`); `src-tauri/src/sharing.rs:2763,2788`
Trigger: open the shared-library web page for a single archive, in any locale.
Causal path:
- `"{count} articles"` and `"Serving {count} archives from this device…"` are in `plural-sources.json`. They are not in the manifest sources or in any `messages/*.json`; I checked `en.json` and `de.json`.
- Rust therefore falls back to the raw template. The page shows "1 articles" and "Serving 1 archives…", in English, in every locale.
- `pluralCoverage.total` became 29, but these two can never be shipped.
- `test-rust-localized-strings.mjs` checks membership in `plural-sources.json` only, so it passes either way.
Smallest correct fix:
- Add both keys to `src/locales/extra-en-rust.json`, so `generate` keeps their English forms. By reading, plural forms override the flat value; I did not run this.
- Make the test assert that each `.counted()` source is present in `messages/en.json`.
- Or record this as a §6.9 residual and stop describing the counted() shape as covered.

### RECORD-R7: RELIABILITY.md and §5.5 do not say that the app was never launched on Linux either
Severity: Should-fix
Evidence class: inference (no impl or adv report records running the Tauri app; the browser tests use `dist/` in Chromium, and the WebKitGTK attack scripts ran against copies of the code)
Location: `docs/RELIABILITY.md`, 2026-11-27 section ("None of these changes has been run on a phone, a Mac, Windows or an iPhone simulator"); record §5.5
Causal path: the document tracks the "open a downloaded ZIM, read it, restart, still read it" loop. Listing every platform except Linux implies that Linux desktop ran that loop. It did not.
Smallest correct fix: add "and the app itself was not launched on any platform, Linux included; only library tests and headless-browser tests of the built frontend ran" to both places.

### Nits
- **§2.2 counts.**
  - `working/` has 124 files at `08528ee`, not 125 (125 is the count at HEAD).
  - The final diff will be 274 paths, because it includes this record.
  - The "31 frontend files" includes 3 added files.
- **"Reviewer's exact text".**
  - §2.1 and §8.1 say the owner used it. Line 3 adds "of the size table below".
  - The owner also reworded line 51 ("The per-platform 2026-11-05 figures (both earlier columns)"), which the reviewer did not propose.
  - Both edits are true, but "that day" on line 51 is ambiguous. It is true when read as 2026-11-28, which I re-measured.
- **§5.4 evidence gaps.**
  - No log records its command, so `--lib --tests` cannot be checked from the logs.
  - `final-clippy-optin.log` has no `EXIT` line, only `Finished`.
  - There is no log for `npm run build`, for the Python tests or for `swiftc -parse`. The `dist/` timestamp, 03:27:32, is consistent with the build.
  - The Kotlin 79 tests are in the JUnit XML, not in the log.
  - `final-rev.txt` has a stray second line, "79".
  - §8.4 says `test-build-hygiene` was re-run after the DEPENDENCIES edit. No run is recorded after 03:31:58. My run now passes 5/5.
- **Closing-review pointers.**
  - READER-3 should cite adv reader r2: READER-R2 was raised against its fix and closed in r2.
  - FEAT-6 cites "adv features r4", which never mentions it. Cite r1 for the backend and r2 for Delete.
- **§3.13** leaves out buildhygiene r2's unnamed should-fix (`DEPENDENCIES.md:3`) from the Should-fix table.
- **`docs/ERRORS.md`** has no row for some new user-facing Rust strings, for example the `IMPORT_CANCELLED` const text and "Kiwix could not start.". This is the extractor's existing heuristic in `error-inventory.mjs` (marker context), not a regeneration defect.

### Per-item assessment
1. **Record vs sources.**
   - I checked all 113 rows plus OWN-1 and the R8 note against each lane's per-fix lines and final report. That is more than 40 rows, across all 11 areas.
   - All partial rows match: APPLE-6, SHELL-7, ACQ-6 (conflict disclosed in §8.7), LIB-9, BUILD-2, READER-3, ANDROID-5, ANDROID-11, ANDROID-12.
   - All accepted-residual rows match: APPLE-9, BUILD-11, NET-10. No row is rejected.
   - A grep over every `adv/*.md` confirms the Blocking list: exactly FEATURES-R1, APPLE-R1, FILES-R1, SERVICE-R1 and CIDOCS-R1, plus TRANSFERS-R1, which the owner raised to Blocking.
   - Every Should-fix ID is in §3.13. ANDROID-R3 is a Nit, so it is correctly left out.
   - §2.1 round counts and owned files match the impl/adv file sets, apart from RECORD-R4.
   - §4 matches the LEDGER decisions. R11 "Refused" was checked against `maps/partials.rs` and adv features r4.
2. **Generated outputs.**
   - `check:errors` and `check:localization` pass.
   - `localization.mjs generate`, run in a scratch copy, reproduces the committed `src/locales/**`, `native-locales` and the Android `res` byte for byte.
   - Sources went from 1552 to 1669: 135 added, 18 removed.
   - In de, fr, ar, ja and es, each locale lost exactly 3 translated keys, all with removed sources. No translated value changed.
   - The translated count per locale is unchanged, about 1241 (1241/1552 = 0.7996 and 1241/1669 = 0.7436). So the drop in non-English coverage (en stays 1.0) comes from the new untranslated sources.
   - These new sources are in `en.json` and in `missingNew.de`: "Printing is not available…", "Open Kiwix to see progress", "Used today: {count} requests", "Quit Without Saving", "Sharing PIN", "Look up in Kiwix", "Reading aloud".
   - In ERRORS.md: "Quit Without Saving" (lib.rs), the `.kiwix-renamed-…` hint (library_files.rs), "…still in {}" (library_files.rs), ACTIVITY_RECREATED (mobile_service.rs; first file under the generator's rule).
   - The plural exception is RECORD-R6.
3. **DEPENDENCIES.md.**
   - The table matches `adv/buildhygiene-round2.md` cell for cell.
   - The 2026-11-05 "After" column equals the "Now" column at `d98cac2`: 686, ~493, ~456, ~460, ~441, ~442, then 7 and 128.
   - I re-measured the 2026-11-28 column: 537 (565 with all features), 488, 511, 506, 502; lockfile 739.
   - Line 3 differs slightly from the proposed text (Nit).
4. **RELIABILITY.md.** Every sentence is true against the record, except the omission in RECORD-R7. "Integration tests" is loose: no Rust `tests/*.rs` exist, so this means the Node, Python and lib-level tests. OWNER-R4's claim ("That record lists the verification actually run on the final tree") now holds.
5. **§7 vs AGENTS.md.** The approach satisfies AGENTS.md's record items:
   - base `d98cac2`;
   - changed paths (§2.2 plus the exact command);
   - findings and dispositions (§3);
   - verification (§5);
   - residual blockers (§6).
   
   It holds only if RECORD-R1 and RECORD-R2 are fixed, so that the evidence stays reachable and the logs are kept. Under AGENTS.md, the final edit (filling EVIDENCE_COMMIT, fixing these findings, removing `working/`) needs its own re-check.

### What I ran
- `git diff fbcb420 -- src-tauri/src src-tauri/build.rs src-tauri/Cargo.toml src-tauri/Cargo.lock` gave an empty diff.
- `git diff --name-only fbcb420 HEAD` shows only docs, locales and `test-build-hygiene.mjs` (mtime 03:25:58, before the Node run).
- `git status` is clean at `c12c231`. `git diff --quiet d98cac2 -- src-tauri/Cargo.lock` gives 0.
- Logs:
  - cargo test: `ok. 1493 passed; 0 failed; 16 ignored`, EXIT 0.
  - clippy: host, Windows and Android EXIT 0; opt-in shows `Finished` only.
  - unsafe census: 75/75/2 in 20 files (base 74 in 19).
  - Node: 545 tests, 544 pass, 0 fail, 1 skipped.
  - Swift: 238/238, `bound: ALL PASSED`, 4×250,000 + 2×300,000 + 500 = 1,600,500 cases, 0 mismatches.
- Kotlin JUnit XML (03:32:28): 79 tests, 0 failures, 0 errors, 0 skipped.
- `swiftc -parse ios/Sources/*.swift ios/Tests/*.swift`: exit 0.
- `node --test scripts/test-build-hygiene.mjs`: 5/5.
- `npm run check:errors`: "matches". `npm run check:localization`: `"failures": []`.
- `localization.mjs generate` in `scratchpad/adv-record/gen/`: no diff against the repo. The repo was verified untouched afterwards.
- `cargo tree --offline` counts on 5 targets (metadata only, nothing built).
- Python comparisons of the locale JSON at `d98cac2` and now.

### What I could not verify
- The exact commands behind each log, including whether `--tests` was passed.
- The `npm run build` warnings and the Python "182 OK (2 skipped)". Neither has a log, and I did not run them.
- The runtime rendering in RECORD-R6 (source-traced, not run).
- Whether a Tauri app launch happened somewhere unrecorded.
- Anything on a device, Mac, Windows, simulator, Apple SDK or GitHub Actions.
- I did not re-run `cargo test` or clippy: the Rust tree is unchanged since the logged runs.
