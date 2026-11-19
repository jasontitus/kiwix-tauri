# Checked DEFLATE output buffers

Workbench uses a local `flate2` 1.1.11. The old vector APIs wrote into uninitialized spare capacity and set the vector length from the codec's output counter. The local wrapper initializes the existing allocation, passes a checked slice to the codec and keeps the reported output. A guard restores the original length on unwind. Miniz's compatibility methods for `MaybeUninit` output use a temporary initialized buffer sized to the caller's slice. Allocation failure returns a codec error before the codec advances. The fork preserves both upstream licenses. [Source and bundle notice](../../src-tauri/vendor/flate2-1.1.11/SOURCE.txt)

The default macOS release compile went from four `unsafe_code` diagnostics in `flate2` to zero. The wrapper enforces `forbid(unsafe_code)` in its Rust backend configurations. The nine checked app target graphs select Miniz 0.9.1 without its SIMD Adler-32 feature. That removes the SIMD dependency edge from the shared Miniz version used by the GPL Glass engine. Tauri's host icon generator still selects the separate zlib-rs backend. PNG's `fdeflate` and Miniz 0.8 still have their own Adler-32 paths. Async compression still adapts Tokio's uninitialized buffers through its own unsafe code. [CRC and remaining checksum paths](2026-11-19-checked-crc32.md)

The app work here is in dependency coverage. The parallel session owns the exclusive-publication and diagnostic-clock changes in rows B3 and B10. [Memory-safety coordination at `1ef9abe`](https://github.com/jasontitus/kiwix-tauri/blob/1ef9abe5ada134151920e0ad2a9b6e96e5a0aa2b/docs/COORDINATION-MEMORY-SAFETY.md)

## Verification

On macOS, 125 Miniz release tests, 138 zlib-rs tests and 138 optional C-zlib tests passed, including doctests. Linux passed 125 Miniz tests. Tests exercise an existing vector prefix and allocation, unwind, initialized and uninitialized output, gzip members, partial writes, corrupt and truncated data, and flush behavior. Four standalone cross-target checks covered iOS ARM64, iOS Simulator ARM64, Android ARM64 and Android ARMv7.

The Mac `custom-protocol` application test build at base `2c11ad68deef150b3ba109bec39231b90be4a0f5` selected the local source for host and target. Its signature verified and all 27 selected updater, archive and search tests passed. Independent review checked the exact dependency and resource inputs, including each backend and the buffer's behavior on errors, and repeated 12 focused release tests.

The signed loopback updater test also passed with that executable. It installed the verified gzip bundle into a test-owned directory, checked the new version and rejected the modified download. Both test bundles were copies of an existing app, with executable names aligned to the runner's fixture contract. The final fixture archive omitted Apple metadata via `COPYFILE_DISABLE=1` after a sidecar entry blocked unpacking. The runner removed its temporary installation after the check.

The MIT tools at `3861631f` and the separate GPL tools at `380d84d7` reproduced both previous complete CJK result files byte for byte. The comparison used Miniz with the portable checksum path. Xapian 1.4 matched 63 of 63 ordinary queries and 6 of 12 rare-Han queries. Xapian 2.0 matched 61 of 63 ordinary queries and 6 of 12 rare-Han queries. The two extra differences were the fullwidth queries. The 1.4 native builder and index checker used Xapian 1.4.32. Its Python libzim query oracle reported Xapian 1.4.23. The 2.0 tools and oracle reported 2.0.0. This is a standalone builder and search comparison. [Existing parity method and result hashes](2026-11-18-spelling-platforms.md#search-compatibility)

## Performance

The reference used the published `flate2` and the same checked CRC32 dependency. The candidate changed the buffer code and Miniz checksum selection. Both hosts used the same 8 MiB article payload from the CRC comparison, with the three-byte alignment prefix removed. Python generated one gzip member at level 6 with `mtime=0` and one zlib stream at level 6. Each case ran 12 alternating pairs after a discarded warm-up pair. Both variants decoded identical bytes and rejected corrupted checksums. Both gzip read paths also rejected a truncated trailer. The async case uses the pinned Tokio/async-compression adapter with 8 KiB output chunks. The writer feeds 8 KiB chunks and checks the resulting payload by decompressing it. [Article fixture recipe](2026-11-19-checked-crc32.md#performance) · [Wall time, CPU and peak process memory samples](../benchmarks/2026-11-19-checked-deflate.json)

| Host and operation | Published median (ms) | Checked median (ms) | Median paired ratio |
| --- | ---: | ---: | ---: |
| macOS ARM64, read gzip | 18.797 | 18.810 | 0.997 |
| macOS ARM64, async gzip | 19.027 | 19.309 | 1.012 |
| macOS ARM64, read zlib | 19.055 | 20.204 | 1.064 |
| macOS ARM64, write gzip | 128.007 | 128.745 | 1.010 |
| Linux x86-64, read gzip | 18.834 | 18.846 | 1.002 |
| Linux x86-64, async gzip | 18.952 | 19.135 | 1.010 |
| Linux x86-64, read zlib | 19.125 | 20.276 | 1.048 |
| Linux x86-64, write gzip | 186.719 | 187.851 | 1.004 |

Median peak resident memory moved by less than 0.07 MiB per case. The reported CPU and memory cover the complete benchmark process. The elapsed timer covers the codec operation on already loaded data. Larger caller-selected buffers can increase memory in the Miniz compatibility methods. Mobile and Windows runtime performance require separate measurements.
