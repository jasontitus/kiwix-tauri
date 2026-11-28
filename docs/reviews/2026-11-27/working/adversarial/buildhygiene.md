## Verdict for lane buildhygiene: only non-blocking findings

No release path can turn on `native_location` for Linux or Android. Shipping cfg emission is unchanged on all five platforms, `Cargo.lock` is unchanged, and zbus is absent from the default dependency tree on all nine targets (I checked all nine myself). Both opt-in type-checks pass. What remains is stale documentation that this lane should finish, plus some weak spots in the new test.

Reviewed: `git diff d98cac2` (HEAD `ca5c427`) for these paths:
- `src-tauri/{build.rs,Cargo.toml,Cargo.lock,native/,src/lib.rs,src/mobile_policy.rs}`
- `.github/workflows/{checks,build}.yml`
- `docs/{FORKS,PACKAGING,DEPENDENCIES,FEATURES,WINDOWS-AGENT-HANDOFF,WINDOWS-TEST-BRIEF,DEV-ITERATION}.md`
- `scripts/test-build-hygiene.mjs` (an untracked addition at the time; it is committed in the checkpoint now)

I also read the consumers:
- `permissions.rs` (every `native_location` cfg)
- `service.rs::native_location_supported`
- `mobile.rs:903`
- the `lib.rs` location mods
- `location_linux.rs` and `location_android.rs`
- `deny.toml` and `dependencies.yml`
- `scripts/linux-appimage.py`
- `summarize.py` and `assemble-sdk.py`

Hunks from other lanes in these files were ignored: the Windows test step in `build.yml`, the checks.yml path filter and frontend-build job, and the BUILD-11 and torrent-fallback text.

### BUILDHYGIENE-R1 — FORKS.md still describes the zimru migration as a future plan, contradicting the new line 30 in the same file
Severity: Should-fix
Fix under review: SERVE-6
Evidence class: source-traced
Location: `docs/FORKS.md:70-109` ("## Replacing libzim with zimru")
Trigger: A reader of the corrected FORKS.md:30 continues down the page.
Causal path:
- Line 30 now says "The app does not link libzim".
- Forty lines later the file says "`src-tauri/native/archive.cpp` uses a small slice of libzim". That file does not exist (`ls src-tauri/native` shows no `archive.cpp`).
- It also says "Nothing in this repository's history mentions zimru". `Cargo.toml:71-72` pins `zimru` and `zimfile-xapian`.
- Its "Recommended path" ends with the steps "Then move search, then remove libzim", which have already been done.
- SERVE-6 asked for FORKS.md to state that the shipping reader is zimru plus zimfile-xapian. The file now says both things.

Smallest correct fix:
- Retitle the section to `## Replacing libzim with zimru (done; pre-migration analysis)`.
- Insert as its first sentence: "This migration is complete: the app reads archives with `zimru` and searches with `zimfile-xapian` (pinned in `src-tauri/Cargo.toml`), and the C++ bridge `src-tauri/native/archive.cpp` has been removed. The rest of this section is the analysis written before the migration and is not a current gap list."
- Delete the sentence "Nothing in this repository's history mentions zimru…".
- Do not restate the gap table as current, since I did not verify what zimru supports today.

### BUILDHYGIENE-R2 — DEV-ITERATION.md still tells developers to set an env var that only the deleted mobile_sdk.rs read
Severity: Nit
Fix under review: SERVE-6
Evidence class: source-traced
Location: `docs/DEV-ITERATION.md:53` (Device loops → iOS simulator)
Trigger: A developer follows the iOS simulator recipe.
Causal path:
- The recipe is `TAURI_KIWIX_MOBILE_SDK_ROOT="$PWD/.native/mobile-retained-fd-v2/aarch64-apple-ios-sim" npm run tauri -- ios dev …`.
- The only reader of `TAURI_KIWIX_MOBILE_SDK_ROOT` was `MobileSdk::load` in the deleted `native/mobile_sdk.rs`, which was already unreferenced at base.
- No other consumer exists: I grepped `scripts`, `src-tauri/gen`, the plugins and the workflows.
- The doc implies a patched SDK is needed when it is ignored.
- PACKAGING.md:97 and :267 say the same, but they sit inside the "Historical native SDK builds" `<details>` block, so they can stay.

