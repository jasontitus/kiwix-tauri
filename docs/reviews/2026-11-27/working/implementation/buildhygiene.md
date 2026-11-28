## buildhygiene lane: report

All three items are fixed. The opt-in type-check found no compile drift in `location_linux.rs` or `location_android.rs`: both are clean under clippy `-D warnings`, so neither file was edited. `Cargo.lock` is byte-identical to `d98cac2`.

### SERVE-6 — Dead libzim SDK glue, and docs that say the app links libzim
Disposition: fixed. I chose to delete `desktop_sdk.rs` rather than move it. It is a Cargo build-script module, nothing builds it, and the real comparison tooling is Python (`scripts/build-*-sdk.py`, `docs/windows-agent-tools/*.py`, `scripts/bench-archive-readers/summarize.py`). Putting it under `scripts/` would only relocate dead code.

Verification of the finding:
- `mobile_sdk.rs` was referenced nowhere.
- `desktop_sdk.rs` was used only by `#[cfg(test)] mod desktop_sdk` in `lib.rs`, and `build.rs` no longer references it.
- No other test used the `cc` dev-dependency; there is no `cc::` under `src`, `tests` or `examples`.
- The `.patch` files are still used by `scripts/build-{desktop,android,ios}-sdk.py` and `docs/windows-agent-tools/assemble-sdk.py`, so they stay.
- What enforces the Windows SDK manifest now: nothing.
  - No workflow mentions an SDK. The only hit is the runner label `kiwix-libzim-9_8` at `build.yml:296`.
  - `assemble-sdk.py` writes a `kiwix-sdk.json`.
  - `summarize.py` checks a benchmark library and its patch copies against such a manifest, but that is a comparison tool, not the app build.
  - The PACKAGING "Guarded Windows" section contained no manifest contract, although `WINDOWS-TEST-BRIEF.md` pointed to it as the place that did.

Change:
- Deleted `src-tauri/native/mobile_sdk.rs` and `src-tauri/native/desktop_sdk.rs`. Backup copies are in `/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/buildhygiene/`.
- `src-tauri/src/lib.rs`: removed the 6-line `#[cfg(test)] #[path = "../native/desktop_sdk.rs"] mod desktop_sdk` block and its comment.
- `src-tauri/Cargo.toml`: removed the `cc = "1"` dev-dependency and its comment. The `cc` build-dependency stays.

Doc claims changed (old → new):
- `docs/FORKS.md:30`
  - Old: "The app links a pinned libzim 9.8.2 built with five patches (`src-tauri/native/libzim-*.patch`). Every SDK applies split-ranges and retained-fd."
  - New: "The app does not link libzim: it reads ZIM content with Zimru and searches embedded indexes with zimfile-xapian (`docs/PACKAGING.md`, `docs/DEPENDENCIES.md`). The five patches (`src-tauri/native/libzim-*.patch`) apply only to the pinned libzim 9.8.2 SDKs that the comparison and previous-release recipes build (`scripts/build-*-sdk.py`, `docs/windows-agent-tools/`). Every such SDK applies split-ranges and retained-fd."
  - The rest of the paragraph is unchanged.
- `docs/WINDOWS-AGENT-HANDOFF.md:21`
  - Old: "`docs/PACKAGING.md` owns the SDK manifest contract. Do not invent hashes or relax `src-tauri/native/desktop_sdk.rs`."
  - New: "This manifest contract was checked at build time by `src-tauri/native/desktop_sdk.rs`, which was removed once the app stopped linking libzim. No current app build step or workflow reads or verifies a Windows libzim SDK or its `kiwix-sdk.json`; `docs/windows-agent-tools/assemble-sdk.py` writes one for the native comparison tools only. Do not invent hashes."
- `docs/WINDOWS-TEST-BRIEF.md:65`
  - Old: "The exact manifest contract is in `docs/PACKAGING.md`, section "Guarded Windows prerequisite and build". `src-tauri/native/desktop_sdk.rs` enforces it at build time. **There is no script for this in the repository.**"
  - New: "No current document or build step defines or enforces this manifest: the app no longer links libzim, and `src-tauri/native/desktop_sdk.rs`, which checked it at build time, has been removed. `docs/windows-agent-tools/build-sdk.ps1` and `assemble-sdk.py` build and describe such an SDK for the native comparison tools only (see `docs/WINDOWS-TEST-REPORT.md`)."
  - The old "no script" sentence was also false, because those two scripts exist.
