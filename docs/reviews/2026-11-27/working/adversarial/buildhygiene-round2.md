## Buildhygiene rounds 2–3: R1–R8 are fixed; one sentence in DEPENDENCIES.md:3 is now false and needs a fix

`node --test scripts/test-build-hygiene.mjs` passes (5 tests, 0 failures). The lane can close once DEPENDENCIES.md:3 is reworded (exact text below). That fix is docs-only and needs no further Rust re-review.

As asked, I only read files and ran node. The only cargo commands were three `cargo tree` metadata queries; no builds or tests.

### 1. Each R1–R8 fix
- **R1:** fixed. `docs/FORKS.md:70-72` has the retitled heading, the "migration is complete" sentence, and "not a current gap list". The false sentence about zimru never appearing in the history is gone. The gap heading and the "used" tense changes are fine.
- **R2:** fixed. `DEV-ITERATION.md:53` is now `npm run tauri -- ios dev --target aarch64-sim`.
- **R3:** fixed. `linux-appimage.py:108` now says "…name, startup class or file associations". No test matches the old string.
- **R4:** fixed (see item 2 for the filter order). The test header (lines 5–6) states that edits to other Markdown files are only caught on the next run that is not docs-only.
- **R5:** fixed. `test-build-hygiene.mjs:112-116` requires at least two `cargo clippy` runs and at least one `--features` line without `experimental-location`. Both jobs satisfy it: linux uses `--features custom-protocol`, android uses `${{ matrix.features }} --`.
- **R6:** fixed. The method now uses `--features custom-protocol`, line 96 mentions cargo-deny's graph, and the PACKAGING Linux paragraph uses "only when a developer opts in…".
- **R7:** fixed.
  - The `WINDOWS-TEST-BRIEF.md:60` opener comes before "the build needs".
  - `WINDOWS-AGENT-HANDOFF.md:23` has been changed to match.
  - The `#build-and-test` anchor exists (`WINDOWS-BUILD-PROVISIONING.md:38`).
  - Phase 3 is retitled "(historical baseline: needs the SDK)" and no doc links to the old anchor.
  - Phase 4's title still says "(needs the SDK)". That is a nit, covered by the opener and the file banner.
- **R8:** fixed (see item 3).

### 2. `checks.yml` path filter
The order is correct in both the `push` and `pull_request` filters: `'**'`, then the `!` exclusions, then `docs/ERRORS.md`, `docs/FORKS.md` and `scripts/test-build-hygiene.mjs` last. GitHub applies the last matching pattern, so a later positive pattern re-includes a path an earlier `!` excluded. A PR that only edits FORKS.md therefore runs the checks.

The `scripts/test-build-hygiene.mjs` entry is redundant, because `'**'` already includes it and no `!` excludes it. It is harmless and the header comment explains it. The YAML parses.

### 3. `build.rs` warning arm (lines 30–41)
It is correct, and the emitted cfg is unchanged on every target:
- macOS, iOS and Windows still hit the first arm.
- Linux and Android with `=1` still hit the assert-then-cfg arm.
- The new arm only prints, and only when the variable is set (to anything, including empty) or the feature is on.
- Default builds and both CI opt-in steps print nothing.
- A `cargo:warning` is not a rustc lint, so `-D warnings` cannot fail on it.
- cargo-deny's `--all-features` runs no build scripts, so the arm never fires there.
- The variable's value is not echoed.

### 4. Refreshed DEPENDENCIES numbers
- I re-measured three rows with the documented command and they match: macOS arm64 511, iOS arm64 502, Windows x86_64 506. Linux 537 and 565 match my round-1 measurement.
- The lockfile has 739 `name = ` entries, and so does the base `d98cac2`. The 686 → 739 growth therefore happened before this lane; this lane's change to `Cargo.lock` is zero.

**Should-fix: `docs/DEPENDENCIES.md:3` is now false.**
- It says: "Snapshot taken 2026-11-05, before and after the reductions under "Done"…".
- The "Now" column is no longer the post-reduction snapshot. It is a 2026-11-28 measurement that includes dependencies added since.
- The table now reads as if the "Done" reductions grew the graph, and the actual post-reduction figures have been erased. At `d98cac2` they were 686, ~493, ~456, ~460, ~441 and ~442.

