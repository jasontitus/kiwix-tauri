# Release compiler settings for archive reads

Compiling Zimru’s Zstandard codec in 16 Rust code-generation units reduced the median time for 24 random article reads on each of four platforms. Against the app’s previous setting, savings ranged from 7% to 16% across the eight primary cases. Native libzim remained faster on all eight. The closest result was Wikipedia 100 on macOS, at 47.82 ms for Rust and 47.10 ms for native.

The app uses this release setting at `7c2dd5d`. Both Rust builds used the same MIT reader at `ff8b0ab`, with the same archive limits, checksum checks, compiler optimization level and 32 MiB cluster cache. The original app profile used one code-generation unit per crate. The new profile changes only the `zstd-rs` package. This comparison measures public archive operations in standalone executables. [App profile change](https://github.com/jasontitus/zimru-xapian/commit/d072a7d) · [Previous decoder study and full-app import measurements](2026-11-18-portable-decoder.md)

## Random article reads

Each row is the median time in milliseconds to resolve the same 24 paths and return up to 64 KiB for each article. Native and both Rust builds returned identical bytes. Each primary comparison discarded one warm-up per reader and used 12 fresh processes per reader per case. Trial order balanced the three readers. OS caches were warm. The mobile numbers come from virtual devices on a Mac also running builds.

| Platform | Archive | Native libzim | Original app setting | Tuned app setting |
| --- | --- | ---: | ---: | ---: |
| macOS ARM64 | Climate | 23.22 | 31.08 | 27.86 |
| macOS ARM64 | Wikipedia 100 | 47.10 | 56.59 | 47.82 |
| Linux x86-64 | Climate | 25.49 | 38.67 | 35.90 |
| Linux x86-64 | Wikipedia 100 | 48.88 | 63.91 | 57.59 |
| iOS 26.5 ARM64 simulator | Climate | 37.76 | 59.48 | 52.81 |
| iOS 26.5 ARM64 simulator | Wikipedia 100 | 39.18 | 55.98 | 48.14 |
| Android 15 ARM64 emulator | Climate | 103.34 | 258.58 | 224.23 |
| Android 15 ARM64 emulator | Wikipedia 100 | 108.84 | 180.65 | 157.16 |

The first article also improved on all eight cases. Media ranges had mixed results. On the iOS simulator the Wikipedia 100 median went from 0.19 to 0.20 ms, versus 0.33 ms for libzim. On Android the initial Climate media median rose from 21.79 to 26.43 ms. A 24-round Android repeat measured 10.49 and 11.02 ms, while the tuned reader used less CPU in 16 of 24 paired media trials. Random Climate reads improved again in that repeat, from 143.74 to 119.00 ms. The Android media results vary too much to establish a consistent direction. Linux measured first and random reads in this comparison. [All 936 timed samples, binary hashes, response hashes and process memory](2026-11-18-reader-build-profiles.json)

## Where first reads spend time

A second Mac experiment compared the first 4 KiB of an early and a late article in the **same compressed cluster**. Each request opened a fresh process. Across four Climate clusters with 1.85–2.10 MB of decoded payload each, native libzim read the early article in 0.21–0.29 ms and the late article in 3.45–4.64 ms. Tuned Rust took 3.39–3.96 ms early and 3.75–4.46 ms late. Four Wikipedia 100 clusters also showed a smaller early-to-late difference for Rust. These are ranges of per-article medians. Every framed response matched byte for byte. Selection used HTML items of at least 4 KiB, the lowest and highest starting offsets in each selected cluster and eight measured processes per reader per item after a warm-up. [Article paths, cluster positions and individual timings](2026-11-18-reader-cluster-position.json)

Zimru currently decodes the complete compressed cluster and validates its offsets before returning any blob. To improve first reads, an incremental reader would need to preserve the current behavior for damaged checksums, bad trailing blocks and invalid offsets, including when a file changes after import. Comparing early and late items gives that work a measurable target. The timing also includes path lookup, which differs between articles. [Current cluster implementation](https://github.com/jasontitus/zimru-xapian/blob/ff8b0ab5e561b0019151d6d614131f59920ff2ee/src/cluster.rs)

## Reproduction

Build the two standalone Rust drivers from the same checkout of the pinned MIT source. Use its sibling path in the driver manifest and confirm the source Cargo resolved. Keep the native executable and both Rust executables separate. Apply identical target and compiler options apart from the package override. The native reference was libzim 9.8.2 on macOS, Linux and Android, and libzim 9.6.0 on the iOS simulator.

```sh
CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 cargo build --release --locked \
  --manifest-path "$driver/Cargo.toml" --target-dir "$trial/original"
CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 cargo \
  --config 'profile.release.package.zstd-rs.codegen-units=16' \
  build --release --locked --manifest-path "$driver/Cargo.toml" \
  --target-dir "$trial/tuned"
```

Use the Climate mini June 2024 and Wikipedia 100 maxi January 2024 archives and the saved request order. Use the 32 MiB cache setting for both readers and disable native eager search-index loading. Each process returns length-framed responses to compare with the saved SHA-256. The sample file records compiler versions, executable digests, elapsed reader API time, whole-process CPU and peak resident memory. [Requests and reference responses](2026-11-18-linux-archive-readers.json) · [Driver setup and phase definitions](../../scripts/bench-archive-readers/README.md) · [Android runner](../../scripts/bench-archive-readers/ANDROID.md)
