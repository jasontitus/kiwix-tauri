# Memory-safety checkpoint

The app still runs native code on untrusted content. A fresh Mac release build counted 108 unsafe Rust sites in the app crate and 17,277 across 420 target-side packages. The previous build counted 110 and 17,379 respectively. The acceptance target trusts the OS and standard runtime and includes application code and its dependencies. The main remaining paths include native speech inference, rendering and media, Nearby connections and discovery, platform document handoffs and bundled database and networking libraries. [Boundary inventory](../MEMORY-SAFETY-INVENTORY.md)

## Changes saved privately

The [voice and receive integration](https://github.com/jasontitus/zimru-xapian/commit/f1a6c0890a3348982a4cb3588d85ef86f54296af) combines the Android factory-voice producer, reviewed voice installer and Apple Nearby receive change. The final three-file Linux test correction exercises the production profile layout. The first combined Apple packages used `bec2658`. The [subsequent Apple stream fix](https://github.com/jasontitus/zimru-xapian/commit/ce566218b035613cfd23ee34ded01ee6083bb113) delivers a final payload and preserves clean end-of-stream on the following read.

- The voice installer authenticates downloaded and bundled inputs, uses safe Rust bzip2 decompression and journals installation and removal. It preserves an existing installation when recovery cannot establish which model files belong to the app. Android treats an inconclusive factory-asset lookup as retryable. Native sherpa-onnx and ONNX Runtime still process text and models. [Installer checks and decoder timings](2026-11-18-voice-pack-decoder.md)
- Apple Nearby now binds each incoming read to a single-use ID, validates its size and retires the ID on cancellation. Native code returns a typed dispatch object and the request ID in place of a raw byte pointer and heap-owned Rust callback context. Native dispatch traversal, other callbacks and connection handles remain. [Apple receive tests and timings](../benchmarks/2026-11-18-apple-nearby-reads.md)
- The app still pins the MIT archive reader at `ff8b0ab` and GPL search at `380d84d`. These changes affect the GPL app. The optional MIT reader performance switch remains deferred. [Reader performance follow-up](../MEMORY-SAFETY-INVENTORY.md#performance-follow-up)

## Verification

| Platform | Scope and outcome |
|---|---|
| macOS ARM64 | The final ad-hoc signed app at `ce56621` contained both expected executables and all 44 license resources. It upgraded a real Climate spelling index, reopened it with 4,204 terms, and opened and served the Ray Charles article found by full-text search. The final optimized test executable passed 1,252 tests with 25 ignored by default. Separate runs passed four live native network tests and three genuine voice-pack installation, recovery and removal tests. |
| Linux x86-64 | The integrated voice binary, Debian package and AppImage at `7c5abf7` served checked article and media bytes. The GTK reader and the cold spelling-index workflow passed. With the test-only correction, 1,240 optimized tests passed with 16 ignored under each of umasks 002 and 022. All 39 focused voice tests passed under each mask, including three authentic-pack cases. Four launches through the public headless interface confirmed fresh profiles used mode 0700 and existing group-writable profiles kept their permissions and unrelated files kept their contents. |
| iOS ARM64 simulator | The final signed `ce56621` package passed 44 hosted tests and its reader UI test. It restored the saved Climate article before user navigation. Settings offered the standard voice download. No voice was downloaded or played. The corrected receive code also transferred all 600,123 bytes and reported clean EOF over real loopback TCP and QUIC in each of four simulator runs. |
| Android | The two-ABI offline appliance APK at `7c5abf7` passed signature and content checks for all 360 voice-pack files, 44 license resources and ten native images. With airplane mode enabled, Wi-Fi and mobile data disabled, and no default network on the ARM64 API 35 emulator, Settings showed the bundled model ready. Removal persisted across a cold launch. After an explicit offline reinstall, the model remained installed through a second cold launch. ARMv7 coverage was build and package inspection. |

The Apple receive changes add an Apple-only direct dependency. Independent review verified that the Linux and Android dependency selections and runtime sources used by those platforms match the tested voice integration. On the combined voice source, 381 JavaScript tests passed with two skipped, and 182 Python tests passed. No physical mobile device was used for these checks.

### Package identities

| Artifact | SHA-256 |
|---|---|
| macOS ARM64 app executable, `ce56621` | `3942c0cbc6560c9b914ae6eb766832d9b975b7f18e7d6d4defe175bd20eec60a` |
| ZIP of signed iOS ARM64 simulator app, `ce56621` | `b9af58c92b12c2ddc3c7ee4a82240e527cd8647ab3af4e465655be8b365d6125` |
| Android two-ABI APK with factory voices, `7c5abf7` | `d02aaddf267cead15bc110e2d4ab1b8369108b0906f7c5d50fbba6d21c1f788d` |
| Linux x86-64 AppImage, `7c5abf7` | `234a34acaf488d3822203434322ed0eb60573810e060b9df159c075f896b44ac` |
| Linux x86-64 Debian package, `7c5abf7` | `ffd7aa9e3086dee888b9ddbd80eb422982d1b571cceca5408f427452b5a57e9f` |

## Performance and open work

Ten alternating Mac comparisons per archive covered 24 full articles per fresh process. The current app served Wikipedia 100 in 111.30 ms versus 112.84 ms with libzim. Climate took 92.07 ms versus 85.36 ms. On Linux the same full-article workloads took 95.30 ms versus 89.51 ms on Wikipedia 100 and 60.19 ms versus 54.46 ms on Climate. The earlier direct-library comparison still favored libzim on the random-article workload on all four platforms. [Complete-article measurements](../benchmarks/2026-11-18-served-articles.md) · [Direct reader timings](../benchmarks/2026-11-18-retained-clusters.md)

Continue with Apple send/readiness and listener ownership, then the remaining platform and dependency boundaries in the inventory. Verify real-device performance before enabling the optional reader setting. The Apple receive timing fixture also showed a regression for contiguous 16 KiB reads. Physical transport throughput remains open.