Smallest correct fix: change the line to `npm run tauri -- ios dev --target aarch64-sim`.

### BUILDHYGIENE-R3 — The AppImage audit message still names a "GeoClue identity"
Severity: Nit
Fix under review: BUILD-5
Evidence class: source-traced
Location: `scripts/linux-appimage.py:108`
Trigger: A packaged desktop entry fails the check.
Causal path:
- The check is `d.get("StartupWMClass") == ID …`, but the message reads "…name, GeoClue identity or file associations".
- No release build has a GeoClue provider any more.
- The `build.yml` step and message were corrected; this sibling message was not.
- No test matches the string. I grepped `scripts/test-linux-appimage.py` and every other `scripts/` file.

Smallest correct fix: change the message to `"Desktop entry lost its executable, icon, name, startup class or file associations"`.

### BUILDHYGIENE-R4 — The documentation half of test-build-hygiene.mjs does not run on the PRs that could break it
Severity: Nit
Fix under review: SERVE-6 regression test
Evidence class: source-traced
Location: `scripts/test-build-hygiene.mjs` (`staleLibzimClaims`), together with the `.github/workflows/checks.yml` `paths:` filter (`'!docs/**'`, `'!**/*.md'`)
Trigger:
1. A docs-only PR reintroduces "The app links a pinned libzim".
2. checks.yml does not run, so the PR merges green.
3. The next unrelated code PR fails this test.

Causal path: the filter excludes all Markdown except `docs/ERRORS.md`, so the check runs only alongside non-doc changes.

Smallest correct fix: pick one.
- Add `'scripts/test-build-hygiene.mjs'` and `'docs/FORKS.md'` as re-included paths, as is done for `docs/ERRORS.md`.
- Or accept the gap and say in the test's header that it fires on the next code PR.

### BUILDHYGIENE-R5 — The CI assertion would still pass if the shipping-cfg clippy step were folded into the opt-in step
Severity: Nit
Fix under review: BUILD-5 regression test
Evidence class: source-traced
Location: `scripts/test-build-hygiene.mjs`, test "CI type-checks the unreleased Linux and Android location providers"
Trigger: A later edit adds `,experimental-location` to the default "Clippy with warnings as errors" step and drops the separate opt-in step. The default `native_location`-off cfg, the one that ships, would then be type-checked nowhere.
Causal path: the test only asserts that `KIWIX_EXPERIMENTAL_LOCATION: '1'` and `--features …experimental-location` occur somewhere in each job body.

Smallest correct fix: in each job body, assert at least two `cargo clippy` invocations, and that at least one `--features` line lacks `experimental-location`.

### BUILDHYGIENE-R6 — DEPENDENCIES.md package-count method and two sentences are imprecise after the feature split
Severity: Nit
Fix under review: BUILD-5
Evidence class: reproduced
Location:
- `docs/DEPENDENCIES.md:51` (count command) and :41 (Linux row)
- `docs/DEPENDENCIES.md:96` ("which only the CI type-check enables")
- `docs/PACKAGING.md` Linux paragraph ("enables it only for an opt-in type-check")

Causal path:
- The documented count command uses `--all-features`, which now pulls in the opt-in zbus. I measured Linux x86_64 at 537 packages by default and 565 with `--all-features`.
- The method therefore no longer measures the shipping Linux build. The table's "~493" was already stale at base.
- `dependencies.yml` cargo-deny runs `--all-features`, so it "enables" the feature too, though only for the graph; nothing is compiled.
- `build.rs` emits the cfg for any build with env+feature, including linked test builds (the implementer ran `cargo test` that way), not only for a type-check.

Smallest correct fix:
- At :51, use `--features custom-protocol` (the shipping set) and refresh the Linux row, or append "on Linux this includes 28 packages from the opt-in `experimental-location` feature".
- At :96, say "which only the CI type-check enables for compilation (cargo-deny's `--all-features` graph also includes it)".
- In PACKAGING, say "only when a developer opts in with `KIWIX_EXPERIMENTAL_LOCATION=1` and the `experimental-location` feature, as the CI type-check does".

