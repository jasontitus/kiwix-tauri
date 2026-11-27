# Fast iteration

The slow loops are release builds (`codegen-units = 1`) and per-device
sign/install pipelines, not ordinary development builds. Use the loops below
for feature work and reserve release builds for acceptance.

## One-time setup

Already configured on this workstation:

- `sccache` as the global rustc wrapper (`~/.cargo/config.toml`) with a 50 GiB
  disk cache. It caches dependency `rlib`s across worktrees and clean builds.
  Incremental builds bypass the cache automatically.
- `jobs = 10` globally; override with `CARGO_BUILD_JOBS` when needed.
- `bacon` for check-on-save and `cargo-nextest` for faster test runs.
- `[profile.dev] debug = "line-tables-only"` in `src-tauri/Cargo.toml`; test
  builds inherit it. This keeps backtraces but drops most debug-object size.

`scripts/dev.sh` puts the rustup toolchain first on `PATH` (the Homebrew `rustc`
has no iOS/Android targets) and sets default Java and Android SDK/NDK locations;
it resolves no desktop SDK. Run:

```sh
scripts/dev.sh doctor
```

## Daily loops

| Goal | Command | Typical |
| --- | --- | --- |
| Rust type/borrow check after an edit | `scripts/dev.sh check` | ~9 s |
| Check on every save | `scripts/dev.sh bacon` | ~9 s/edit |
| One Rust test module | `scripts/dev.sh test localswarm::protocol` | build once, then seconds |
| Faster test runner | `scripts/dev.sh nextest <substring>` | seconds |
| Full Rust suite | `scripts/dev.sh test` | ~90 s |
| Desktop app with frontend HMR | `scripts/dev.sh run` | debug build, then HMR |
| Debug app binary only | `scripts/dev.sh build` | ~45 s cold, ~2 s link-only |
| Frontend typecheck + bundle | `npm run build` | seconds |
| Frontend unit tests | `npm run test:tab-stack` (etc.) | seconds |

`scripts/dev.sh run` is `tauri dev`: Vite reloads the frontend instantly; Rust
edits trigger a debug rebuild. This is the fastest way to exercise library,
reader, search, downloads, and settings behavior. Use devices only for
platform-specific paths (nearby/AWDL, BLE, Android intents, signing).

## Device loops

- Android debug APK (fast, debug-signed):
  `scripts/dev.sh android-debug`, then
  `adb install -r src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk`.
- Android release APK (acceptance): `scripts/dev.sh android`.
- iOS simulator (fast, no signing):
  `TAURI_KIWIX_MOBILE_SDK_ROOT="$PWD/.native/mobile-retained-fd-v2/aarch64-apple-ios-sim" npm run tauri -- ios dev --target aarch64-sim`.
- iOS device release pipeline: `scripts/dev.sh ios -- --config ... --prepare-only ...`
  (the `scripts/iphone.py` flow; ~80-200 s depending on native cache).
- iOS device debug: `npm run tauri -- ios build --debug --target aarch64` and
  sign/install with the same profile; not part of the automated acceptance
  pipeline.

## Why release is slow

`src-tauri/Cargo.toml` sets `[profile.release] codegen-units = 1` so the
shipping binary gets whole-crate optimization. That forces a single rustc codegen
unit and a long serial build; it is intentional. Do not use `--release` for
iteration. `cargo check` stops before codegen and is the correct feedback loop.

The crate is monolithic, so its frontend (type checking/borrow checking) is
single-threaded. Splitting the crate would parallelize it but is a large
refactor; `cargo check` avoids the expensive parts instead.

## Reproducing an issue

1. Locate the owning module under `src-tauri/src/`.
2. Run its tests: `scripts/dev.sh test <module-or-test-name>`.
3. For frontend behavior, use the matching `scripts/test-*.mjs` node test and
   the desktop app.
4. Only after the fast loop passes, run the full suite and the device pipeline.

## Measure the whole testing iteration

Start a durable marker before preparation and finish it after the device window
and cleanup. Wrap the next **required** build; do not rebuild merely to obtain a
timing sample:

```sh
TIMING_DIR="$PWD/.native/timing-run-001"
ITERATION="run-001"
mkdir -m 700 "$TIMING_DIR"
python3 scripts/testing_timing.py begin --output "$TIMING_DIR" --iteration "$ITERATION"
python3 scripts/testing_timing.py command --output "$TIMING_DIR" --iteration "$ITERATION" --category build --label android-release -- scripts/dev.sh android
# Run the prepared acceptance command with the same iteration ID.
python3 scripts/testing_timing.py finish --output "$TIMING_DIR" --iteration "$ITERATION"
python3 scripts/testing_timing.py report "$TIMING_DIR"
```

The command wrapper preserves streams and exit status, propagates the iteration
ID, and bounds cancellation/reaping of its own child. It does not add execution
timeouts or retries. Timing-write failures are disclosed without replacing the
original command failure or preventing cleanup.

The iPhone, Mac/Android, Android-pair and Android Nearby entrypoints also write
`timing.jsonl` and `timing-summary.json` in their existing output directories.
For an unwrapped command, set `KIWIX_TIMING_ITERATION="$ITERATION"`; coordinators
propagate their selected ID to children. Pass those output directories as
additional `report` arguments to merge the same iteration's preparation,
device and cleanup ledgers. Merge inputs must share the host and boot.

Interpret the report carefully:

- Categories distinguish build, host preparation, ADB command wall,
  instrumentation/device automation, observed UI waits and cleanup.
- Category durations are **inclusive**. Nested or parallel spans overlap;
  use the interval union for measured coverage, not a sum of category totals.
- ADB wall includes transport **and device execution**. Long-running
  instrumentation is device-automation time, not pure ADB latency. Reader
  readiness ends UI waiting before subsequent hashing and evidence capture.
- Durable markers include gaps between commands. The residual includes
  orchestration, review, model generation, tool round trips and idle time;
  it is **not measured inference**. Exact inference duration is unavailable
  and remains `null`.
- Incomplete runs, missing/truncated records and logging errors are disclosed;
  they are not silently converted into complete timing measurements.

Prepare fresh frozen run receipts after timing-source changes. The timing module
is a verified transitive dependency, not part of the production-app build cache
key. `iphone-build.py` remains unchanged: its invocation is measured by the
caller or command wrapper, preserving the existing signed-app cache and its
identity checks.

