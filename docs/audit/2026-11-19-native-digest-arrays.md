# Native arrays in the digest stack

The app and its SHA-256 verifier use a local `hybrid-array` 0.4.15. Default construction and cloning now initialize the actual Rust array through `core::array::from_fn`. Indexing, iteration, borrowed conversions to allocated containers, and optional constant-time assignment use the native array's slice views. Torrent web-seed SHA-1 comparisons use checked indexing for the resulting digest bytes. The normal digest interfaces and their serialized formats are maintained. [Upstream archive and notices](../../src-tauri/licenses/hybrid-array/SOURCE.txt)

The fallible constructor writes through an indexed prefix. Its cursor advances only after an element has been written. If the generator returns an error or unwinds, standard slice drop glue drops that initialized prefix. The layout conversion for this cleanup keeps the same element count and exclusive borrow. The successful array or error value also stays owned by the constructor while it drops the generator.

## Independent review

The 29 implementation, test, dependency and notice paths were reviewed against `320b17cef3e6425e44787d51149bbea1cc1c6520`. The sorted SHA-256 and path manifest hashes to `5ab20abc91c1f96526b400db4b38f636aa3beee3b0a0261f011497301c64128e`. Both original licenses and the complete published archive are retained. The vendored tree contains the build inputs, tests and the upstream Clippy configuration. Upstream CI and coverage configuration are available in the archive.

An initial method on `ArraySize` could conflict with an external extension method of the same name. The reviewed revision keeps the helper in a private module and attaches it to the associated native array type. The reviewer compiled the external source pattern against the final revision.

The reviewer reproduced two resource cleanup failures in the published crate. If dropping a generator panicked after construction, completed elements were skipped. If dropping an element panicked while cleaning up a failed constructor, later elements were skipped. Focused regression tests verify that the revised constructors retain ownership and complete those drops. The review also exercised compile-time slice views, empty arrays, a 1,357,824-element zero-sized array, range indexing, and callback failures at different positions.

## Verification

All 100 locked, all-feature crate tests passed with Rust 1.85.0 and 1.99.1. The same tests passed under Miri's default and Tree Borrows modes. The SHA-256 differential passed for arbitrary compressor states, streamed partitions and cloned digests, including inputs immediately below, at and above 4 GiB.

The unfiltered optimized Rust app library test binaries passed 1,259 tests on macOS with 25 ignored and 1,242 tests on Ubuntu with 16 ignored. The first parallel Mac run timed out at the initial QUIC connection in the Nearby Sharing stop-and-restart test, before the archive transfer. Eight isolated runs passed on each of the previous and current binaries. A full rerun with the same parallelism passed. The timeout's cause remains open.

The app library compiled for ARM64 iOS and iOS Simulator and for ARM64 and ARMv7 Android. These mobile checks stopped at compilation. The retained Ubuntu executable returned the recorded results for single and split archives, reopen and ranged image responses.

The ad-hoc signed Mac package and Ubuntu AppImage included all three hybrid-array notices byte for byte. The AppImage matched the recorded archive-service responses in extraction launch mode. The signed Mac app downloaded and verified two real archives, rejected the incorrect advertised checksums without publishing the files, and reopened the previous app's download records.

## Performance

In paired Mac SHA-256 tests, median times with the published and local arrays were 119.60 and 119.99 ns for a 111-byte digest, 4.21 and 4.20 ms for an 8.6 MiB archive, and 17.16 and 17.10 ms for a 34.8 MiB archive. Both builds used the same compressor and digest buffer.

Median elapsed time for the 256 MiB loopback download went from 0.590 to 0.595 s on the Mac and from 1.695 to 1.710 s on Ubuntu. Mac CPU time was 0.390 s for both. Ubuntu CPU time went from 1.655 to 1.670 s. The two real signed Mac downloads took a median 201.07 and 235.02 ms with the preceding build and 204.38 and 240.83 ms with the checked build.

The signed Mac app's previews and normalized MHTML exports matched the previous app and the historical libzim build. Median preview time was 4.83 ms for the checked app and 5.15 ms for libzim. Median MHTML time was 21.61 and 24.21 ms, respectively. All sampled operations in those reader comparisons stayed below 50 ms. [Samples and package digests](../benchmarks/2026-11-19-native-digest-arrays.json)

## Remaining raw operations

The normal digest update still feeds `BlockBuffer::digest_blocks` through `Array::slice_as_chunks`. This groups complete blocks without copying. The SHA-1 and SHA-2 cores use `Array::cast_slice_to_core` to present the wrappers to their compressors. The block count, array size, borrow lifetime and transparent representation are still relied on. The public `const` slice methods, raw representation conversions and the fallible constructor's initialization boundary also remain. The local fork removes several reachable unsafe operations while retaining these boundaries. Hardware dispatch and older digest implementations have separate entries in the inventory. [Memory-safety worklist](../MEMORY-SAFETY-INVENTORY.md)

## Native block prototype

A separate prototype feeds whole input blocks directly to the Rust SHA-1 and SHA-2 compressors through borrowed native arrays. Independent tests checked the normal streaming path with both legacy conversion helpers forced to panic. They also compared callback batches with an unchanged buffer build, exercised callback failure and retry, and verified digest reset and state compatibility. The median candidate-to-baseline ratios within pairs ranged from 1.0025 to 1.0046 in three Mac SHA-256 workloads. App packages in this checkpoint use the reviewed array fork. The prototype still uses low-level SHA-2 wrapper conversions and SHA-1 hardware loads. Its LoongArch throughput remains unmeasured. [Prototype benchmark samples](../benchmarks/2026-11-19-native-digest-arrays.json)

```sh
cargo test --locked --release --manifest-path src-tauri/vendor/hybrid-array-0.4.15/Cargo.toml --all-features
cargo test --locked --release --manifest-path scripts/check-sha256-loads/Cargo.toml -- --include-ignored
```