- `docs/WINDOWS-TEST-BRIEF.md:70`
  - Old: "Never invent hashes, and never relax the checks in `desktop_sdk.rs`."
  - New: "Never invent hashes."
- `docs/PACKAGING.md`, Guarded Windows section: added after "The existing `kiwix-libzim-9_8` label selects the restricted Windows runner.":
  - "The name is historical: the app no longer links libzim, so the Windows build needs no libzim SDK, and no app build step or workflow reads or verifies a `kiwix-sdk.json` manifest."
- `docs/PACKAGING.md:3` already matched the tree and is unchanged.

Tests: new `scripts/test-build-hygiene.mjs`, picked up automatically by the CI glob `node --test scripts/test-*.mjs`.
- It checks that every `src-tauri/native/*.rs` is named as `native/<file>` in `build.rs` or `src/lib.rs`.
- It checks that no `.md` file outside `docs/reviews/`, `CHANGELOG*`, `node_modules`, `target`, `vendor`, `dist` and `.git` matches `/\bthe app links (?:a pinned )?libzim\b/i`.
- A temp-directory fixture self-test proves both checks detect a violation and ignore the historical exemptions.
- Result: `# tests 5  # pass 5  # fail 0`.
- Observed failing at the base: I extracted `d98cac2` with `git archive` into scratch and ran the exported checkers on it. They reported `unreferenced: [ 'mobile_sdk.rs' ]` and `stale claims: [ 'docs/FORKS.md' ]`.

Not exercised: nothing Windows-native. The 3 removed `desktop_sdk` unit tests were pure path helpers for the deleted module, so the host lib test count drops by 3 compared with the baseline 1329.

### BUILD-5 — Linux and Android location providers never compiled; zbus in every Linux build; DEPENDENCIES.md says the opposite
Disposition: fixed, with one deliberate addition. `build.rs` panics with a clear message when `KIWIX_EXPERIMENTAL_LOCATION=1` is set on Linux or Android without the feature:
- Without it, on Linux the result is confusing unresolved-zbus errors.
- The env var is kept as well as the feature, so that `--all-features` (used by `cargo deny` and the DEPENDENCIES.md counts) never turns on `native_location` or an unreleased provider.

Verification of the finding:
- At the base, `build.rs` emitted the cfg only for macos, ios and windows.
- zbus was a non-optional Linux dependency, used only by the cfg'd-out `location_linux.rs`.
- After the fix, `cargo tree --locked --target <t> -e normal,build -i zbus` (and `-i ordered-stream`) prints nothing on all nine targets: x86_64/aarch64 Linux, x86_64/aarch64 macOS, x86_64 Windows MSVC, aarch64/armv7 Android, iOS and iOS-sim.
- With `--features experimental-location` it shows `zbus v5.19.0 └── kiwix-workbench`.
- Linux unique `name version` count with `-e normal,build`: 537 by default, 565 with the feature.

Change:
- `src-tauri/build.rs`: adds `cargo:rerun-if-env-changed=KIWIX_EXPERIMENTAL_LOCATION`. It emits `native_location` for `linux | android` only when that env var is `1` and `CARGO_FEATURE_EXPERIMENTAL_LOCATION` is set; otherwise it panics with "KIWIX_EXPERIMENTAL_LOCATION=1 also needs --features experimental-location". macOS, iOS and Windows are unchanged.
- `src-tauri/Cargo.toml`: zbus is now `optional = true`, with an updated comment. New feature `experimental-location = ["dep:zbus"]`, commented "Not for release builds".
- `.github/workflows/checks.yml`:
  - `linux-typecheck` gets a second step, "Clippy the unreleased GeoClue location provider (opt-in)": `--lib --tests --features custom-protocol,experimental-location` with `KIWIX_TYPECHECK_ONLY=1` and `KIWIX_EXPERIMENTAL_LOCATION=1`.
  - `android-typecheck` gets a second step, aarch64 matrix entry only, with the same env and `--features ${{ matrix.features }},experimental-location`.
  - The default-feature steps are unchanged, so the shipping cfg is still checked.
- `.github/workflows/build.yml`: release gate kept, wording corrected. It still guards file associations, `kiwix://`, the startup class and desktop integration, not only GeoClue.
  - Step name: "Check packaged libraries and GeoClue desktop identity" → "Check packaged libraries and desktop identity".
  - Message: "…basename must match the GeoClue runtime identifier." → "…basename must match the application identifier org.kiwix.workbench."

