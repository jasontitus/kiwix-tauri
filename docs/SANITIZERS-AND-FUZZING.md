# Sanitizers, fuzzing and platform test handoff

This records the local sanitizer/fuzz pass on 2026-11-20. It is evidence for the
tested paths only; it is not a device, Windows, Linux or binary-level
certification.

## Local results

| Check | Result | Scope |
|---|---|---|
| LocalSwarm Objective-C++ ASan + UBSan | Pass, 12 checks, no sanitizer report | `native/localswarm_tests.mm` + `native/localswarm.mm` |
| ZIM archive parser fuzz | 100,000 seeded runs, no crash; 1,549 coverage edges | `zimru::Archive::from_positioned` and entry/check paths |
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

The `-Zub-checks` suite's only failure is
`update::tests::signature_verification_rejects_a_tampered_payload`, because the
test binary ran from a `/tmp` symlink and the updater rejects that path. The
targeted storage test and the other 1,286 tests pass with UB checks enabled.

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
  tracked.

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
