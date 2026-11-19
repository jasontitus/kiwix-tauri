# Checked SHA-512 loads and ARM64 feature detection

The RustCrypto SHA-1 and SHA-2 forks now use the standard runtime detector on ARM64 Linux and Android when the `std` feature is enabled. The app enables it for both dependencies. It uses the software backend when the detector declines the required feature. On those targets, a build of either library without `std` uses hardware when the compiler target guarantees the instructions and software otherwise. The Apple and x86 detection paths and the explicit backend overrides keep their existing dispatch rules.

The previous dependency tested the Linux SHA-256 capability before entering the shared ARM64 SHA-1 feature path. Its SHA-3 feature check accepted either of two capability bits. Partial capability reports could therefore select instructions without establishing their prerequisites. The older Android support range also includes systems for which Rust's detector accounts for differences between cores. The review traced the failure condition in source. A crash on affected hardware remains unobserved.

The ARM64 SHA-512 compressor now reads checked big-endian words and builds its state vectors from integer lanes. The x86 AVX2 compressor uses checked input words and extracts the scheduled values directly from registers. It still processes two blocks at a time when a complete pair is available. The targeted compressor modules forbid unsafe Rust. Calling them still requires the CPU feature preconditions. [SHA-1 notice](../../src-tauri/licenses/sha1/SOURCE.txt) · [SHA-2 notice](../../src-tauri/licenses/sha2/SOURCE.txt)

## Source review

Independent review covered the 16 implementation, regression and attribution paths in `cf7e08e2809b1f8224dabd4a5254149fb97c0134` against `eb12e9afdedef301df47db36c0f20aa73afdc05a`. The sorted file manifest has SHA-256 `770031a92b4948db12cdea212ee6dbec517c17b02700178f6369d4ca57ff792a`. The review confirmed that both app dependencies select `std`, that their target builds on ARM64 Linux and Android drop the direct `cpufeatures` edge, and that the original license texts and upstream crate archives are unchanged. The new regression stays in the GPL verifier.

The final Mac verifier passed all ten checks both with default dispatch and with the SHA instruction paths explicitly enabled. Both runs include the 4 GiB SHA-256 boundary. The verifier also passed nine checks with its default features disabled, with the 4 GiB case left ignored in that run. SHA-512 compression matches the separate implementation from arbitrary states using offsets 0, 1 and 15 and batch sizes 0, 1, 2, 3, 127 and 128. The locked standalone SHA-1 and SHA-2 all-feature suites passed five and sixteen tests respectively, including documentation tests.

The prior standalone detector probe used the same dispatch logic with the modern SHA-1 and SHA-2 libraries built with and without `std`. Across both builds and two emulated Linux ARM64 CPU models, all 196 digest comparisons matched Python. The emulated Cortex-A53 reported SHA-2 and lacked SHA-3. The emulated maximum-feature CPU reported both. Disassembly found SHA-1, SHA-256 and SHA-512 instructions in the unforced build with the detector. The libraries built without `std` produced no such instructions in that executable. Both Android variants compiled. That probe covered simulated Linux CPUs and compilation for Android.

## Desktop and iOS verification

The optimized Mac app library suite passed 1,259 tests with 25 ignored. The Ubuntu suite passed 1,242 with 16 ignored. Native Linux passed all ten standalone verifier tests, including the 4 GiB case. Its forced AVX2 run passed the seven streaming and compression checks. A separate QEMU x86-64 run exposed SHA, SSE4.1 and AVX2. It passed the nine shorter cases using automatic CPU selection and left the 4 GiB case ignored.

The Ubuntu native app and extraction-mode AppImage returned matching archive-service data across the earlier and current builds. The recorded cases include single-file and split archives, reopening, and ranged image serving. In eight alternating 256 MiB loopback pairs against the preceding Rust build, the elapsed median moved from 1.720 to 1.715 s and the process CPU median from 1.670 to 1.660 s. The stopwatch covers the transfer through observed completion and the recorded times have 0.01 s resolution.

Both ARM64 iOS target checks passed on retry with accessible Swift compiler caches. The signed iOS 26.5 simulator build passed its Wikipedia import, search, spelling correction, article reading and cold-launch restoration test. The source and imported archives matched, as did all three installed images and 73 notices. The owned simulator was deleted. The portable ZIP was regenerated to omit macOS provenance sidecars. It contains all 165 signed files and passed strict signature verification after extraction. Physical iPhone speed and memory remain unmeasured.

## Android verification

The ARM64 and ARMv7 Android app checks passed when retried after the release APK build. The package verification initially used the Mac frontend fingerprint and stopped before signing. After correction, the Android build's 101 frontend input files matched the previous Android build. The signed ARM64 package included the 73 expected notices and carried five ARM64 libraries aligned to 16 KiB. The required imports of the app library resolved in its transitive Android API 24 dependency closure.

A standalone executable built from the reviewed SHA inputs ran first on the owned API 35 ARM64 emulator. Its standard detector reported SHA-2 and SHA-3 available. All seven digests matched the independent references at seven input partition sizes, and the runner removed the probe. The installed app imported the Climate Wikipedia archive, displayed title and full-text results, built its spelling index, opened corrected results and restored reading state after a cold launch. The runner removed the test app and its fixture, stopped the emulator and deleted the profile. Physical Android performance and ARMv7 runtime remain unmeasured.

## Mac reader and download performance

The current signed Mac app, its signed Rust predecessor and the recorded libzim-backed app returned matching previews and matching MHTML after normalizing the generated MIME boundaries. Across two Wikipedia archives and a small export fixture, each app ran 42 preview commands and 12 export commands. The app order was balanced across six rounds per input. Timings include local IPC and file output for MHTML.

| Operation | libzim-backed median | Previous Rust median | Checked Rust median |
| --- | ---: | ---: | ---: |
| Article preview | 4.98 ms | 4.78 ms | 4.80 ms |
| MHTML export | 24.48 ms | 22.21 ms | 22.85 ms |

All sampled commands finished within 30 ms. Both Rust builds accepted both Wikipedia archives with valid checksums and rejected the incorrect digest for each archive. The new build reopened both preceding download records. The test runner independently read and hashed the saved files. Across six alternating pairs per archive, median start-to-observed-completion times moved from 193.31 to 195.72 ms for Climate and from 223.08 to 227.47 ms for Wikipedia 100. The separate 256 MiB app-library loopback test moved from 0.58 to 0.59 s across twelve pairs, with 0.40 s median process CPU on both sides. [Exact artifacts and raw samples](../benchmarks/2026-11-19-checked-sha-backends.json)

## SHA-512 performance

The load implementations were measured in standalone executables before promotion. Twelve alternating pairs per input on Apple ARM64 put the median paired checked-to-previous elapsed ratio between 0.996 and 1.003 for a 111-byte input and two Wikipedia archives. A separate native Linux x86-64 AVX2 test measured paired median elapsed decreases of 6.0% for Climate and 7.5% for Wikipedia 100. Both probes hashed resident bytes and checked their digests against Python. These measurements cover hashing on the sampled hosts. [AVX2 samples](../benchmarks/2026-11-19-native-digest-blocks.json)

## Remaining boundaries

The scope is the modern vendored SHA-1 and SHA-2 paths. Other versions of these providers, other `cpufeatures` consumers, the older core APIs that accept wrapped SHA-2 arrays, and AWS-LC remain separate. The RISC-V, LoongArch and optional Wasm backends still use architecture-specific operations. [Memory-safety worklist](../MEMORY-SAFETY-INVENTORY.md)
