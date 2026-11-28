## buildhygiene round 2: report

I applied all eight items, R1–R8. Everything I ran passes:
- the hygiene test (5 of 5);
- host clippy, both the default and the opt-in build;
- the AppImage test.

I did not re-run the Android or Windows-gnu clippy. This round changed only one Rust file, `build.rs`, and the new code just prints a warning.

### What changed
- **R1, `docs/FORKS.md`:**
  - The heading is now "## Replacing libzim with zimru (done; pre-migration analysis)".
  - Its first sentence is the reviewer's exact text: the migration is complete, the app reads with `zimru` and searches with `zimfile-xapian` (both pinned in `src-tauri/Cargo.toml` at lines 71–72), and `archive.cpp` has been removed. The rest of the section is labelled as pre-migration analysis, not a current gap list.
  - I deleted the sentence "Nothing in this repository's history mentions zimru…".
  - So the gap table no longer reads as current, I made two small tense changes: "### Gaps today" became "### Gaps at the time of the analysis", and "`archive.cpp` uses a small slice" became "used a small slice". The table contents are unchanged.
- **R2, `docs/DEV-ITERATION.md:53`:** the iOS simulator command is now just `npm run tauri -- ios dev --target aarch64-sim`.
- **R3, `scripts/linux-appimage.py:108`:** the message now reads "Desktop entry lost its executable, icon, name, startup class or file associations".
- **R4, `checks.yml` path filter:**
  - In both the `push` and `pull_request` filters, `'docs/FORKS.md'` and `'scripts/test-build-hygiene.mjs'` now come after `'docs/ERRORS.md'`, so they follow every exclusion; the last matching pattern wins.
  - I extended the header comment to explain this.
  - The test script was never actually excluded, because `'**'` already matches it; re-including it is redundant but harmless.
  - A docs-only PR that edits any Markdown file other than FORKS.md still skips the check. I said this in the test's header, as the reviewer suggested.
- **R5, `scripts/test-build-hygiene.mjs`:** each type-check job must now contain at least two `cargo clippy` invocations, and at least one `--features` line without `experimental-location`.
  - To check this, I folded the opt-in feature into the default Linux step of a modified in-memory copy of the workflow. The "one clippy run must use the shipping feature set" check then failed: no shipping `--features` line was left.
- **R6, `docs/DEPENDENCIES.md` and `docs/PACKAGING.md`:**
  - The count method now uses `--features custom-protocol`. I added a note: earlier rows used `--all-features`, which on Linux now adds 28 packages from `experimental-location` (565 instead of 537 on 2026-11-28). The other four targets gave the same count either way that day, and only the Linux row was refreshed.
  - The Linux row is now "~564 | ~537 (remeasured 2026-11-28)".
  - At line 96 it now says "…which only the CI type-check enables for compilation (… cargo-deny's `--all-features` graph also includes it)".
  - The PACKAGING Linux paragraph now says the provider is enabled "only when a developer opts in with `KIWIX_EXPERIMENTAL_LOCATION=1` and the `experimental-location` feature, as the CI type-check does".
- **R7, Windows docs:**
  - Phase 2 of `docs/WINDOWS-TEST-BRIEF.md` now opens with "Current builds skip Phase 2's SDK steps; see [WINDOWS-BUILD-PROVISIONING.md](WINDOWS-BUILD-PROVISIONING.md#build-and-test). For the historical baseline, the build needs…". I put it at the top of Phase 2 rather than mid-section, so it comes before "the build needs".
  - Line 23 of `docs/WINDOWS-AGENT-HANDOFF.md` now starts "Current builds need no SDK (see [same link]); for this historical baseline, if the machine lacks the SDK…".
  - The anchor exists: `## Build and test` is at line 38 of `WINDOWS-BUILD-PROVISIONING.md`.
- **R8, `src-tauri/build.rs`:** a new arm handles Linux and Android when the env var is not "1". If the env var is set (to anything else) or the feature is on, it prints one `cargo:warning`.
  - The warning says whether the env var is unset or not 1, and whether the feature is on or off. It never echoes the env var's value, which keeps it to a single line.
  - The existing panic (env var set to 1 without the feature) is unchanged.
  - Default builds print nothing, and the emitted cfg is unchanged on every target.

### Commands run
- `node --test scripts/test-build-hygiene.mjs` → 5 pass, 0 fail.
- Host clippy, default (`--lib --tests --features custom-protocol -- -D warnings`) → Finished, exit 0, no warning.
- Host clippy, opt-in (`KIWIX_EXPERIMENTAL_LOCATION=1 KIWIX_TYPECHECK_ONLY=1 … --features custom-protocol,experimental-location`) → Finished, exit 0, no warning.
- Feature on, env var unset → Finished, with the warning "…KIWIX_EXPERIMENTAL_LOCATION=1 (it is unset) together with --features experimental-location (it is on)".
- `KIWIX_EXPERIMENTAL_LOCATION=true`, feature off → Finished, with the warning "…(it is not 1)…(it is off)".
- `python3 -I scripts/test-linux-appimage.py` → Ran 1 test, OK.
- `checks.yml` parses with PyYAML.

### Files changed this round
- `docs/FORKS.md`
- `docs/DEV-ITERATION.md`
- `scripts/linux-appimage.py`
- `.github/workflows/checks.yml` (filter and header comment only)
- `scripts/test-build-hygiene.mjs`
- `docs/DEPENDENCIES.md`
- `docs/PACKAGING.md`
- `docs/WINDOWS-TEST-BRIEF.md`
- `docs/WINDOWS-AGENT-HANDOFF.md`
- `src-tauri/build.rs`

There are no new user-visible frontend strings. The only new Rust string is the build-time warning.

### Doubts
- The other "Now" figures in the DEPENDENCIES size table are stale. Measured today with the shipping feature set:

  | Graph | Table says | Measured |
  |---|---|---|
  | macOS arm64 | ~456 | 511 |
  | Windows | ~460 | 506 |
  | Android arm64 | ~441 | 488 |
  | iOS | ~442 | 502 |
  | Lockfile | 686 | 739 |

  The prose figure "~440–490" is stale for the same reason. As asked, I refreshed only the Linux row and left the rest for the owner.
- In `WINDOWS-TEST-BRIEF.md`, Phase 3 is still titled "(needs the SDK)". The new Phase 2 opener covers it, but I did not retitle it.