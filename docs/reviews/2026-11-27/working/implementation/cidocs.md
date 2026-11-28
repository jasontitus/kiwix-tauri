# Lane `cidocs` — report

All ten findings are dispositioned. Nothing was pushed, committed or regenerated. Two of my new tests fail by design until you regenerate (`docs/ERRORS.md`, localization). Three things need your attention before push:

- **Other lanes' browser tests fail here.** `scripts/test-shell-surfaces.mjs` and `scripts/test-nearby-sharing-surfaces.mjs` failed 7 of 8 with a browser (load average was about 10). My CI step sets `KIWIX_TEST_CHROME`, so they become gating.
- **The Windows job has never run.** The only manual run of `build.yml` skipped it, so my new test step there is unexercised on an unexercised job.
- **`generate` on this container's Node reorders plural keys** in most `messages/*.json` files (details under SHELL-8).

Coordinator's resume note: `scripts/fuzz.sh` and the fuzzing doc were already done when it arrived. My `docs/STATE_MACHINES.md` edits are confined to §1.2 and §1.4.

---

### BUILD-3 — real-browser UI tests skipped everywhere and still green
Disposition: fixed.
Verification of the finding: both files had `skip: !process.env.KIWIX_TEST_CHROME && …`; no workflow set the variable; without it both reported `# skipped 2`, exit 0.
Change:
- `scripts/test-frontend-surfaces.mjs` — the `skip:` line only: `!process.env.KIWIX_TEST_CHROME && !process.env.CI && …`.
- `scripts/test-startup-navigation.mjs` — same skip line, plus an `assert.ok(process.env.KIWIX_TEST_CHROME, …)` at the top of the body for a clear message.
- `.github/workflows/checks.yml`, `frontend` job — new step before "JavaScript tests". It finds `google-chrome`, `chromium` or `chromium-browser`, fails if none exists, prints the version, and writes a wrapper that execs it with `--no-sandbox` into `$RUNNER_TEMP`, exported as `KIWIX_TEST_CHROME`.
- Why `--no-sandbox`: it does not depend on whether the runner's AppArmor user-namespace restriction applies to that binary. The pages are the checkout's own frontend on loopback, which the same job already runs unsandboxed under Node.

