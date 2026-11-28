## Verdict for lane cidocs: BLOCKING findings exist

### CIDOCS-R1 — The new `linux-tests` job fails every run: a Rust test needs Node with `typescript`, and the job never runs `npm ci`
Severity: Blocking
Fix under review: BUILD-4
Evidence class: reproduced (Node side) + source-traced (the test is compiled on Linux)
Location: `.github/workflows/checks.yml` job `linux-tests`; the test is `src-tauri/src/native_pdf/live.rs:271` `bridge_audio_captures_prepare_with_the_real_pdf_sanitizer`
Trigger: any PR or push that runs Checks.
Causal path:
- `native_pdf.rs:12` declares `mod live;` with no cfg, and `live.rs:223` has `#[cfg(test)] mod tests`, so `cargo test --lib` on Linux runs this test.
- The test calls `Command::new("node")…args(["scripts/test-reader-live-export.mjs", "--audio-snapshots"])` and asserts success. Its own expect text says it "requires Node.js and the installed frontend dependencies".
- `test-reader-live-export.mjs:5` has `import ts from "typescript"`.
- `linux-tests` has no `actions/setup-node` and no `npm ci`; it only downloads the `frontend-dist` artifact. The runner's preinstalled Node has no `node_modules`.
- I ran the script in a scratch copy without `node_modules`: `ERR_MODULE_NOT_FOUND: Cannot find package 'typescript'`, exit 1. The test therefore fails and the job is red permanently.
- `build.yml`'s Linux job passes only because it runs `npm ci` before cargo. The comment "the same test command as the Linux job in build.yml" hides that difference.
Smallest correct fix: before "Rust unit tests", add `actions/setup-node@v4` (node-version 22, npm cache) and `run: npm ci`. Keep the downloaded `dist` and do not run `npm run build`: it would empty `dist`.

### CIDOCS-R2 — `generate` writes plural categories in the host V8's order, so the same inputs give different files on Node 22 and Node 24
Severity: Should-fix
Fix under review: SHELL-8 (regeneration handoff)
Evidence class: reproduced
Location: `scripts/localization.mjs:579-580` (`generate`, `pluralCoverage`/catalog plural objects); same pattern at `:984-986` (`audit`)
Trigger: the integration owner runs `node scripts/localization.mjs generate` with this container's Node 22.23.5.
Causal path:
- `categories = new Intl.PluralRules(code).resolvedOptions().pluralCategories` is used as the object key order.
- Node 22.23.5 (ICU 78.3) returns `["few","many","one","two","zero","other"]` for ar. `/opt/node24` (ICU 78.3) returns `["zero","one","two","few","many","other"]`, which is the committed order.
- In a scratch copy, the Node 22 and Node 24 `generate` outputs differ in 20 files (`catalogs.json`, `manifest.json`, `translation-status.json`, 17 `messages/*.json`), with no value changes.
Smallest correct fix:
- Sort the categories into CLDR order: `const CLDR_ORDER = ["zero","one","two","few","many","other"]; categories = [...pluralCategories].sort((a, b) => CLDR_ORDER.indexOf(a) - CLDR_ORDER.indexOf(b))`. Apply it at both sites.
- I applied exactly this in the scratch copy and ran it on Node 22. Its output was byte-identical to the Node 24 output (`diff -rq` empty).
- The runtime selects forms by key, so the order does not affect behaviour.

