# Borrowed native blocks for SHA-1 and SHA-2

SHA-1 and SHA-2 now receive complete runs of streaming input as borrowed bytes. The digest buffer still owns an incomplete block. It sends a completed partial block first, then sends the contiguous run directly from the input and retains the tail. The lazy buffer retains the last full block when there is no trailing partial block. The existing wrapped-block interface and its default digest core implementation still support callers such as HMAC. [Digest source and notices](../../src-tauri/licenses/digest/SOURCE.txt)

On ARM64 and x86, the Rust SHA-1 compressors read checked big-endian words into register lanes and extract the resulting state words directly. The x86 SHA-1 instruction layout places the first message word in the high lane. ARM64 uses forward lane order. CPU feature checks still guard dispatch. The packaged forks carry their original MIT and Apache licenses. [SHA-1 source and notices](../../src-tauri/licenses/sha1/SOURCE.txt)

## Independent review and verification

The independent source review covered 65 implementation, test, dependency and notice paths against `039a70c16008340807d8f2d84414d7a545305a1c`. The sorted path and SHA-256 manifest hashes to `f60ad3598be897a995e688abd8e3177aaffa92d328d858233abe6e5f6446f722`. It verified both complete published archives and the packaged license texts. The review caught a standalone build failure because the vendored SHA crates selected the registry digest API when built directly. Both standalone manifests and lockfiles now select the paired local crates. The reviewer repeated the locked standalone tests.

The application verifier passed nine tests on macOS ARM64 with its default, forced hardware and forced software paths. It exercises the 4 GiB boundary, all six SHA-2 variants, SHA-1 compression from asymmetric states and unaligned input, and SHA-1 streaming against independent expected digests. It also checks reset, the preserved core interface, native batch grouping, direct input borrows and retry after a callback panic. The forced x86 SHA builds passed the six integration tests and two shorter SHA-256 differential tests in QEMU. The independent HMAC consumer matched Python on all 14 comparisons across SHA-1 and SHA-256 and exercised the generic fallback used by the host build.

Standalone SHA-1 passed five tests, including doc tests, with the exact Rust 1.85 compiler. Standalone SHA-2 passed sixteen tests, including doc tests. The checked buffer passed all sixteen tests in its locked all-feature suite, including doc tests.

SHA-1 and all six SHA-2 streaming checks passed with both legacy grouping and wrapper-to-native conversions forced to panic. The callback-panic retry and SHA-1 partition tests passed under Miri's default and Tree Borrows modes with the software compressor. The larger batch matrix was stopped under Miri and passed in the compiled verifier. A Rust 1.85 LoongArch executable passed one-shot, partitioned and wrapped SHA-1 checks against independent digests in QEMU 8.2.2.

## Desktop application verification

The unfiltered optimized Rust app library test binaries passed 1,259 tests with 25 ignored on macOS and 1,242 tests with 16 ignored on Ubuntu. The desktop products use the 62-path runtime source snapshot `f9195e639fef51586ad8d915ec3900ad317f7bc104e02782464a76e781998e68`. The final implementation also contains fixes to six standalone manifest and lock files. The normalized application dependency graph matched on all six targets. The app-library compile checks passed for ARM64 iOS and iOS Simulator and ARM64 and ARMv7 Android.

The Mac bundle passed strict signature verification. It and the Ubuntu AppImage included the six new notice files byte for byte. The Ubuntu native executable and AppImage in extraction launch mode matched the recorded single and split archive responses, including reopen and ranged image serving.

## Android emulator verification

The signed ARM64 test APK `3198102278123d974acee96348cd98d7cd20a69543dd2196b2e7527bac6ffca7` used all 65 reviewed implementation files from `00dbf69550c93e96befb1dba27b0770ebd0cf92f`. Package review verified all 73 notices, the five ARM64 libraries and the app library's Android API 24 dependency closure. The API 35 emulator opened the Climate Wikipedia archive through the file picker, rendered title and full-text results, built the spelling index and restored the open article and correction behavior after a cold launch. The runner removed its test app and fixture, stopped the emulator and deleted its profile. Physical Android performance and ARMv7 runtime remain unmeasured.

## iOS simulator verification

