## Record review r2 (at `c47a940`): only non-blocking findings

All seven r1 findings are closed. I found one new Should-fix: the final-run localization row still gives the old source count. The rest are small wording and staleness nits. Nothing in the record claims a device, Mac, Windows, simulator, Apple-compiler or GitHub Actions run. The evidence logs hold nothing private beyond container paths and proxy settings (details at the end).

### RECORD-R8: §5.4 says it gives the final run, but its localization row still says 1669 sources
Severity: Should-fix
Evidence class: reproduced
Location:
- `PROJECT-REVIEW-REMEDIATION.md:505` (§5.4 row "English sources 1552 → 1669")
- `PROJECT-REVIEW-REMEDIATION.md:607` (§6.9 "sources 1552 → 1669")
Causal path:
- The §5.4 lead now says "The rows below give the final run".
- After the R6 fix, `src/locales/manifest.json` at `c47a940` has **1671** sources: the two Rust plurals were added.
- Coverage is now de 0.7427, fr 0.7433 and ar 0.7421, so "about 0.74" still holds.
- Line 484 (§5.3) is a historical integration line and can stay at 1669.
Smallest correct fix:
- §5.4: "English sources 1552 → 1671 (1669 before the two shared-library plurals were collected)".
- §6.9: the same correction.

### Nits
- **§1 lane count does not add up.** The record says "Thirteen lanes and both crash ports closed… Two closed on a recorded condition".
  - 12 implementation lanes closed in their own reports: transfers, features, files, shell, reader, android, net, startup, cidocs, desktop, service and panes.
  - Apple and buildhygiene closed on a condition. That makes 14 implementation lanes, plus the owner lane listed separately.
  - Suggested wording: "Twelve implementation lanes and both ports closed in their reports; two more (apple, buildhygiene) on a recorded condition".
- **§1 apple condition.** The record says APPLE-1 and APPLE-4 were kept short of done "until the device check APPLE-R7 runs". In adv apple r2, only APPLE-1 hangs on R7. APPLE-4 hung on R8, the stale wording missing from the catalogs. The owner added that string; it is still untranslated.
- **§1 line 31 and §6.9 line 609 assume this review's outcome.** They say the findings were "applied and re-checked in record review r2". That becomes true once RECORD-R8 is fixed. Keep the wording only after the fix.
- **Stale §8 items.**
  - §8.4 says the Node and Python runs "predate only documentation edits (buildhygiene round 3 and the DEPENDENCIES table)". The final `final-node2.log` and `final-python.log` (03:58–03:59) postdate those edits. They now predate only this record and the RELIABILITY.md edit (04:01).
  - §8.2 still says no reviewer checked the generated outputs.
  - §8.1 still says "the reviewer's exact … replacement sentence". Line 3 adds "of the size table below", and §3.13 has already dropped "exact".
  - §8.10 could point at the §5.4 final host clippy run.
- **§5.4 says clippy does "not read the catalogs".** Strictly, the crate compiles `include_str!("../../src/locales/locale-index.json")`, which changed in `c47a940`. Lint results cannot depend on that JSON's content, so not re-running clippy is harmless. The sentence is just inexact.
- **The opt-in clippy log's exit line was written afterwards.** `verification/final-clippy-optin.log` ends with "EXIT 0 (Finished above; command: …)". It is honestly labelled, but it was inferred from `Finished`, not captured. The clippy, unsafe census and Kotlin console logs (`final-clippy-{host,win,android,optin}.log`, `final-unsafe.log`, `final-kotlin.log`) have no CMD line; the README defers them to §5.4. That is acceptable.
- **Before you make the evidence commit,** copy this r2 report into `working/adversarial/` (for example `record-round2.md`), so §6.9 and §7 ("including this record's own reviews") stay true. `adversarial/record.md` is present and identical to the scratch copy.

### Per-finding status
- **R1 (header and §7 reviewed state): closed.** The reviewed state is now the commit that removes `working/`, the child of the evidence commit and the first non-`wip:` commit. `c12c231` and `c47a940` are wip commits, consistent with §7. EVIDENCE_COMMIT is still a placeholder, as intended.
- **R2 (evidence durability): closed.**
  - §7 says to merge without squashing, or to tag first.
  - `working/verification/` holds every final log plus `final-README.log`.
  - `adversarial/record.md` is in place.
  - The full Kotlin command, environment and JUnit path are in §5.4. `final-kotlin-junit.log` records 79/0/0/0, which matches my count of the XML files.
