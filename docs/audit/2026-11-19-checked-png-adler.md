# Checked Adler-32 for PNG decoding

Workbench's local `simd-adler32` 0.3.10 uses the already locked `adler2` 2.0.1 implementation. Both Rust crates forbid unsafe code. The local wrapper preserves seeded and incremental checksums, reset, the one-shot API, and the `std` stream helpers. The fork omits the hidden per-architecture modules and retains the portable scalar helper. It keeps the upstream MIT license and feature names. [Source and license notice](../../src-tauri/vendor/simd-adler32-0.3.10/SOURCE.txt)

The normal and build-time graphs select the local package on nine app target triples. The desktop Mac target reaches it through `fdeflate` for image streams and Miniz 0.8.9 for PNG metadata and related zlib data. The other seven checked triples reach it from host-side code generation or build dependencies. The Windows graph also reaches it through Zopfli in the speech SDK build. Miniz 0.9.1 and Glass already use the portable checksum path. [Earlier DEFLATE and Glass verification](2026-11-19-checked-deflate.md)

## Verification

With the published default features, the macOS ARM64 compile of `simd-adler32` emitted one `unsafe_code` diagnostic and the Linux x86-64 compile emitted 24. The local wrapper emitted zero for both targets. These are library compile counts. The other codec dependencies have their own verification.

The local package passed 15 release tests with all features and 11 with default features disabled on each of macOS ARM64 and Linux x86-64, including doctests. Tests exercise noncanonical seeded states, empty writes, resumption, reset and input lengths around both checksum boundary sizes. The independent source reviewer verified the MIT notice, locked consumer APIs and overflow bounds, and ran the same eight unit tests in both feature configurations. Standalone compile checks covered ARM64 iOS, ARM64 iOS Simulator, ARM64 Android and ARMv7 Android, each with all features and with default features disabled.

Both standalone builds decoded the same 8 MiB RGBA pixels and matched the checked application icon's pixel checksum. When asked to verify Adler-32, both rejected a synthetic PNG with a bad Adler-32 value and a recomputed PNG chunk CRC. The default check policy still accepted that fixture and returned the same pixels. Separate Mac tests exercised Miniz 0.8 through the PNG compressed-text API. Both builds returned the original Latin-1 text and rejected its corrupted zlib checksum.

The Mac `custom-protocol` application test executable built from base `cc36ec2f51b6c0f71196bf01f264efee9ed974bd` and the checked dependency. Its signature verified and all 31 focused archive, metadata, Glass and updater tests passed. The first full test run needed the frontend dependencies for its PDF bridge test. After those were linked, a parallel run saw temporary lock contention in a profile restart test and an import retry test. Both passed in isolation. The same executable then ran the full suite serially in a fresh temporary directory, with 1,281 tests passing and 25 ignored.

## Performance

The tests used the locked PNG 0.18.1 dependencies and changed only the Adler-32 package. Each row uses 12 alternating pairs after a discarded warm-up pair. The probe read `src-tauri/icons/icon.png` with Adler-32 verification enabled. The 2048 × 1024 fixture uses zero-based coordinates and RGBA bytes `(x % 256, y % 256, ((x // 16) ^ (y // 16)) % 256, 255)`. Each row starts with a filter-zero byte. Python's zlib compresses the rows at level 6 into one IDAT chunk. Timed operations used already loaded bytes. The PNG cases checked the decoded output outside the timer. The direct checksum case compared each result with Python zlib's reference value during the timed loop. [Fixture hashes, raw wall time, CPU and peak resident memory](../benchmarks/2026-11-19-checked-png-adler.json)

| Host and operation | Published median (ms) | Checked median (ms) | Median paired ratio |
| --- | ---: | ---: | ---: |
| macOS ARM64, 8 MiB checksum | 0.257 | 1.492 | 5.790 |
| macOS ARM64, application icon | 0.462 | 0.627 | 1.354 |
| macOS ARM64, 8 MiB PNG with default policy | 20.236 | 20.195 | 0.998 |
| macOS ARM64, 8 MiB PNG with Adler-32 verification | 20.446 | 21.706 | 1.063 |
| Linux x86-64, 8 MiB checksum | 0.417 | 1.166 | 2.792 |
| Linux x86-64, application icon | 0.416 | 0.641 | 1.535 |
| Linux x86-64, 8 MiB PNG with default policy | 29.395 | 32.376 | 1.109 |
| Linux x86-64, 8 MiB PNG with Adler-32 verification | 29.904 | 34.181 | 1.152 |

Both checked 8 MiB decodes finished below 35 ms at the median. The largest median increase was 4.28 ms on Linux. These tests cover the standalone PNG path. Application latency, embedded browser rendering and mobile hardware require separate evidence.