Doc claims changed (old → new):
- `docs/DEPENDENCIES.md`, Done entry (finding cited :96)
  - Old: "`cargo tree --target <triple>` shows no zbus on any of the nine shipping targets (…)."
  - New: "The app's own unreleased GeoClue provider (`location_linux.rs`) also uses zbus, so zbus is an optional Linux dependency behind the `experimental-location` feature, which only the CI type-check enables (with `KIWIX_EXPERIMENTAL_LOCATION=1`, see `build.rs`). With default features, `cargo tree --target <triple>` shows no zbus on any of the nine shipping targets (…)."
- `docs/DEPENDENCIES.md`, removed list (finding cited :127, now about :134)
  - Old: "`ordered-stream` (zbus) is no longer compiled for any shipping target, now that `tauri-plugin-single-instance` is Windows-only, but it remains in Cargo.lock."
  - New: "`ordered-stream` (zbus) is not compiled for any default shipping build, now that `tauri-plugin-single-instance` is Windows-only and the app's own zbus dependency needs the opt-in `experimental-location` feature, but it remains in Cargo.lock."
- `docs/DEPENDENCIES.md` (about :58)
  - Old: "…notices (RUSTSEC-2026-0370/0371/0372) appear only in a run without `--target`; no shipping target compiles them…"
  - New: "…appear in a run without `--target`, and in a Linux run with `--all-features` through the opt-in `experimental-location` feature; no default shipping build compiles them…"
- `docs/PACKAGING.md`, Linux section
  - Old: "The Linux runtime/GeoClue application identifier is `org.kiwix.workbench`."
  - New: "The Linux runtime application identifier is `org.kiwix.workbench`; the unreleased GeoClue location provider (`src-tauri/src/location_linux.rs`) presents the same identifier."
  - Old: "…GeoClue availability, desktop integration, and the user's location permission still need runtime validation."
  - New: "…desktop integration still needs runtime validation. Release builds compile no Linux location provider (`src-tauri/build.rs` enables it only for an opt-in type-check), so there is no GeoClue location fix to validate yet."
- `docs/PACKAGING.md`, release acceptance item 3
  - Old: "…then exercise location permission with GeoClue available; report unsupported/service-denied states honestly instead of treating a filename check as a successful location fix."
  - New: "Confirm desktop integration retains `org.kiwix.workbench.desktop`. Linux release builds compile no location provider, so GeoClue location permission cannot be exercised until that provider ships; do not treat a filename check as a successful location fix."
- `docs/FEATURES.md:417`
  - Old: "…providers are written and reviewed but not yet compiled, so they stay off in build.rs."
  - New: "…providers are written and reviewed but not shipped: build.rs leaves them out of every release build, and CI only type-checks them through the opt-in `KIWIX_EXPERIMENTAL_LOCATION=1` and `experimental-location` feature."

Tests:
- `test-build-hygiene.mjs` also asserts:
  - zbus is declared once with `optional = true`;
  - the `experimental-location = ["dep:zbus"]` feature exists;
  - `build.rs` has the `rerun-if-env-changed` line and the feature check;
  - both CI typecheck jobs set `KIWIX_EXPERIMENTAL_LOCATION: '1'` and `--features …experimental-location`.
- The base would fail those assertions (zbus line without `optional`; 0 occurrences of `experimental-location` in the base `Cargo.toml`, `checks.yml` and `build.rs`). That is inferred from the base file contents; I did not run them against the base.
- Runtime check of the opt-in: `KIWIX_EXPERIMENTAL_LOCATION=1 cargo test --lib --features experimental-location -- location_linux permissions` gave `test result: ok. 15 passed; 0 failed`. That includes the 5 `location_linux::tests`, linked against zbus.
- The env-without-feature panic was observed: "panicked at build.rs:24:13: KIWIX_EXPERIMENTAL_LOCATION=1 also needs --features experimental-location".

Not exercised:
- `location_android` tests: Android is type-check only here.
- The new CI steps were not run on GitHub; the YAML parses.
- No GeoClue runtime.

### mobile_policy.rs:566 — stale `localswarm_hotspot_start`
Disposition: fixed.

Verification: the test asserts `!local_network_command(name)`. Any unknown name returns `false` through the `_ => false` arm, so that entry was vacuous. The command is retired; `service_tests.rs::retired_internal_commands_are_no_longer_dispatched` already covers it as "Unknown application command". The hotspot now starts through `localswarm_share_all {hotspot:true}`, which this test already asserts is admitted.