The signed ARM64 simulator bundle ZIP `7b16c75002f3cfb1e0a9028aadedbae0c9213f8e42b7ac4a10377d28e39e4c40` used revision `00dbf69550c93e96befb1dba27b0770ebd0cf92f`. The iPhone 17 simulator on iOS 26.5 imported the Climate Wikipedia archive, opened search results, built its spelling index, corrected a misspelling and restored the open article and correction behavior after a cold launch. The reviewer reconciled the passing XCTest result, the identical source and imported archive hashes, all three installed ARM64 images, all 73 notices and the deletion receipt for that simulator instance. Physical iPhone speed and memory use remain unmeasured.

## Reader and download performance

The Mac comparisons used the signed checked executable, the preceding signed Rust-reader executable and the recorded libzim-backed executable. All three produced matching article preview responses and matching MHTML bytes after normalizing the generated MIME boundary. Six orders per archive balanced which build ran first, second and third. Each build produced 42 previews and 12 MHTML exports across two Wikipedia archives and the preview fixture. The command timer includes local IPC and MHTML file output.

| Mac operation | libzim-backed median | Previous Rust median | Checked Rust median |
| --- | ---: | ---: | ---: |
| Article preview | 5.08 ms | 4.64 ms | 4.80 ms |
| MHTML export | 24.87 ms | 21.82 ms | 22.51 ms |

Every preview and export sample from all three builds finished within 50 ms. The largest checked sample was 36.08 ms. Median per-run sums for the three previews on Wikipedia 100 were 23.23 ms with libzim, 26.13 ms with the previous Rust build and 25.64 ms with the checked Rust build. The aggregate preview medians above pool the individual commands across all three inputs.

The two signed Rust builds accepted both real Wikipedia ZIMs with the correct Metalink SHA-256 and rejected a wrong digest for each without publishing the destination. The checked build reopened both predecessor download records. The test runner independently verified the saved file hashes against the digests reported by the app. For six alternating pairs per archive, elapsed medians from start to observed completion moved from 210.90 to 212.13 ms for Climate and from 276.20 to 277.13 ms for Wikipedia 100. These are local transfer and verification timings and include completion polling.

The 256 MiB application-library loopback benchmark used the immediately preceding Rust checkpoint as its baseline. On the Mac, twelve alternating pairs moved median elapsed time from 0.94 to 0.95 s and CPU time from 0.64 to 0.655 s. On Ubuntu, eight alternating pairs held the elapsed median at 1.71 s and moved median CPU time from 1.655 to 1.67 s.

A separate Mac ARM64 probe measured SHA-1 and SHA-256 over already-read bytes, from the two Wikipedia archives and a 111-byte input. Across twelve alternating pairs per algorithm and input, the median checked-to-previous ratios ranged from 0.994 to 1.007. Both executables checked every digest against the result of a warmup verified with Python. [Samples and artifact identities](../benchmarks/2026-11-19-native-digest-blocks.json)

## SHA-512 AVX2 prototype

A separate prototype replaced pointer loads and stores in the SHA-512 AVX2 compressor with checked byte reads and register extraction. All seven tests passed on native and emulated x86-64 AVX2, including batches of zero, one, two and three blocks and unaligned input. Across twelve alternating pairs on one Linux x86-64 host, elapsed time to hash resident bytes fell by a paired median of 6.0% for the 8.98 MB Climate archive and 7.5% for the 36.52 MB Wikipedia 100 archive. Each digest matched Python. Disk I/O, 32-bit x86 execution and application performance remain unmeasured. The prototype was tested separately from the packages above.

## Remaining memory boundaries

The generic digest implementations still accept wrapped arrays through their existing block callback. The public low-level SHA-2 cores still use the corresponding layout conversion for callers that supply wrapped blocks directly. Those operations rely on the fixed array size, the transparent wrapper and the input slice's borrow. Some serialization and read-buffer conversions also remain in `hybrid-array`. On LoongArch, the SHA-1 backend still uses assembly with a pointer derived from one live byte or wrapped-block slice and a block count derived from the length. Both input forms retain one assembly call per batch. The CPU feature and architecture-specific boundaries remain part of the worklist. [Memory-safety worklist](../MEMORY-SAFETY-INVENTORY.md)

```sh
cargo test --locked --release --manifest-path src-tauri/vendor/sha1-0.11.0/Cargo.toml --all-features
cargo test --locked --release --manifest-path src-tauri/vendor/sha2-0.11.1/Cargo.toml --all-features
cargo test --locked --release --manifest-path scripts/check-sha256-loads/Cargo.toml -- --include-ignored
```