- **R3 (stale lines): closed.** I checked SERVE-6, BUILDHYGIENE-R1, the §5.3 buildhygiene entry, §6.4 (it now matches `dispatch.rs:1704-1709`) and the §6.9 clippy, buildhygiene and command lines. The §8 leftovers are listed under Nits.
- **R4 (closure overstated): closed in substance.** The lane count and the APPLE-4 attribution are listed under Nits.
- **R5 (missing residuals): closed.** The buildhygiene items are in the new §6.10, the panes and service items in §6.8 and §6.4, the FILES-R2 effect in §6.4, and VARIABLE_SOURCES in §6.9.
- **R6 (Rust plurals not collected): fixed.**
  - Both keys are in `extra-en-rust.json`. `messages/en.json` and `catalogs.json` now have one/other forms for "{count} articles" and "Serving {count} archives…". Provenance moved them into the right position.
  - The changed `test-rust-localized-strings.mjs` passes on the tree.
  - In a scratch copy, removing "{count} articles" from `en.json` fails the test, naming `sharing.rs counted(): {count} articles`. Restoring the `c12c231` catalogs fails on both strings. So the test can fail.
  - The VARIABLE_SOURCES plural entries go through the same `plural` set. Their search-note sources are already in `en.json`.
  - The five "unused" plural sources occur nowhere in `src/`, `src-tauri/src` or the Kotlin/Swift plugin sources.
  - No translation was lost. Against both `d98cac2` and `c12c231`, de, fr, ar and ja lost no translated key whose source still exists, and no value changed.
- **R7 (app never launched): closed.** The statement is in §5.5 and RELIABILITY.md. Every sentence of the RELIABILITY section is now true, including "the Node and Python script suites and headless-Chromium tests of the built frontend".
- **R1-era nits (pointers, file counts, BUILDHYGIENE-R2 in §3.13): closed.** READER-3 now points to r2, FEAT-6 to r1/r2, and the count reads 124.

### Evidence logs: privacy
- I scanned `working/verification/*.log` for e-mail addresses, home, `/root` and `/Users` paths, secret, token, key and UDID patterns, the session URL and the scratch path.
- **Nothing private.**
  - The only filesystem path is `/home/user/kiwix-tauri`. The node CMD uses a `<chrome-nosandbox.sh>` placeholder.
  - "secret" and "password" appear only in test names. `sk-BAMi3-zt.js` is a Vite chunk name.
  - The one infrastructure detail is in `final-kotlin.log` lines 1 and 43: the JVM's `JAVA_TOOL_OPTIONS` (local proxy `127.0.0.1:36079`, its `nonProxyHosts` list including `*.anthropic.com` and registry hosts, the trust store path). That is container configuration, not personal data.
- The adversarial reports in `working/adversarial/` (not part of this check) do contain `/tmp/claude-0/…` scratch paths, which are container paths.

### What I ran
- **Timeline:** `git log` and `stat`. Catalogs were regenerated at 03:55:49, the test changed at 03:55:59, then build 03:56:16, cargo 03:58:07, node 03:59:25 and python 03:59:48. All final runs postdate the fix.
- **Log tails:**
  - cargo2: `1493 passed; 0 failed; 16 ignored`, EXIT 0.
  - node2: 545 tests, 544 pass, 0 fail, 1 skip.
  - python: 182 OK (2 skipped), then 1 OK.
  - build: EXIT 0.
  - swift-parse: EXIT 0.
- **Checks:** `node --test scripts/test-rust-localized-strings.mjs` passes 1. `test-localization-inventory`, `test-error-docs` and `test-build-hygiene` give 13/13. `npm run check:localization` reports `"failures": []`. `npm run check:errors` reports "matches".
- **Regeneration:** `localization.mjs generate` in `scratchpad/adv-record/gen2/` is byte-identical to the committed `src/locales/**`, `native-locales` and Android `res`. The repo was verified untouched afterwards.
- **Fail-without-fix:** both scratch runs in `scratchpad/adv-record/r2t/` failed as described under R6.
- **Comparisons:** Python comparisons of the locale JSON, and greps for the unused plural sources.

### What I could not verify
- I did not re-run cargo, clippy, Kotlin or Swift myself; I relied on the logs.
- The runtime rendering of "1 article" is source-traced: the `en.json` forms plus `Localizer::render`'s English fallback.
- Anything on a device, Mac, Windows, simulator or GitHub Actions.
