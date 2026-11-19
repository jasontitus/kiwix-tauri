# Checked async gzip buffers

The app selects local copies of `async-compression` 0.4.52, `compression-codecs` 0.4.47 and
`compression-core` 0.4.34. The Tokio read adapter initializes at most 64 KiB of the caller's
unfilled buffer per poll. The Flate codec writes to a checked slice. The adapter then advances the
caller by the reported byte count. The normal path reuses the caller's allocation.

If a buffered reader returns an input error after the codec produces output, the adapter returns a
successful short read. On the next nonempty read it returns the original error before polling the
input again, then lets the caller resume with the same codec state. The shared encoder and decoder
logic applies to both Tokio and futures. The Tokio regressions cover plain reads, a follow-up
`read_to_end`, an interruption before any output and a zero-capacity read with an error queued.
The two tests that expect a short read reproduced failures before the repair.

All nine app feature graphs select the local packages with `tokio,gzip`. They do not enable
`legacy-output`. A macOS ARM64 release compile found 3 unsafe-code diagnostics in the published async
adapter, 4 in the published codec crate and 10 in the published buffer crate. The corresponding
local builds each reported zero with the same gzip configuration. The adapter forbids unsafe Rust.
The core and codecs forbid it in the initialized-output configuration. Optional backends that
still need the original buffer representation explicitly select `legacy-output`. Tokio's internals
and nonselected codec implementations remain in the inventory.

The selected path is in librqbit's support for downloading IP allowlists and blocklists. Kiwix
leaves both URLs unset in its session options. This is a selected-code and dependency check.
The benchmark exercises the actual Tokio adapter directly with fixture data.

The three packages keep the MIT and Apache-2.0 texts, source commit and published package checksum.
The app bundles those notices. The MIT Zimru and separate GPL Glass pins and sources were unchanged.
[App integration manifest](../../src-tauri/Cargo.toml) ·
[Async adapter notice](../../src-tauri/vendor/async-compression-0.4.52/SOURCE.txt) ·
[Codec notice](../../src-tauri/vendor/compression-codecs-0.4.47/SOURCE.txt) ·
[Buffer notice](../../src-tauri/vendor/compression-core-0.4.34/SOURCE.txt)

## Verification

- The selected standalone suites passed 343 tests each on macOS ARM64 and Linux x86-64 with the
  app's checked `flate2` and CRC32 wrappers. The three focused read-error tests also passed in a
  debug build. Cases include an input error with and without progress, cancellation while pending,
  a filled-prefix check, partial reads, member handling and gzip checksums and trailers.
- The macOS all-features suite with `--cfg async_compression_unstable` passed 1,041 tests. The two
  buffer configurations passed 4 initialized and 11 legacy core tests. The optional feature
  suite exercises compatibility backends, including ones still outside the safe configuration.
- Standalone codec library checks passed for ARM64 iOS device and simulator, and Android ARM64
  and ARMv7. These were compile checks without device runtime or throughput measurements.
- The exact macOS `custom-protocol` app test binary passed strict signature verification.
  Its 43 selected updater, archive, search and torrent publication tests passed, with one
  ignored. Four package configuration checks passed and the Linux-only check was skipped.

Independent review used base `8431794e392f4b9b2ae149f407cbdcaa6c051134` and the 151-path source
manifest `532959251cf3f9effedab088e601020d2e0584f1e31864aeab1eb798a09ad07e`. It traced both poll
contracts, checked the feature and license inputs, and reran the 358 selected async and core
tests. The source review's progress-before-error finding was corrected and independently rechecked.

## Performance

Both implementations used the same checked Flate and CRC32 dependencies. The reference used the
published async, codec and buffer crates. The candidate used the local copies. Both decoded the
same 8 MiB HTML fixture in memory and rejected a bad gzip checksum and truncated trailer. The write
case feeds 8 KiB input slices and decodes the result to verify the original payload. Each case used
12 alternating pairs after a warm-up pair. Elapsed time measures the operation on preloaded bytes.
The CPU and peak resident memory samples cover the process.
[Fixture recipe](2026-11-19-checked-deflate.md#performance) ·
[Individual wall time, CPU and memory samples](../benchmarks/2026-11-19-checked-async-gzip.json)

| Host and operation | Published median (ms) | Checked median (ms) | Median paired ratio |
| --- | ---: | ---: | ---: |
| macOS ARM64, 8 KiB reads | 19.617 | 19.320 | 0.9839 |
| macOS ARM64, 8 MiB caller buffer | 19.260 | 19.031 | 0.9872 |
| macOS ARM64, read\_to\_end | 19.114 | 18.903 | 0.9894 |
| macOS ARM64, write gzip | 130.452 | 128.687 | 0.9857 |
| Linux x86-64, 8 KiB reads | 19.329 | 18.994 | 0.9722 |
| Linux x86-64, 8 MiB caller buffer | 20.386 | 18.895 | 0.9280 |
| Linux x86-64, read\_to\_end | 19.753 | 19.043 | 0.9642 |
| Linux x86-64, write gzip | 188.889 | 187.809 | 0.9928 |

Median process RSS with the 8 MiB caller buffer rose from 38.07 to 40.44 MiB on Mac and fell from 37.79 to 29.92 MiB on Linux. The other cases and CPU samples are in the raw data.

These measurements cover the async gzip adapter. The archive reader and search engine remained
unchanged, so there is no new libzim or Xapian comparison in this checkpoint.
