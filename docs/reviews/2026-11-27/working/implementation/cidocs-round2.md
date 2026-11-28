# Lane `cidocs` — follow-up to the adversarial review (changes only)

All eight review items are addressed; nothing in the repo's `docs/ERRORS.md` (md5 still `a04f3033…`) or `src/locales/**` was regenerated. No GitHub Actions run was possible, so every workflow change is unexercised on a runner. The two "until regenerated" tests still fail by design and are the only failures in the Node tests I re-ran.

---

### CIDOCS-R1 — `linux-tests` needs Node and `node_modules`
Disposition: fixed.
Verification of the finding: `native_pdf/live.rs` `bridge_audio_captures_prepare_with_the_real_pdf_sanitizer` runs `node scripts/test-reader-live-export.mjs --audio-snapshots` from the repository root; the script imports `typescript` and reads `src/reader-bridge.js`. With the installed dependencies it exits 0 here.
Change: `.github/workflows/checks.yml`, job `linux-tests` — added `actions/setup-node@v4` (Node 22, npm cache, same as the frontend job) and an `npm ci` step after the `dist` download and before cargo. No `npm run build`; a comment says why. The job comment now names the Node-backed test.

Every process spawn in `src-tauri/src` (`grep 'Command::new'`), and what the job provides:

| Site | What it runs | Needed on the Linux job |
|---|---|---|
| `native_pdf/live.rs:273` (test) | `node scripts/test-reader-live-export.mjs` | Node + `npm ci` — now provided |
| `archive.rs:1899` (test, `cfg(unix)`) | `mkfifo` | coreutils, on the runner image |
| `localswarm/tests.rs:325`, `exports.rs:2529`, `crash.rs:844` (tests) | the test binary itself (`current_exe`) | nothing |
| `update.rs:1832` (test) | the installed AppImage | nothing: `loopback_end_to_end` is `#[ignore]` |
| `spelling.rs:1791/1802` (test) | `/bin/chmod +a` | nothing: `cfg(target_os = "macos")` |
| `instance.rs:160`, `:230` | `osascript`; zenity/kdialog | production code; the tests only inspect the argument lists |

No test shells out to python, npm, git or a shell.
Not exercised: the job itself. The script was run here with the real `node_modules` only.

### CIDOCS-R2 — plural categories in CLDR order
Disposition: fixed.
Change: `scripts/localization.mjs` — new exported `pluralCategories(code)` sorts `Intl.PluralRules(...).resolvedOptions().pluralCategories` into `zero, one, two, few, many, other`; used at both sites (`generate` and `audit`).
Tests:
- New test "plural categories are written in CLDR order whatever order the engine reports" in `scripts/test-localization-inventory.mjs`: passes on Node 22.23.5 and Node 24.22.3.
- Not observed failing against the old code: the old code has no function to call. The engines do differ: for `ar`, Node 22 reports `few,many,one,two,zero,other` and Node 24 reports `zero,one,two,few,many,other`.
- Scratch copies, `generate` with the fixed script on Node 22 and on Node 24: `diff -rq` empty, byte-identical.
- Against the committed files: 1,260 plural objects compared, 0 key-order mismatches; `manifest.pluralCoverage` category order identical for every locale.
- After that scratch `generate`: `inventory` exits 0 with no failures; inventory + resources tests 13/13.

One thing that is still not byte-stable against the committed files, and is not Node-dependent: in 29 `messages/*.json` a few top-level keys move (for example `ar.json` has 11 changed lines). That comes from the current inputs, not from the engine.

### CIDOCS-R3 — error inventory depends on the host path separator
Disposition: fixed.
Change: `scripts/error-inventory.mjs` — new exported `portablePath(relative, separator)` and `isRustTestSource(relative)`. The test-file exclusion now runs on the repo-relative `/`-separated path, and the Rust, frontend and frontend-error-site inventories store that form (the native and locale inventories already used literal `/` paths).
Tests:
- New test "test-only Rust sources are excluded whatever the host's path separator" in `scripts/test-error-docs.mjs`: a backslash path converts and is excluded, near-miss names (`contests.rs`, `tests/helper.rs`) are kept, and the real inventory holds only `src-tauri/src/…` paths with no backslash and no test source.
- Scratch copy: generator output md5 is identical with the base and the new `error-inventory.mjs` (no change on Linux); write then `--check` → exit 0 on Node 22 and Node 24; 5/5 tests pass after regeneration.

Not exercised: a real Windows host. The backslash case is covered only through the `separator` parameter.

### CIDOCS-R4 / R5 — `docs/STATE_MACHINES.md` §1.2 and §1.4
Disposition: fixed; the reviewer was right and my earlier sentence was wrong.
Verification of the finding: `Service::operation` captures `launch_generation()` under a short read lease for `map_recipe_metadata | map_prepare`, and after dispatch takes another short read lease and returns "Browsing session expired after a privacy transition" if it changed. `update.rs` `before_installer_exit` calls `store.flush_browsing()`.
Change:
- §1.2: `map_recipe_metadata` is now listed with the commands that are re-checked under a short read lease, with one sentence on how `operation` does it for the two map commands.
- §1.4: the `flush_browsing` callers now include "the Windows installer hook before the updater exits the process, §1.5".
- §1.1 and §1.5 untouched.