Tests (all observed):
- `CI=true`, no browser: `# pass 0 # fail 2`, exit 1 (puppeteer's "`executablePath` or `channel` must be specified" and my assertion message).
- No `CI`, no browser: `# skipped 2`, as before.
- The workflow step's script, extracted from the YAML and run with a fake `RUNNER_TEMP`/`GITHUB_ENV`: exit 0 with a browser on `PATH`, exit 1 with the message when none.
- With the generated wrapper and `CI=true`: `# tests 7 # pass 7 # fail 0 # skipped 0`.

Not exercised: the step on a real runner. The public runner-image readme (image 20261122.23.1) lists Google Chrome 158.0.8127.36, the same version as the local Chromium. Whether Chrome's sandbox would work there without the flag is unknown.

### BUILD-4 — Rust tests never run on PRs; Windows tests never run; clippy lacks `--tests`; AppImage test undiscovered
Disposition: fixed (Linux PR tests, Linux clippy `--tests`, AppImage test, Windows step); Android `--tests` not added, as instructed.
Verification of the finding: confirmed in both workflows. Actions history for `build.yml`: at d98cac2 (run 46069449619) "Clippy" took 1 m 49 s and "Rust tests" 4 m 06 s on hosted `ubuntu-24.04`, both green.
Change:
- `checks.yml` new job `linux-tests` (`needs: frontend`, same draft `if:` as its neighbours, 45 min timeout). It uses `linux-typecheck`'s apt list plus `xvfb`, the `frontend-dist` artifact and the empty sidecar placeholder. The command is the one `build.yml` already runs green: `xvfb-run --auto-servernum cargo test --locked --manifest-path src-tauri/Cargo.toml --lib --features custom-protocol`.
- `checks.yml` `linux-typecheck`: clippy now `--lib --tests`.
- `checks.yml` `frontend`: new step `python scripts/test-linux-appimage.py`.
- `checks.yml` header comment updated: it said host tests run only in the package workflow.
- `build.yml` `windows` job: new step "Rust tests on the Windows target" before "Build NSIS installer", 60 min step timeout. `docs/WINDOWS-BUILD-PROVISIONING.md` does document the exact commands and prerequisites, so the step runs `npm ci`, `npm run build`, puts `$CARGO_TARGET_DIR\x86_64-pc-windows-msvc\debug` on `PATH` for the speech DLLs, then the two documented `cargo test … --target x86_64-pc-windows-msvc` commands. No secrets are in scope at that step.

Tests: `python3 scripts/test-linux-appimage.py` → `Ran 1 test … OK`. Both workflow files parse as YAML; I re-read `needs:`, artifact and `if:` against the neighbouring jobs.
Not exercised:
- No cargo was run against `src-tauri` (lane rule), so nothing in these jobs ran here.
- `linux-tests` differs from the green `build.yml` job in two ways: placeholder sidecar instead of a built one, and a different apt list. Your baseline here covered the placeholder, but without `--features custom-protocol` and without xvfb.
- Linux clippy `--lib --tests` with `KIWIX_TYPECHECK_ONLY=1` is a combination nobody has run. Your exercised command has no env var; the Windows job uses the same combination on its own target.
- The Windows step is entirely unexercised. Also, `docs/windows-agent-tools/env.ps1` notes that profile-privacy tests reject broadly shared temp folders, so the runner account's `TEMP` ACL may matter.

### CORE-10 / BUILD-10 — `docs/ERRORS.md` stale; generator writes on any argument
Disposition: fixed. `docs/ERRORS.md` was not regenerated or touched (md5 checked before and after every run).
Verification of the finding: the base generator run in a scratch copy with `--help` rewrote the file (md5 changed, exit 0).
Change, `scripts/generate-error-docs.mjs`:
- Restructured into exported `renderDocument`, `buildDocument`, `describeDifference`, `parseArguments`, with a main guard.
- No argument writes, as before. `--check` builds in memory, compares, prints counts and the first 10 rows each of added and removed, exits 1, never writes. `--help`/`-h` prints usage. Anything else prints the usage and exits 2 without writing.
- **Line numbers are dropped.** The Source column is now the file only, and rows are sorted by file then literal (code-unit order, `/` separators). The reviewer measured 2,896 of 3,191 rows with stale line numbers after eight days, so a line-number check would fail on nearly every Rust edit. Sorting also removes dependence on `readdir` order, which differs between hosts. The cost is that you grep for the literal instead of jumping to a line.
- The generated header text changes accordingly, so the regenerated file will be a full rewrite.

`package.json`: added `check:errors`. New `scripts/test-error-docs.mjs` (4 tests); it runs in CI through the existing `node --test scripts/test-*.mjs`.
Tests:
- "docs/ERRORS.md matches the error copy in the current sources" **currently fails** on the stale file, as required (3,631 rows the file lacks, 3,191 it should not have).
- The other three pass: help and bad arguments do not write; no line numbers and order-independence; the difference summary names added and removed literals.
- In a scratch copy: write then `--check` → exit 0 and 4/4 pass; two lines inserted above an error string → still exit 0; one literal reworded → exit 1 naming that row.

Not exercised: Windows path normalisation (`path.sep` → `/`).

### SHELL-8 — UI strings missing from the localization inventory; no usable gate
Disposition: fixed for the gate and allow-list; handoff for the data (`src/locales/**` is not mine).
Verification of the finding: `node scripts/localization.mjs audit` exits 1 with 44,154 "missing new copy", 71 "Uncollected UI key" (others have added strings since the review's 64) and 2 "Untranslated JSX".
Change, `scripts/localization.mjs`:
- `inventory()` is exported and takes an optional source directory.
- New exported `inventoryFailures(manifest, found)` returns exactly the three named classes. `audit` now calls it; its output is otherwise unchanged.
- New command `inventory` (read-only, exits 1 on failures) and `package.json` `check:localization`.
- Allow-list `productNames = {"Zimru", "zimfile-xapian"}` for JSX text. This is the right place: they are crate names and must stay untranslated, and wrapping them in `t()` would add two permanently untranslatable sources. No change is needed in `Settings.tsx`.
- **One addition beyond the brief:** literal `t()` sources in `src/i18n.tsx` are now collected (its dynamic calls and JSX are still not reported). Without this, `generate` deletes "The interface language could not be loaded." from `messages/en.json`, because that string was hand-added, is used only at `i18n.tsx:288`, and the inventory skipped the file. With it, the current count is 74 uncollected.

New `scripts/test-localization-inventory.mjs` (2 tests).
Tests:
- "every UI string in the sources is collected in the localization manifest" **currently fails** (74 uncollected), as required.
- The synthetic test passes. I observed it fail against the base inventory logic (the `i18n.tsx` literal is not collected there).
- In a scratch copy after `generate`: `inventory` exits 0 and inventory + resources tests are 12/12.

**What `node scripts/localization.mjs generate` does** (no `--qt-root`/`--apple-root`), from reading it and running it twice in an isolated copy on Node 22.23.5:
- **Offline.** No network; `fetch` exists only in `translate`. It needs `node_modules/typescript`.
- **Deterministic for a given Node/ICU.** A second run was byte-identical. Across Node versions it is not byte-stable: plural-category key order comes from `Intl.PluralRules`. The committed files have `zero, one, two, few, many, other`; this Node writes `few, many, one, two, zero, other`. That reorders plural objects in 62 of 74 `messages/*.json`, `catalogs.json` and `manifest.pluralCoverage`. Values are unchanged (0 changed values). `manifest.dynamic`/`untranslated` are in `readdir` order (both empty now).
- **Reads only:** `machine.json`, `plural-sources.json`, `plural-translations.json`, `community-provenance.json`, `translation-errors.json`, `model-availability.json`, `extra-en-*.json`.
- **Rewrites under `src/locales/`:** `inherited.json` and `inherited-provenance.json` (byte-identical here), `catalogs.json`, every `messages/<code>.json`, `locale-index.json`, `manifest.json`, `provenance.json`, `translation-status.json`.
- **Rewrites outside it:** `src-tauri/native-locales/*.lproj/InfoPlist.strings`, the Android `values-b+*/kiwix_widget_strings.xml` and `xml/kiwix_locales_config.xml` (all byte-identical here).
- **Does it drop translations or provenance?** Nothing in the input stores is lost. Runtime catalogs keep only sources the UI still uses.
- **Effect on the current tree:**
  - Sources go 1552 → 1610 (74 added, 16 removed; the removed ones are obsolete "neural voice" strings and `{count} bytes copied`).
  - `en.json` gains 70 and loses 16.
  - `{count} bytes copied` leaves 59 locales.
  - "Build" gains inherited translations in 32 locales.
  - Each locale's `missingNew` grows (fr 311 → 369) and coverage drops; the new strings fall back to English.
  - 15 `"source"` provenance entries go, all for removed sources.
- **It never translates.** Only `translate` does, by calling the public Google Translate endpoint; do not run that as part of regeneration.

Not exercised: `generate` in the real tree; `--qt-root`/`--apple-root` refresh; `translate`.

### BUILD-6 — eight documentation contradictions
Disposition: fixed, all eight verified against code first.
Change:
1. `docs/PACKAGING.md`: the script always signs the staged bytes and uses a `.sig` only under `--no-sign --public-key` (`make-update-manifest.mjs` `planArtifacts`/`prepareArtifact`).
2. PACKAGING: "public key is a placeholder" → empty public key and no endpoint (`tauri.conf.json` has `"pubkey": ""`, `"endpoints": []`).
3. PACKAGING code comment: key and endpoint come from `KIWIX_UPDATER_PUBLIC_KEY` and the `.updater-release.json` overlay written by `stage-update-release.mjs --configure`.
4. PACKAGING: "No workflow here creates a release" → the `release` job of `build.yml` creates a draft; nothing publishes.
5. PACKAGING: staged name is `<target>-<version>-Kiwix.app.tar.gz`.
6. `docs/UPDATE.md` (two places): no `app.restart()`. The command calls `app.exit(0)` and the Quit path relaunches via `request_restart()` in `lib.rs` after retirement; on Windows the installer starts and the app exits.
7. `docs/DEV-ITERATION.md`: `dev.sh` sets PATH and Java/Android defaults and resolves no desktop SDK.
8. `docs/PLATFORM-GAPS.md`: Flatpak bullet removed (README has no "flatpak"). The other two bullets in that section were checked and are still true.

`scripts/make-update-manifest.mjs`: the "No signing key supplied" error no longer suggests that a `.sig` alone is enough. The text is asserted nowhere.
Tests: `node --test scripts/test-update-manifest.mjs` → `# pass 17 # fail 0`.
Not exercised: none beyond the above.

### CORE-11 — `docs/STATE_MACHINES.md` §1.2 and §1.4
Disposition: fixed, with one correction to the reviewer.
Verification of the finding: each claim read in `service.rs`, `service/dispatch.rs`, `store.rs`.
Change (four small replacements, §1.2 and §1.4 only):
- **Two epochs.** `launch_generation()` returns `privacy_generation`. Both are advanced only under the writer; mobile background reader retirement advances only the browsing epoch.
- **Lock order as the code has it:** `update_admission` → `mobile.transfer_gate` → `settings_writes` → `privacy_gate`. The reviewer's suggested line had `settings_writes` before `transfer_gate`; `Service::operation` acquires them the other way round.
- **No nested read lease**, stated as a rule ("must not"), because CORE-3 is a live violation being fixed elsewhere.
- **Full lease-free list**, including `catalog_facets`, `catalog_torrent`, `map_recipe_metadata` and `update_install`, with what each actually does. The old sentence over-claimed: `map_recipe_metadata` checks the generation once with no re-check, and `sensor_location` takes no lease.
- **§1.4:** coalesced browsing telemetry (`update_browsing` from `record_visit`, 5 s window, `browsing_pending`) and its flush points (next transaction, first refresh after the window, `flush_browsing` in `Service::shutdown` and `mobile_transition`, before a private cutover).

Tests: none (documentation).
Not exercised: I did not trace every caller of every `transfer_gate` holder, so the ordering sentence names only the sites I read.

### SERVE-7 — fuzz target never decodes a cluster; no search-index target
Disposition: fixed; type-checked only. **No fuzzing was run.**
Verification of the finding: the target stopped at dirents and `check()`, and used different limits from `archive.rs` `open_source`.
Change:
- `src-tauri/fuzz/fuzz_targets/zim_archive.rs`: uses the app's `ArchiveLimits`/`ClusterReadLimits` and the desktop 32 MiB cluster-cache budget. It resolves the first `min(entry_count, 64)` entries (`get_item(true)`, `size()`, `read_at`), calls `title_prefix_range(b'C', "a")`, and `validate_cluster_references` on the first four clusters. Every API exists in the pinned zimru `cbafcb4`.
- New `fuzz_targets/search_index.rs`: the whole input is one Glass index, opened with the app's `ReaderLimits { max_query_bytes: 64 KiB, .. }`, then `metadata`, one `search_page` and one `suggest_page`.
- `fuzz/Cargo.toml`: adds `zimfile-xapian` at the app's pinned rev and a `[[bin]]`. `Cargo.lock` gains 4 packages, the same versions as the app's lock.
- New `src-tauri/fuzz/extract-index-seeds.py` and a line in `scripts/fuzz.sh`: seeds are copied out of `tests/fixtures/indexed_search.zim` at run time; no binary seeds are committed. Run here: it produces 24,576- and 32,768-byte blobs that start with the engine's Glass magic.
- `docs/SANITIZERS-AND-FUZZING.md`: states that the recorded 100,000-run / 1,549-edge result predates these changes and that neither target has been run since.

Tests: `CARGO_TARGET_DIR=<scratch>/fuzz-target cargo check --manifest-path src-tauri/fuzz/Cargo.toml` → `Finished dev profile`, both times. `bash -n scripts/fuzz.sh` ok.
Not exercised: any fuzz run (no nightly, no cargo-fuzz).

### APPLE-9 — iOS plugin XCTest suite not in CI
Disposition: documented as local-only, as instructed; no CI job invented.
Verification of the finding: no workflow references the suite. `Package.swift` depends on `../.tauri/tauri-api`, which is git-ignored and copied by `tauri-plugin` 2.7.2's build script only for an iOS-target build on macOS.
Change: `docs/SANITIZERS-AND-FUZZING.md`, new section "iOS plugin XCTest suite: local only" — what it is, that no workflow builds or runs it, why it cannot resolve from a clean checkout, how to run it, and why CI does not.
Tests: none.
Not exercised: the run instructions themselves. The section says so, and tells the reader to find the scheme with `xcodebuild -list` instead of naming one, because the repository records no earlier command for this suite.

### BUILD-11 — floating toolchain and mutable action refs
Disposition: documented residual; nothing pinned.
Verification of the finding: 8 uses of `dtolnay/rust-toolchain@stable` (5 in `checks.yml` including my new job, 3 in `build.yml`), 2 of `cargo-deny-action@v2`, no `rust-toolchain.toml`.
Change: `docs/DEPENDENCIES.md` "To do" — an open item with the state, the consequences, why it was not done, and five remediation steps. One step says to confirm in the action's README that a SHA ref needs a `toolchain:` input; that is my recollection, not something I verified.
Tests: none.
Not exercised: n/a.

---

## Commands run (final result lines)
- `node --test scripts/test-*.mjs` → `# tests 485 # pass 475 # fail 2 # skipped 8`. The two failures are exactly my two "until regenerated" tests.
- Same with `CI=true KIWIX_TEST_CHROME=<wrapper>` → `# tests 490 # pass 483 # fail 6 # skipped 1`: my two, plus four in `test-shell-surfaces.mjs`.
- `test-shell-surfaces.mjs` + `test-nearby-sharing-surfaces.mjs` alone with a browser → `# pass 1 # fail 7`.
- `test-frontend-surfaces.mjs` + `test-startup-navigation.mjs` with a browser → `# pass 7 # fail 0 # skipped 0`.
- `python3 -m unittest discover -s scripts -p 'test_*.py'` → `Ran 182 tests … OK (skipped=2)`.
- `python3 scripts/test-linux-appimage.py` → `Ran 1 test … OK`.
- YAML load of `checks.yml` and `build.yml` → ok.
- `node scripts/check-unsafe-census.mjs` → `75 matches / 75 blocks … every item documented`.
- `cargo check --manifest-path src-tauri/fuzz/Cargo.toml` (scratch target dir) → `Finished`.
- Not run: any cargo against `src-tauri` or `kiwix-mcp`, `npm run build`, `tsc`.

## Files changed
All under `/home/user/kiwix-tauri/`:
- `.github/workflows/checks.yml`, `.github/workflows/build.yml`
- `docs/PACKAGING.md`, `docs/UPDATE.md`, `docs/DEV-ITERATION.md`, `docs/PLATFORM-GAPS.md`, `docs/DEPENDENCIES.md`, `docs/SANITIZERS-AND-FUZZING.md`, `docs/STATE_MACHINES.md` (§1.2, §1.4)
- `scripts/generate-error-docs.mjs`, `scripts/localization.mjs`, `scripts/make-update-manifest.mjs`, `scripts/fuzz.sh`
- `scripts/test-startup-navigation.mjs`, `scripts/test-frontend-surfaces.mjs` (skip line only)
- `package.json` (two scripts)
- `src-tauri/fuzz/Cargo.toml`, `src-tauri/fuzz/Cargo.lock`, `src-tauri/fuzz/fuzz_targets/zim_archive.rs`
- New: `scripts/test-error-docs.mjs`, `scripts/test-localization-inventory.mjs`, `src-tauri/fuzz/fuzz_targets/search_index.rs`, `src-tauri/fuzz/extract-index-seeds.py`

New user-visible strings: none in the app. CLI text only (generator usage, the reworded manifest error, the `inventory` hint).

## Integration owner: end-of-run commands
1. `npm run docs:errors`, then `npm run check:errors` (expect "docs/ERRORS.md matches the current sources.") and `node --test scripts/test-error-docs.mjs` (expect 4 pass). The file is fully rewritten because the format changed.
2. Before `generate`, add to `src/locales/plural-sources.json`: `"Used today: {count} requests": {"one": "Used today: {count} request", "other": "Used today: {count} requests"}`. The call site already passes a numeric count, so `t()` picks the form.
3. `node scripts/localization.mjs generate`, then `npm run check:localization` (expect `"failures": []`, exit 0) and `node --test scripts/test-localization-inventory.mjs scripts/test-localization-resources.mjs` (expect 12 pass). Do not run `translate`.
4. Then `npm run build` (the catalogs are embedded) and the full Node suite: expect 0 failures without a browser.

## Handoffs
- **Shell/nearby lanes:** `test-shell-surfaces.mjs` and `test-nearby-sharing-surfaces.mjs` must pass with `KIWIX_TEST_CHROME` set before push; they now gate CI.
- **`Settings.tsx` owner:** no change needed for "Zimru"/"zimfile-xapian". Separately, `<dt>{t("Build")}</dt>` will pick up 32 inherited translations of "Build" after `generate`, some in the wrong sense (French "Construire"). A more specific source such as "Build number" would avoid that.
- **Anyone matching text:** the `make-update-manifest.mjs` "No signing key supplied" message changed.
- **`docs/WINDOWS-BUILD-PROVISIONING.md`** (not mine) could note that `build.yml` now runs the two test commands.

## Doubts
- `generate`'s plural-key reordering on this Node makes a large, value-neutral diff. If you would rather keep the committed order, run it on the Node version that produced the committed files; I did not identify which.
- The inventory gate covers only the three named classes. A formatted plural count, a placeholder mismatch and the 25 existing `{count}` sources with no English plural stay ungated.
- The skip rule treats any non-empty `CI` as CI, including `CI=false`.
- `checks.yml` ignores `docs/**` paths, so a PR that edits only `docs/ERRORS.md` by hand is not checked.
- The Windows test step may fail for environmental reasons and block packaging. I left it gating, because a non-gating test step would be the silent success the brief forbids.
- The fuzz targets use the app's real limits (up to 64 MiB decoded per cluster, entry count unbounded). That is faithful, but may make individual executions slow, and without the app's `validate_layout` some findings may be unreachable in the app.
