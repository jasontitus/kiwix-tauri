# Zimru reader dependency reduction

Workbench links the MIT reader library with default features disabled at revision `cbafcb41da64c390c7889b122c677f23bcde8dc5` in the MIT repository; it contains the reviewed `3861631f` reader branch. The `parallel` feature supplies the Rayon API, and `cli` builds `zimcheck` and `zimdump` and enables the Unix SIGPIPE helper. The default Zimru build still provides both tools and the writer. Standalone builds with default features disabled can still build `zimru`, `zimbench` and `zimsplit`. Their source stays in the MIT history. Kiwix consumes the separate GPL engine through its `da3221d79734dab7e7e3415af1bb3454f8608250` pin. [MIT merge](https://github.com/jasontitus/zimru/commit/cbafcb41da64c390c7889b122c677f23bcde8dc5)

Compared with Workbench base `133f946983ab4e165812036d8befb18d4435b07c`, the locked normal app graphs omit `rayon`, `rayon-core`, `crossbeam-deque` and `crossbeam-epoch` for nine targets. These are ARM64 and Intel macOS, ARM64 iOS and iOS Simulator, ARM64 and ARMv7 Android, ARM64 and x86-64 Linux, and x86-64 Windows MSVC. `sigpipe` disappears on the eight Unix targets. The app still reaches `crossbeam-utils` through dependencies such as `crossbeam-channel` and `dashmap` on macOS. Windows dependency choices outside Zimru were preserved in the lockfile. The graph comparison accounts for package selection. App-owned bridges and the remaining transitive code still need coverage.

## Direct reader timings on Apple Silicon

The same standalone drivers exercised patched libzim 9.8.2 and MIT Zimru before (`ff8b0ab5e561b0019151d6d614131f59920ff2ee`) and after the feature change. Across six workloads on each of two real archives, all three builds returned identical length-framed responses. Each phase used a fresh process and the same 4 MiB requested cluster-cache budget. The harness discarded one warm-up per reader and measured six trials per reader per phase, using all six reader orders. The filesystem cache was warm and native Xapian preload was disabled. The Rust builds used rustc 1.99.1 with the same release settings.

| Archive | Reader API workload | libzim (ms) | Prior Rust (ms) | Current Rust (ms) |
| --- | --- | ---: | ---: | ---: |
| Climate | Open | 2.034 | 0.057 | 0.061 |
| Climate | First article | 1.294 | 1.480 | 1.492 |
| Climate | 32 warm reads | 0.034 | 0.045 | 0.051 |
| Climate | 24 random articles | 16.472 | 27.135 | 27.032 |
| Climate | Title prefixes | 0.112 | 0.174 | 0.182 |
| Climate | 16 media ranges | 2.103 | 2.051 | 2.039 |
| Wikipedia 100 | Open | 2.167 | 0.052 | 0.055 |
| Wikipedia 100 | First article | 1.072 | 2.228 | 2.265 |
| Wikipedia 100 | 32 warm reads | 0.074 | 0.081 | 0.087 |
| Wikipedia 100 | 24 random articles | 31.902 | 41.062 | 41.182 |
| Wikipedia 100 | Title prefixes | 0.153 | 0.140 | 0.139 |
| Wikipedia 100 | 16 media ranges | 0.192 | 0.111 | 0.116 |

The timed reads exclude opening the archive and framing the responses. The random workload reads up to 64 KiB per article. All selected Climate articles fit within that limit, and all 24 Wikipedia 100 reads reached it. Media ranges are 4 KiB each. The random-read medians moved less than 0.4% versus the prior Rust build. Libzim still led the random-read workload on both archives. The two sampled Rust workloads finished below 50 ms at the median for the direct API. User-facing latency and current-pin speed on mobile and Linux require their own measurements. [216 timed samples, archive identities and verified response hashes](../benchmarks/2026-11-19-zimru-reader-graph.json.gz) · [Driver setup](../../scripts/bench-archive-readers/README.md)

## Verification

Zimru's offline locked test runs passed in four configurations. Reader-only ran 103 tests, CLI without the writer ran 117, default ran 188, and all features ran 197, including doctests. Some archive tests depend on optional local fixtures. The all-feature build also generated the same C header. The independent source review checked the feature/API contract, test registration, and normal and build-time graphs for five platform families. A second review matched the benchmark binaries, archives and Rust sources to their recorded identities, recomputed all medians and checked the scope of the byte-equality and performance claims.

The exact Workbench lockfile built the macOS ARM64 library test executable with `custom-protocol`. Its signature verified and its dynamic imports excluded libzim and Xapian. All 33 selected tests passed across archive reads and integrity, metadata, indexed search and provider references. The Mac packaging check passed four cases and skipped its Linux-only ELF case. The other platforms in this checkpoint were checked for dependency selection. The earlier packaged reader results remain recorded in the [platform verification report](2026-11-18-reader-platforms.md).