Change: removed the name. The comment changed from "Bluetooth and hotspot commands request their own NEARBY_DEVICES grant." to "Bluetooth commands request their own NEARBY_DEVICES grant."

Tests: `local_network_commands_are_admitted_before_lan_access ... ok`.

### Commands run
- `cargo metadata --locked` → METADATA_OK. `git diff --quiet d98cac2 -- src-tauri/Cargo.lock` → unchanged.
- Host clippy, default features (`--lib --tests --features custom-protocol -- -D warnings`) → Finished, exit 0.
- Host clippy with `KIWIX_EXPERIMENTAL_LOCATION=1 KIWIX_TYPECHECK_ONLY=1 … --features custom-protocol,experimental-location` → Finished, clean. The build-script output contained `cargo:rustc-cfg=native_location`.
- Windows-gnu clippy → Finished, exit 0.
- Android, default (`scratchpad/android-clippy.sh`) → Finished, clean.
- Android opt-in (`/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/buildhygiene/android-clippy-location.sh`, which adds the env var and `,experimental-location`) → Finished, exit 0. The build output contained `rustc-cfg=native_location`.
- `cargo test --lib permissions` → 10 passed; 0 failed.
- `cargo test --lib mobile_policy` → 17 passed; 0 failed.
- `node --test scripts/test-build-hygiene.mjs` → 5 pass, 0 fail.
- `node --test scripts/test-*.mjs` → 540 tests, 524 pass, 2 fail, 14 skipped. The 2 failures are the `docs/ERRORS.md` check and the localization manifest check. They are drift from other lanes' changes; the error inventory scans only `src-tauri/src`, and my Rust edits there add no strings. They are for the integration owner's regeneration.
- `node scripts/check-unsafe-census.mjs` → "75 matches / 75 blocks / 2 impls in 20 files; every item documented."
  - Both location modules are `#![forbid(unsafe_code)]` and contain no `unsafe`.
  - The census scans `src-tauri/src` text regardless of cfg, so cfg'd-out code was already counted. It never scanned `src-tauri/native`.

### Files changed (mine)
Deleted:
- `src-tauri/native/mobile_sdk.rs`
- `src-tauri/native/desktop_sdk.rs`

Edited:
- `src-tauri/src/lib.rs` (mod removal only)
- `src-tauri/Cargo.toml`
- `src-tauri/build.rs`
- `src-tauri/src/mobile_policy.rs` (test only)
- `.github/workflows/checks.yml` (two new steps)
- `.github/workflows/build.yml` (step name and one message only)
- `docs/FORKS.md`
- `docs/PACKAGING.md`
- `docs/DEPENDENCIES.md`
- `docs/WINDOWS-AGENT-HANDOFF.md`
- `docs/WINDOWS-TEST-BRIEF.md`
- `docs/FEATURES.md`

New:
- `scripts/test-build-hygiene.mjs`

`git diff d98cac2` on `lib.rs`, `checks.yml` and `build.yml` also shows other lanes' checkpointed work, including a new Windows test step in `build.yml`. That is not mine.

There are no new user-visible frontend strings. The only new Rust string is a build-script panic message.

### Handoffs / contract changes
- New cargo feature `experimental-location` and env var `KIWIX_EXPERIMENTAL_LOCATION`. Both are needed together, on Linux and Android only, and must never be set in release builds.
- The host lib test count drops by 3 (the removed `desktop_sdk` tests).

### Doubts
- The env var plus feature check does not stop a developer from building a linkable Linux or Android binary with the unreleased provider, which also turns off the webview-geolocation path in `permissions.rs`. I left that possible for local runtime testing and did not require `KIWIX_TYPECHECK_ONLY=1`.
- The Android opt-in CI step covers aarch64 only; armv7 runs only the default step.
- Stale claims I found but left alone because they are outside this lane:
  - `docs/FORKS.md`, "Replacing libzim with zimru" (about lines 70–109), still reads as a future plan.
  - `docs/DEV-ITERATION.md:19`, "dev.sh resolves the patched desktop SDK" (BUILD-6).
  - The `scripts/linux-appimage.py:108` message still says "GeoClue identity".
  - DEPENDENCIES.md `--all-features` package counts include zbus on Linux, and the "31 unique" figure for single-instance was already doubtful at the base, because zbus was a direct dependency.