Preferred fix (keeps the evidence): give the table three columns:

```
| Graph | Before reductions (2026-11-05) | After reductions (2026-11-05) | Now (2026-11-28) |
| Rust lockfile, all targets | 726 | 686 | 739 |
| Rust, Linux x86_64 build (normal + build deps) | ~564 | ~493 | ~537 |
| Rust, macOS arm64 | ~488 | ~456 | ~511 |
| Rust, Windows x86_64 | ~491 | ~460 | ~506 |
| Rust, Android arm64 | ~472 | ~441 | ~488 |
| Rust, iOS arm64 | ~473 | ~442 | ~502 |
| npm runtime packages | 7 | 7 | not remeasured |
| npm dev/build packages | 129 | 128 | not remeasured |
```

Drop the per-cell "(remeasured 2026-11-28)" notes. Replace line 3 with:

> "The first two columns were measured on 2026-11-05, before and after the reductions under "Done", with `--all-features` and from `src-tauri/Cargo.lock` and `package-lock.json`. The last column was remeasured on 2026-11-28 with the shipping feature set; it also includes dependencies added after 2026-11-05, so its difference from the earlier columns is not the effect of those reductions. Re-run the commands below to refresh it. Vendored forks and native patches are covered in [FORKS.md](FORKS.md)."

If the owner wants to keep two columns, the minimum fix to line 3 is:

> "The "Before" column was measured on 2026-11-05, before the reductions under "Done". The "Now" column was remeasured on 2026-11-28 and includes dependencies added since, so it does not show the effect of those reductions (the post-reduction figures from 2026-11-05 were 686, ~493, ~456, ~460, ~441 and ~442)."

The rest of the nearby prose is still true:
- Line 49, "The reduction opportunities are in what the app adds on top", describes opportunities, not results.
- "~490–540" matches the measured 488–537.
- The "Done" entries (removed 10, removed 25, "Linux x86_64 dropped 31…") are per-change deltas, not table readings.
- Line 132 is explicitly dated ("710 lockfile packages").
- Line 134's "Removed this round" names crates, not counts.

**Nit:** the npm "Now" cells (7 and 128) were not remeasured, but they sit in a column now labelled 2026-11-28. Hence "not remeasured" above. I could not reproduce the doc's npm counting method, so I am not claiming the numbers are wrong.

### Residuals
- The new CI opt-in steps have not run on GitHub, and their timing is unmeasured.
- The Android opt-in check covers aarch64 only. armv7 and Windows-gnu clippy were not re-run in round 2, though only `build.rs` changed and that change prints only.
- No runtime test of the GeoClue or LocationManager providers; both remain unreleased.
- A docs-only PR touching Markdown other than FORKS.md still skips the libzim-claim check until the next PR that is not docs-only. This is documented in the test header.
- The historical "31 packages" figure for single-instance cannot be verified, because the git history is squashed.
- Historical PACKAGING `<details>` blocks still mention `TAURI_KIWIX_MOBILE_SDK_ROOT` and the deleted mobile SDK loader. This is deliberate.
- Phase 4 of WINDOWS-TEST-BRIEF is still titled "(needs the SDK)" (nit).
- The mobile_policy test comment ("Bluetooth commands…") heads a list that is mostly not Bluetooth (cosmetic).

### What I ran
- `node --test scripts/test-build-hygiene.mjs` → 5 tests, 5 pass, 0 fail.
- `cargo tree --locked --features custom-protocol --target {aarch64-apple-darwin,aarch64-apple-ios,x86_64-pc-windows-msvc} -e normal,build --prefix none | awk | sort -u | wc -l` → 511, 502, 506.
- `grep -c '^name = '` on `Cargo.lock` → 739 now and 739 at `d98cac2`.
- Read the `checks.yml` filters, `build.rs:13-43`, and the round 2–3 edits in FORKS, DEV-ITERATION, `linux-appimage.py`, PACKAGING, both Windows docs, DEPENDENCIES and the test.