### CIDOCS-R6 — header wording
Disposition: fixed. `checks.yml` header now says "the only workflow that runs tests on pull requests".

### CIDOCS-R7 — docs-only edit of `docs/ERRORS.md` triggers nothing
Disposition: fixed, with two redundant lines beyond the reviewer's list.
Change: for both `push` and `pull_request`, `paths-ignore` is replaced by
`paths: ['**', '.github/**', '.gitattributes', '!docs/**', '!**/*.md', '!LICENSE', '!COPYING', '!.gitignore', 'docs/ERRORS.md']`, with a header comment explaining the order.
- `'.github/**'` and `'.gitattributes'` are my additions. They guard against `'**'` not matching dot-prefixed paths; they are harmless if it does.
- Confidence in the semantics, from GitHub's documentation as I remember it (not re-read here): about 90% that patterns are evaluated in order and a later positive pattern re-includes a path, that the workflow runs if any changed file ends up included, and that `'**'` is documented as the no-filter default.
- Local simulation of that documented algorithm against this list: `docs/ERRORS.md` alone → runs; `docs/UPDATE.md`, `README.md`, `LICENSE`, `.gitignore` alone → do not; `src/…`, `scripts/…`, `.github/workflows/checks.yml`, `.gitattributes`, `src/locales/…json` → run.

Not exercised: GitHub's own evaluation. A one-file PR touching only `docs/ERRORS.md`, and one touching only `checks.yml`, would confirm it.

### CIDOCS-R8 — build split from tests
Disposition: fixed.
Change, `checks.yml`:
- New job `frontend-build` ("Frontend type-check and build"): checkout, Node 22, `npm ci`, `npm run build`, upload `frontend-dist`. It is the only uploader.
- Job `frontend` keeps its id and its display name "Frontend build and tests", now `needs: frontend-build` and downloads `frontend-dist` into `dist` instead of building. It runs the unsafe census, the Chrome step, the JS, Python and AppImage tests. I kept the name in case a required status check refers to it; it still means "build succeeded and tests passed".
- `android-typecheck`, `windows-typecheck`, `linux-typecheck` and `linux-tests` now `needs: frontend-build`, so a failing script or browser test no longer skips them.

Checked mechanically: one upload of `frontend-dist`; five jobs download it and each needs `frontend-build`; every job keeps the draft `if:`.
Not exercised: the artifact hand-off to the `frontend` job on a runner (the same pattern the clippy jobs already use).

---

## Commands run
- PyYAML load of `checks.yml` and `build.yml` → ok.
- Chrome step script re-extracted from the YAML and run → exit 0, wrapper written.
- `node --test` on `test-error-docs`, `test-localization-inventory`, `test-localization-resources`, `test-startup-navigation`, `test-frontend-surfaces`, `test-update-manifest` → `# tests 37 # pass 33 # fail 2 # skipped 2`. The 2 failures are the "until regenerated" tests.
- `CI=true KIWIX_TEST_CHROME=<wrapper> node --test scripts/test-startup-navigation.mjs scripts/test-frontend-surfaces.mjs` → `# tests 7 # pass 7 # fail 0`.
- Scratch experiments as listed under R2 and R3.
- Not run: the full Node suite, any cargo, `npm run build`.

## Files changed in this round
- `/home/user/kiwix-tauri/.github/workflows/checks.yml`
- `/home/user/kiwix-tauri/scripts/localization.mjs`
- `/home/user/kiwix-tauri/scripts/error-inventory.mjs`
- `/home/user/kiwix-tauri/scripts/test-localization-inventory.mjs`
- `/home/user/kiwix-tauri/scripts/test-error-docs.mjs`
- `/home/user/kiwix-tauri/docs/STATE_MACHINES.md` (§1.2, §1.4)

New user-visible strings: none.

## Handoffs / contract changes
- `checks.yml` now has a job id `frontend-build`; anything that names job ids or required checks should know. The `frontend` check name is unchanged.
- The end-of-run commands from my first report are unchanged. After `npm run docs:errors`, `test-error-docs.mjs` should be 5 pass; after `generate`, the inventory + resources tests should be 13 pass.
- `generate` is now safe to run on this container's Node 22: it keeps the committed plural key order.

## Doubts
- The `paths` filter is from memory of the documentation; see the confidence above.
- If branch protection requires a check by name, the new "Frontend type-check and build" check is not in that list; I could not read the repository's protection settings.
- The `frontend` job now waits for `frontend-build`, about a minute later than before, and is skipped if the type-check or build fails.
