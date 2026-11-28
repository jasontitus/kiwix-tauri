## Verdict for lane cidocs (round 2): only non-blocking findings

R1 to R8 are all closed. The two remaining findings are Nits about how much the new tests cover; neither needs to block. Nothing was run on GitHub Actions, so every workflow conclusion below comes from reading the YAML and simulating locally.

### CIDOCS-R9 — The CLDR-order test protects the shared function, not the two places that use it
Severity: Nit
Fix under review: R2
Evidence class: reproduced (I broke the code in a scratch copy and re-ran the tests)
Location: `scripts/test-localization-inventory.mjs` ("plural categories are written in CLDR order…"); `scripts/localization.mjs:83-87` (`pluralCategories`), call sites `:588` (`generate`) and `:992` (`audit`)
Trigger:
- Someone puts the raw `Intl.PluralRules(...).resolvedOptions().pluralCategories` call back at line 588, or the test only ever runs on Node 24.
Causal path:
- I replaced the sort with a no-op. Node 22.23.5 then fails the test (`pass 2 fail 1`), but Node 24.22.3 passes (`pass 3 fail 0`), because its engine already reports CLDR order.
- CI uses Node 22, so a broken function is caught there.
- If line 588 goes back to the raw call, no test notices on any Node version.
Smallest correct fix (optional): a source-level assertion that neither `generate` nor `audit` calls `resolvedOptions().pluralCategories` directly. Or leave it as is.

### CIDOCS-R10 — Two cosmetic leftovers
Severity: Nit
Fix under review: R3, R8
Evidence class: source-traced (first point backed by a reproduced change)
Location: `scripts/test-error-docs.mjs` ("test-only Rust sources are excluded whatever the host's path separator"); `.github/workflows/checks.yml` job `frontend` (name: "Frontend build and tests")
Causal path:
- **Separator test.** On a host that uses `/`, its real-inventory half cannot fail. The pure-function half (`portablePath`, `isRustTestSource`) and the backslash case still can.
  - I did show it fails on a simulated Windows path: in a scratch copy I fed the filter backslash paths, and test 5 failed with `tests.rs` rows appearing.
- **Job name.** The `frontend` job no longer builds; `frontend-build` does. The implementer kept the old name deliberately in case a required status check refers to it.
Smallest correct fix: none required. If no required check uses the name, rename the job to "Frontend tests".

**Per-fix status, R1–R8**
- **R1: closed.**
  - `linux-tests` now runs `setup-node@v4` (Node 22) and `npm ci` after the `dist` download, without `npm run build`. `package.json` has no install hooks that could touch `dist`.
  - Every `Command::new` in `src-tauri/src` is covered: `node` (now has `node_modules`), `mkfifo` (part of the runner image), `current_exe`, `/bin/chmod` (macOS only), `osascript` (production code), and an `#[ignore]`d AppImage test.
- **R2: closed.**
  - In a fresh scratch copy, `generate` on Node 22: all 1,260 plural objects keep the committed key order (0 mismatches).
  - Node 22 and Node 24 outputs are byte-identical (`diff -rq` empty).
  - `inventory` then exits 0.
- **R3: closed.**
  - The exclusion now runs on the repo-relative `/` path, and the Rust, frontend and error-site inventories store that form.
  - Write then `--check` gives exit 0; error-docs, inventory and resources tests pass 18/18 after regeneration in the scratch copy.
- **R4: closed.** `map_recipe_metadata` is now listed with the re-checked commands. This matches:
  - `service.rs:2850-2857` (short lease at admission) and `:2919-2929` (short lease and comparison after dispatch);
  - `find_edition_updates` (`edition_service.rs:65-82`);
  - `shared_library_browse` (`dispatch.rs`, lease at entry and again after the wait).
- **R5: closed.** §1.4 now lists all three `flush_browsing` callers: `service.rs:3648`, `mobile_service.rs:1915` and `update.rs:1081`. It agrees with §1.5.
- **R6: closed.** The header now says "runs tests on pull requests".
- **R7: closed, assuming GitHub's documented ordered-pattern semantics.** I simulated that algorithm against the new list (`**` matching `/` and dot names):

  | Changed files | Checks run? |
  |---|---|
  | `docs/ERRORS.md` alone | yes |
  | `docs/UPDATE.md`, `README.md`, a nested README, `LICENSE`, root `.gitignore` | no |
  | `docs/windows-agent-tools/*.ps1` | no (ignored before this change too) |
  | `.github/workflows/checks.yml`, `.gitattributes`, Rust, scripts, locale JSON | yes |
  | `src-tauri/gen/apple/.gitignore`, vendored dotfiles | yes |
  | docs plus code together | yes |

  - If `**` turned out not to match dot-prefixed paths, the explicit `.github/**` and `.gitattributes` lines cover what matters. Only the tracked dotfiles under `src-tauri/gen` and `src-tauri/vendor` would be missed, and those rarely change alone.
  - No test reads any other `docs/` or `.md` file. I checked `scripts/test-*` and `check-unsafe-census.mjs`.
- **R8: closed.**
  - `frontend-build` is the only job that uploads `frontend-dist`.
  - Five jobs download it into `dist` and each `needs: frontend-build`: `frontend`, android, windows and linux clippy, and `linux-tests`.
  - Every job keeps the draft-PR `if:`.
  - A failing JS or browser test no longer skips the Rust jobs.
  - `dist` has no hidden files, so `upload-artifact@v4` excluding them loses nothing.
  - The `frontend` job runs `npm ci` before the download, and `npm ci` leaves `dist` alone.

**Doubts carried forward**
- **Owner decision:** if branch protection requires checks by name, the new "Frontend type-check and build" check is not in that list. I cannot read the protection settings.
- **Unconfirmed:** GitHub's own evaluation of the `paths` filter. A one-file PR that touches only `docs/ERRORS.md` would confirm it.
- The shell-surfaces browser test flake from round 1 is still the shell lane's to fix. It now fails only the `frontend` job.

**What I ran**
- YAML parse of `checks.yml` and `build.yml`: ok.
- A per-job map of `needs`, uploads and downloads, generated from the parsed YAML.
- The `paths` simulation above, in `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/adv-cidocs/paths_sim.py`.
- In a fresh scratch copy:
  - `generate` on Node 22 and Node 24, the plural-order comparison, `inventory`, and error-docs write then `--check`;
  - deliberate breaks of `pluralCategories` (tested on both Nodes), of `portablePath`, and of the separator filter (simulated Windows).
- In the repository, read-only: `node --test scripts/test-error-docs.mjs scripts/test-localization-inventory.mjs scripts/test-localization-resources.mjs` → `# pass 16 # fail 2`. The 2 failures are the tests that pass only once `docs/ERRORS.md` and the localization manifest are regenerated. `docs/ERRORS.md` md5 is unchanged (a04f3033…).

**What I could not verify**
- Any Actions run: the artifact hand-off, path-filter evaluation, and runner Chrome or Node.
- The Windows host.
- `cargo test` of `src-tauri`, not run under this lane's rules.
