# Sanitizers, fuzzing and platform test handoff

This records the local sanitizer/fuzz pass on 2026-11-20. It is evidence for the
tested paths only; it is not a device, Windows, Linux or binary-level
certification.

## Local results

| Check | Result | Scope |
|---|---|---|
| LocalSwarm Objective-C++ ASan + UBSan | Pass, 12 checks, no sanitizer report | `native/localswarm_tests.mm` + `native/localswarm.mm` |
| ZIM archive parser fuzz | 100,000 seeded runs, no crash; 1,549 coverage edges | `zimru::Archive::from_positioned` and entry/check paths, as the target stood on 2026-11-20 (see "Fuzz targets changed since this pass") |
| MCP protocol fuzz | 100,000 seeded runs, no crash | `kiwix-mcp/src/protocol.rs` JSON-RPC handling |
| Rust `-Zub-checks` | 1,286 passed; 1 environment-only failure | Full lib suite; see below |
| App-test ASan | Not completed | x86_64 Apple link fails in `tauri-utils` initializer; fuzz targets provide the ASan coverage |

Commands:

```sh
./scripts/test-native-sanitizers.sh
RUNS=100000 MAX_TOTAL_TIME=60 ./scripts/fuzz.sh zim_archive
RUNS=100000 MAX_TOTAL_TIME=60 ./scripts/fuzz.sh mcp_protocol
```

`scripts/test-native-sanitizers.sh` builds the LocalSwarm bridge with
`-fsanitize=address,undefined` and runs its native assertions. Leak detection is
disabled because Apple's ASan runtime does not support it.

`scripts/fuzz.sh` requires nightly Rust and `cargo install cargo-fuzz`. It copies
the tracked ZIM fixtures and the MCP seed line into the local corpus, then runs a
bounded libFuzzer session. Corpora and crash artifacts are ignored.

### Fuzz targets changed since this pass

The results above were recorded on 2026-11-20. On 2026-11-27 the targets were
changed and **have not been run since**; both changes were type-checked only
(`cargo check --manifest-path src-tauri/fuzz/Cargo.toml` on stable Rust, Linux).

- `zim_archive` then stopped at directory entries and the MD5 pass, so it never
  decoded a cluster. It now resolves the first 64 entries to their blobs (zstd/xz
  cluster decoding and blob offset tables), looks up a title prefix range and
  validates the first four clusters. It uses the limits and cluster-cache budget of
  `open_source` in `src-tauri/src/archive.rs` instead of its own smaller ones. It
  does not apply the app's `validate_layout` table-extent check, so it also reaches
  inputs the app refuses.
- `search_index` is new. Its whole input is one embedded Glass index, opened with
  the app's `zimfile_xapian::ReaderLimits`, followed by one full-text page and one
  title-suggestion page with fixed queries. `scripts/fuzz.sh` seeds it with the two
  indexes of `tests/fixtures/indexed_search.zim`, copied out by
  `src-tauri/fuzz/extract-index-seeds.py` (needs `python3`).

```sh
RUNS=100000 MAX_TOTAL_TIME=60 ./scripts/fuzz.sh search_index
```

The 1,549-edge figure describes the earlier, shallower `zim_archive` target. Replace
it and add a `search_index` row only from an actual run.

The `-Zub-checks` suite's only failure is
`update::tests::signature_verification_rejects_a_tampered_payload`, because the
test binary ran from a `/tmp` symlink and the updater rejects that path. The
targeted storage test and the other 1,286 tests pass with UB checks enabled.

## iOS plugin XCTest suite: local only

`src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Tests` holds eight XCTest files
(the `KiwixMobileTests` target of `ios/Package.swift`). **No workflow builds or
runs them.** `checks.yml` and `build.yml` contain no iOS `xcodebuild` or
`swift test` step; the only native test CI runs is
`scripts/test-native-sanitizers.sh`, which builds `native/localswarm_tests.mm`
alone. A change to the Swift plugin is therefore not covered by a green
"Checks" run.