### BUILDHYGIENE-R7 — WINDOWS-TEST-BRIEF Phase 2 now contradicts itself
Severity: Nit
Fix under review: SERVE-6
Evidence class: source-traced
Location: `docs/WINDOWS-TEST-BRIEF.md:58-71`
Causal path:
- The new sentence ("…the app no longer links libzim…") is true. I checked:
  - no workflow mentions an SDK except the runner label at `build.yml:296`;
  - `build.rs` contains no SDK reference;
  - `build-sdk.ps1`, `assemble-sdk.py` and `WINDOWS-TEST-REPORT.md` exist.
- But it sits between "The build needs an MSVC build of libzim 9.8.2" and "If the machine already has such an SDK: set `LIBZIM_SDK_ROOT` to it and continue". Phase 3 is still titled "(needs the SDK)".
- Only the banner at the top of the file tells an operator to skip all of this.
- WINDOWS-AGENT-HANDOFF:23 has the same problem ("If the machine lacks the SDK, stop after the compile-only gate").

Smallest correct fix: open the rewritten paragraph with "Current builds skip Phases 2's SDK steps; see [WINDOWS-BUILD-PROVISIONING.md](WINDOWS-BUILD-PROVISIONING.md#build-and-test)." Make the matching one-clause change in HANDOFF:23.

### BUILDHYGIENE-R8 — Half-set opt-ins are ignored silently
Severity: Nit
Fix under review: BUILD-5
Evidence class: reproduced
Location: `src-tauri/build.rs:16-31`
Trigger, either of:
- `--features experimental-location` without the env var;
- `KIWIX_EXPERIMENTAL_LOCATION=true` (any value other than `1`).

Causal path:
- Both fall through to `_ => {}`. zbus is compiled, but no provider is checked and nothing is printed.
- Reproduced: `KIWIX_EXPERIMENTAL_LOCATION=true … clippy --features custom-protocol` finished clean with no cfg emitted.
- The reverse case (env var without the feature) panics, as intended. I reproduced that panic at `build.rs:24:13`.

Smallest correct fix: on `linux | android`, emit `cargo:warning=` when exactly one of the two opt-ins is present. Cargo-deny runs no build scripts, so this cannot disturb `--all-features` in CI.

### Per-fix assessment
- **SERVE-6:** fixes it, but R1 (FORKS.md later section), R2 (DEV-ITERATION:53) and R7 leave stale references nearby.
  - Both deletions are clean. No code, script or workflow references either file; the remaining mentions are in `docs/reviews`, the benchmark JSON hashes, and the two Windows docs that describe the file as removed.
  - The `cc` dev-dependency is gone, the build-dependency stays, and no `cc::` appears in `src`, `tests` or `examples`.
  - "Nothing enforces the Windows SDK manifest" is true for the app build and workflows. `summarize.py` does verify a manifest, but only for the benchmark tool, and the docs say so.
  - FORKS.md:30 is true: the recipes in `scripts/build-*-sdk.py` and `assemble-sdk.py` apply exactly those patches.
- **BUILD-5:** fixes it.
  - Shipping builds emit exactly the cfg they did at base: macOS, iOS and Windows hit the same arm; Linux and Android without the env var hit `_ => {}`.
  - `rerun-if-env-changed` is emitted on every target before the `KIWIX_TYPECHECK_ONLY` early return. That is correct. A feature change re-runs the script through the unit fingerprint.
  - Cross-compiles key on `CARGO_CFG_TARGET_OS`, so a Linux host building for Windows never panics.
  - `cargo doc` or a developer shell can hit the panic only if the env var is set without the feature.
  - No release path (`build.yml`, `tauri build`, `scripts/*`) sets the env var or the feature. `--all-features` in cargo-deny turns on zbus but never `native_location`, and that zbus graph is the same as at base, when zbus was non-optional.
  - The `permissions.rs` webview-geolocation fallback (`#[cfg(not(native_location))]` at :422 and :471) is unchanged for shipping Linux and Android.
  - The CI YAML parses. The `if: matrix.target == 'aarch64-linux-android'` condition implicitly includes `success()`. The default steps are unchanged, `--locked` is kept, and the opt-in Android step relies on the preceding step's NDK guard, which is fine.
  - The `build.yml` rename and message are accurate: the identifier in `tauri.conf.json:5` and `location_linux.rs:40` is `org.kiwix.workbench`.
  - The FEATURES.md, DEPENDENCIES.md (:58, :96, :134) and PACKAGING Linux and acceptance-item-3 sentences are true, apart from the precision points in R6.