### CIDOCS-R3 — The error-docs gate depends on the host's path separator: on Windows, `--check` and `test-error-docs.mjs` fail
Severity: Should-fix
Fix under review: CORE-10 / BUILD-10 (the claim that the output is host-independent)
Evidence class: reproduced by simulation
Location: `scripts/error-inventory.mjs:192-196` (`rustInventory` test-file filter `!/(^|\/)(.*_tests|tests)\.rs$/.test(file)`), used by `scripts/generate-error-docs.mjs`
Trigger: `node --test scripts/test-*.mjs` or `npm run check:errors` on Windows. `docs/WINDOWS-BUILD-PROVISIONING.md` and `run-gate.ps1` run that suite there.
Causal path:
- `file` is an absolute path with `\` separators on Windows, so the `\/` branch never matches `…\localswarm\tests.rs` (which has no `_tests` suffix).
- `generate-error-docs.mjs:48` normalises separators only for the displayed file, after this filter has already run.
- In a scratch copy I fed the filter backslash paths. `--check` then reported 4 extra rows from `src-tauri/src/localswarm/tests.rs` (for example "LocalSwarm source read failed", "Unexpected fixture chunk"), exit 1.
Smallest correct fix: in `rustInventory`, compute `relative = path.relative(root, file).split(path.sep).join("/")` and apply the exclusion regex to `relative`. Do the same in the other inventories, so every stored `file` is already `/`-separated.

### CIDOCS-R4 — §1.2 now says `map_recipe_metadata` is checked only once before its fetch; the code re-checks it after the fetch
Severity: Should-fix (documentation)
Fix under review: CORE-11
Evidence class: source-traced
Location: `docs/STATE_MACHINES.md` §1.2 invariant ("`map_recipe_metadata` checks the caller's generation once, before its fetch"); `src-tauri/src/service.rs:2850-2857` and `:2919-2929`
Trigger: a reader concludes the result can outlive a privacy cutover and "fixes" it, or trusts the implementer's claim that the old text over-claimed.
Causal path:
- For `map_recipe_metadata` | `map_prepare`, `operation` captures `Some(self.launch_generation())` under a short `privacy_gate.read()`.
- After `dispatch_inner`, it takes `_diagnostic_guard = Some(self.privacy_gate.read().await)` and returns `Err("Browsing session expired after a privacy transition")` when `generation != self.launch_generation()`.
- So it re-checks under a short read lease before returning, as the catalog commands do.
Smallest correct fix: say that `map_recipe_metadata` checks the caller's generation before its fetch, and that `operation` re-checks the admission epoch under a short read lease after it. Or simply list it with the commands that re-check.

### CIDOCS-R5 — §1.4's list of flush points leaves out the Windows installer hook that §1.5 relies on
Severity: Nit
Fix under review: CORE-11
Evidence class: source-traced
Location: `docs/STATE_MACHINES.md` §1.4 ("`Store::flush_browsing` (called by `Service::shutdown` and by the mobile background transition)"); `src-tauri/src/update.rs:1067` `before_installer_exit` calls `service.store.flush_browsing()` at `:1081`
Causal path: the updater lane's §1.5 text cites "`Store::flush_browsing`, §1.4" for this hook, but §1.4 names only two callers. This is not a contradiction, only an incomplete list.
Smallest correct fix: add "and by the Windows installer hook before the updater exits (§1.5)".

### CIDOCS-R6 — The new `checks.yml` header says Checks is "the only workflow that runs on pull requests"
Severity: Nit
Fix under review: BUILD-4
Evidence class: source-traced
Location: `.github/workflows/checks.yml:3-5`
Causal path: `dependencies.yml` also has a `pull_request` trigger, for dependency-manifest paths.
Smallest correct fix: "the only workflow that runs tests on pull requests".

### CIDOCS-R7 — A PR that only hand-edits `docs/ERRORS.md` triggers no check
Severity: Nit (this is the implementer's doubt; a cheap fix exists)
Fix under review: CORE-10 / BUILD-10
Evidence class: source-traced
Location: `.github/workflows/checks.yml:9-22` (`paths-ignore: docs/**, **/*.md`)
Trigger: a docs-only PR edits `docs/ERRORS.md`. The next code PR then fails `test-error-docs.mjs`, which blames the wrong author.
Smallest correct fix:
- For both events, replace `paths-ignore` with `paths: ['**', '!docs/**', '!**/*.md', '!LICENSE', '!COPYING', '!.gitignore', 'docs/ERRORS.md']`.
- GitHub evaluates `paths` in order, and a later positive pattern re-includes a path.
- Generated locale files are JSON, so they are already not ignored.

### CIDOCS-R8 — The browser tests are now gating, but one of them is flaky, and every Rust job is skipped behind a red frontend job
Severity: Nit (handoff to the shell lane)
Fix under review: BUILD-3
Evidence class: reproduced
Location: `.github/workflows/checks.yml` `frontend` job (Chrome step) together with `needs: frontend` on `linux-tests` and all clippy jobs; `scripts/test-shell-surfaces.mjs:207`
Causal path:
- My full run with `CI=true` and the wrapper browser had 3 failures (load about 4): the two "until regenerated" tests, plus "a failure inside a modal is shown, announced and dismissed above it, with one owner" (`Waiting failed: 10000ms exceeded`).
- When the frontend job fails, `linux-tests` and the Android, Windows and Linux clippy jobs are skipped, so one UI timeout hides all Rust results.
Smallest correct fix:
- The shell lane must make that test reliable under CI load.
- Optionally, split the build and artifact upload into its own job, so the Rust jobs do not depend on the JS test outcome.

**Per-fix assessment**
- BUILD-3: fixes it. The skip rule fails closed under CI, and the Chrome step fails closed without a browser and exports the wrapper to later steps of the same job. Other lanes' browser tests now gate (R8).
- BUILD-4: does not fix it as written, because `linux-tests` fails on every run (R1). Clippy `--lib --tests`, the AppImage test step and the Windows step are fine as YAML.
- BUILD-6: fixes it. All eight items were checked against `make-update-manifest.mjs`, `tauri.conf.json`, `stage-update-release.mjs`, `update.rs:1007-1022`, `dev.sh` and README.
- BUILD-10 / CORE-10: fixes it, but the result is host-dependent on Windows (R3) and docs-only edits are unchecked (R7). `--check`, `--help` and unknown arguments never write. Output is deterministic across runs, locales and Node versions on Linux. The test cannot pass vacuously, because a zero-source scan would differ from the committed file.
- BUILD-11: fixes it as a documented residual. The counts are right: 5 + 3 toolchain uses, 2 deny-action uses.
- CORE-11: fixes it, but see R4 and R5.
  - The two-epoch statement and the increment sites are correct: `dispatch.rs:1425` advances both; the mobile retirement sites in `mobile_service.rs` and `mobile_speech.rs` advance only browsing, under the writer.
  - The lock-order line matches `Service::operation:2670-2760`, and every other acquirer I found takes `update_admission` → `transfer_gate` → privacy.
  - The lease-free list exactly matches the `matches!` at `service.rs:2778`.
  - The §1.4 flush points match `store.rs:2929-3120`.
- SERVE-7: fixes it, type-check only.
  - The limits and the 32 MiB cache budget match `archive.rs:388-408`, and `MAX_QUERY` (64 KiB) matches `archive_search.rs:234,304`.
  - The fuzz lock additions match the app's lock versions.
  - The seed extractor writes only into its output argument; `fuzz.sh` passes the git-ignored `src-tauri/fuzz/corpus/search_index`. It fails with an error, not an empty seed, on a fixture without both indexes (`small.zim`).
- APPLE-9: fixes it as documentation, as instructed. The run steps were not executed; the doc says so.
- SHELL-8: fixes the gate. It catches a newly added `t("…")` (scratch copy: `inventory` exit 1, "Uncollected UI key: A brand new reviewer string") and stays quiet on the English-fallback class after `generate`. The `i18n.tsx` extension collects only UI literals (`The interface language could not be loaded.`, `Try again`, `Dismiss`, `Unknown date`, `Unknown size`). R2 should be fixed before regenerating.

**Implementer's doubts**
- Plural reordering: a real problem with a cheap fix. Fix it now (R2), rather than looking for the right Node version.
- Inventory covers only three classes: acceptable. That is the finding's stated scope, and the full `audit` keeps the rest.
- `CI=false` treated as CI: acceptable. It fails closed, and only someone who explicitly sets `CI=false` without a browser is affected.
- `docs/**` ignored: a minor gap with a cheap fix (R7).
- The Windows test step gating packaging: acceptable and correct. A non-gating test would be silent success. It sees no secrets: only the job `CARGO_TARGET_DIR` and the workflow `KIWIX_RELEASE_VERSION`, and nothing reads the latter in tests. It needs no sidecar, because the Windows config has no `externalBin`. Whether the runner account's TEMP ACL passes the profile-privacy tests is unknown until the job runs.
- Fuzz targets use the app's real limits: acceptable. This is faithful, and `validate_layout` is documented as not applied.
- Other lanes' browser tests: confirmed. 1 failure in my run (R8).

**Workflow points checked with no finding**
- `linux-tests` downloads `frontend-dist` to `./dist`, which is what `build.rs`/`server.rs` embed (`../dist` from `src-tauri`).
- `KIWIX_TYPECHECK_ONLY=1` with `--tests` under clippy does not link the crate. `rustc-link-lib` applies only at link time, and the Windows job already uses the same combination.
- The wrapper quoting is correct (`exec "/usr/bin/google-chrome" --no-sandbox "$@"`), and the file is executable.
- pwsh `$LASTEXITCODE` checks follow the existing pattern.
- No `if:` change makes an existing gating step non-gating.
- `paths-ignore` still triggers on `scripts/**` and on the Rust sources.

**What I ran**
- `node --test scripts/test-error-docs.mjs scripts/test-localization-inventory.mjs scripts/test-localization-resources.mjs scripts/test-update-manifest.mjs scripts/test-startup-navigation.mjs` → `# tests 34 # pass 31 # fail 2 # skipped 1`. The 2 failures are the "until regenerated" tests.
- Full suite with `CI=true KIWIX_TEST_CHROME=chrome-nosandbox.sh` → `# tests 501 # pass 497 # fail 3 # skipped 1`, 126 s.
- Startup and frontend browser tests, by environment:
  - `CI=true`, no browser → `pass 0 fail 2`
  - with the wrapper → `pass 7 fail 0`
  - `CI=false` → `fail 1`
  - `CI` unset → `skipped 1`
- Both workflow files parse with PyYAML. `python3 scripts/test-linux-appimage.py` → `Ran 1 test … OK`.
- Chrome step script extracted from the YAML: exit 0 and correct `GITHUB_ENV` and wrapper with a browser; exit 1 and an empty `GITHUB_ENV` without one.
- Generator experiments in the scratch copy:
  - every non-write argument (`--help`, `-h`, `--chek`, `--check extra`, `--CHECK`, a path, an empty string) left the md5 unchanged, with exit 0 or 2 as designed
  - write, then `--check` → 0; identical md5 under `LC_ALL=tr_TR` and `TZ=Asia/Tokyo`; Node 24 `--check` → 0
  - 4/4 tests pass after regeneration
- Localization experiments in the scratch copy: `generate` on Node 22 and on Node 24; the CLDR-sort patch made them byte-identical; `inventory` gave exit 0, then exit 1 with a new string; inventory + resources tests 12/12 after regeneration.
- Seed extractor run into scratch: 24,576 + 32,768-byte Glass blobs; error on `small.zim`.
- `CARGO_TARGET_DIR=…/adv-cidocs/fuzz-target cargo check --locked --manifest-path src-tauri/fuzz/Cargo.toml` → `Finished`.
- The repository's `docs/ERRORS.md` md5 was unchanged throughout (a04f3033…). All scratch output is under `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-cidocs/`.

**What I could not verify**
- Any GitHub Actions run.
- The Windows runner: the step, TEMP ACLs and timing.
- Chrome's sandbox behaviour on the hosted image.
- Any fuzz execution (no nightly or cargo-fuzz here).
- The iOS XCTest instructions.
- `cargo test` for `src-tauri` itself: not run under this lane's rules. R1 is traced from source, plus the reproduced Node failure.