They also do not resolve from a clean checkout. `Package.swift` depends on the
local package `../.tauri/tauri-api`. That directory is git-ignored: the plugin's
build script (`tauri-plugin`'s mobile setup) copies Tauri's Swift API there only
when the crate is built for an iOS target on a Mac.

To run them locally, on a Mac with Xcode and an iOS simulator runtime:

1. Build the app for iOS once so that
   `src-tauri/plugins/tauri-plugin-kiwix-mobile/.tauri/tauri-api` exists
   (`npm run iphone -- --prepare-only` builds without contacting a device).
2. Open `src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Package.swift` in Xcode,
   choose an iOS simulator destination and run the tests; or, in that `ios`
   directory, list the generated schemes with `xcodebuild -list` and run
   `xcodebuild test -scheme <scheme> -destination 'platform=iOS Simulator,name=<installed simulator>'`.

These steps were written on a Linux host from `Package.swift` and the
`tauri-plugin` build script and were **not executed** when this section was
added; the repository records no earlier command line for this suite. `swift test`
on the Mac host is not an alternative: the package declares only the iOS platform
and its sources import UIKit.

CI does not run them because no such job exists yet. It would need a macOS
runner, an iOS-target Rust build to materialize `tauri-api`, and a simulator
destination, and it could not be written and observed to pass from the Linux
host used when this section was added. Add the job only after it has run green on
a real runner; until then record local runs, with the Xcode and simulator
versions, in the review notes for the change.

## Dependency audit

`cargo audit --no-fetch` now reports **no vulnerabilities** in the app, relay or
shared-crate lockfiles. It still reports six allowed warnings: `chrono`,
`proc-macro-error`, `zbus_names`, `zvariant`, `zvariant_derive` and the GTK
`glib` `VariantStrIter` unsoundness. The app and its UI stack do not iterate
`GVariant` string arrays, so that warning is outside the used surface; the others
are unmaintained-package notices.

The pass found `ringbuf 0.4.8` (double free/use-after-free when an element's
`Drop` panics) in the app lockfile through `librqbit-utp 0.7.0`. uTP stores `u8`
elements, whose `Drop` cannot panic, so this was a lockfile finding rather than a
demonstrated reachable panic path. It is still upgraded: `librqbit-utp` is now
vendored with `ringbuf 0.5.3`, and the host suite and Windows type checks pass. Run:

```sh
./scripts/check-dependency-audit.sh
```

## Limits

- The fuzz harnesses exercise Rust parsers. They do not fuzz the Windows named
  pipe, GTK print dialog, AppKit/DNS-SD callbacks, JNI bridge or the native PDF
  bridges.
- The native LocalSwarm harness is the only native code instrumented here.
  WebKit, Network.framework, sherpa/ONNX and ring are accepted platform
  boundaries and are not instrumented.
- No Windows, Linux or physical-device run was performed in this pass.
- `cargo-fuzz`'s corpus is local; only the seed fixtures and seed JSON line are
  tracked. The search-index seeds are derived from a tracked fixture at run time.
- No workflow runs a fuzz target; every fuzz result in this file is a local run.

## Platform test handoff

Use the exact commit under test. These are the paths that type checks and the
local sanitizers cannot prove.

### Windows

1. Build with MSVC and the current Windows prerequisites; record `rustc`, `cl`,
   WebView2 and Windows versions.
2. Agent connector/MCP relay: accept a same-user client; reject a different-user
   client; start the relay before the app, close the app mid-request, and retry
   after a stale pipe name.
3. Private-sync identity: create a new profile, confirm owner/DACL, delete and
   recreate it, and exercise a token whose default owner is a group.
4. PDF print: open the dialog, cancel, close the originating window while the
   dialog is open, and let the dialog time out. Confirm no stale HWND command
   reaches another window.
5. File publication: replace and no-replace paths, a FAT/exFAT volume, a
   remote mounted folder and a file symlink. Check capacity and filesystem type
   through the destination handle.
6. Run the Windows test targets; record any MSVC-only warning or failure.

### Linux

1. PDF export with a file print backend present, absent, and with two backends
   sharing the file printer's display name. Confirm the ambiguous case fails
   closed rather than printing on paper.
2. Cancel export while printer discovery is pending; confirm the job returns
   within the 2 s deadline and the app remains responsive.
3. Open the system print dialog, cancel it, and close the article window during
   the dialog.
4. Run the packaged AppImage/Debian package and the reader benchmark.

### iPhone / iOS

1. LocalSwarm discovery: browse, advertise, resolve and stop/restart; exercise
   AWDL/peer-to-peer and a disappearing interface.
2. QUIC connect/listen/read/write; cancel during connect, read and write; allow
   late callbacks and listener disposal.
3. PDF export handoff: present, cancel, background/relaunch, and revoke the
   originating window.
4. Document providers: open in place and split documents, revoke/regrant,
   relaunch, backup exclusion and failure partway through a set.
5. Run the physical-device window with a bounded sampler check first.

### Android

1. Cold start and Activity recreation; confirm the platform TLS verifier is
   initialized once and HTTPS tracker announces succeed.
2. Share/PDF export and a pending callback during shutdown.
3. Run the ARM64 and ARMv7 checks described in the Android test docs.

## Follow-up

- Add the Windows/Linux/iPhone runtime observations to the relevant audit rows
  when available.
- Keep the fuzz seeds current with new parser formats; rerun after parser or
  bridge changes.