- **mobile_policy.rs:566:** fixes it.
  - The entry was vacuous: `_ => false` returned false for it anyway.
  - `localswarm_hotspot_start` remains covered as retired in `service_tests.rs:6674`.
  - The new comment ("Bluetooth commands…") still heads a list that is mostly not Bluetooth, such as status, stops, `start_download` and the private_sync commands. That is cosmetic.

### Implementer's doubts
- **A linkable Linux or Android build with the unreleased provider is possible:** acceptable. It is a double opt-in that no release path sets, and it is useful for running `location_linux` tests. Optionally add a `cargo:warning` when the cfg is emitted without `KIWIX_TYPECHECK_ONLY`.
- **The Android opt-in covers aarch64 only:** acceptable. `location_android.rs` is plain Rust with no pointer-width code and no jni or `android-evaluation` dependency. Note that the aarch64 entry carries `android-evaluation`, whereas armv7 carries the release feature set; either choice is defensible.
- **Stale claims left alone:**
  - FORKS "Replacing libzim with zimru": fix now (R1).
  - `DEV-ITERATION.md:19` is already corrected by the BUILD-6 lane, and the new text matches `scripts/dev.sh:7-12`. A separate stale line, :53, should be fixed (R2).
  - `linux-appimage.py:108`: fix now (R3).
  - DEPENDENCIES `--all-features` counts: fix in this lane (R6). The historical "31 packages" figure could not be verified, because the git history is squashed.

### What I ran
- `node --test scripts/test-build-hygiene.mjs` → 5 pass, 0 fail.
- Base-failure check: I extracted `d98cac2` with `git archive` into `adv-buildhygiene/base` and ran the exported checkers on it.
  - Result: `unreferenced [ 'mobile_sdk.rs' ]`, `stale [ 'docs/FORKS.md' ]`.
  - The base `zbus` line lacks `optional`, and the base has no `experimental-location` feature and no CI env var, so the zbus and CI tests also fail on the base.
- `cargo tree --locked --target <t> -e normal,build -i zbus` and `-i ordered-stream`, on all nine targets (x86_64/aarch64 Linux, aarch64/armv7 Android, x86_64 Windows MSVC, aarch64/x86_64 macOS, iOS, iOS-sim) → "nothing to print" for every one.
  - `--features experimental-location` on Linux → `zbus v5.19.0 └── kiwix-workbench`; on Android → nothing.
  - `--all-features` on Linux → zbus.
  - Unique `name version` count for Linux x86_64: 537 by default, 565 with `--all-features`.
- `git diff --quiet d98cac2 -- src-tauri/Cargo.lock` → unchanged.
- Host clippy, default (`KIWIX_TYPECHECK_ONLY=1 … --lib --tests --features custom-protocol -- -D warnings`) → Finished.
- Host clippy, opt-in (`KIWIX_EXPERIMENTAL_LOCATION=1 KIWIX_TYPECHECK_ONLY=1 … --features custom-protocol,experimental-location -vv`) → Finished, and rustc received `--cfg native_location`.
- Env var without the feature → `panicked at build.rs:24:13: KIWIX_EXPERIMENTAL_LOCATION=1 also needs --features experimental-location`.
- `KIWIX_EXPERIMENTAL_LOCATION=true` → Finished, with no cfg (silent; see R8).
- Android aarch64 opt-in clippy, using my copy `adv-buildhygiene/android-optin.sh` with `custom-protocol,android-evaluation,experimental-location` → Finished.
- `cargo test --locked --lib --features custom-protocol -- permissions mobile_policy` → 27 passed, 0 failed.
- `KIWIX_EXPERIMENTAL_LOCATION=1 cargo test --locked --lib --features custom-protocol,experimental-location -- location_linux permissions` → 15 passed, including the 5 `location_linux` tests.
- Workflow YAML parsed with PyYAML; the step names, env and `if` values are as intended.

### What I could not verify
- The new CI steps have not run on GitHub, and I measured no CI timing; the 30-minute budget looks sufficient.
- I did not run armv7 Android opt-in clippy or Windows-gnu clippy myself.
- No GeoClue or Android LocationManager runtime is available here.
- No Windows operator walkthrough of the rewritten Windows docs.
