# Memory-safety checkpoint

The app still runs native code on untrusted content. A fresh Mac release build counted 102 unsafe Rust sites in the app crate and 17,271 across 420 target-side packages. Moving the Apple send, readiness and accept callbacks to owned blocks removed six app sites from the previous release build. The acceptance target trusts the OS and standard runtime and includes application code and its dependencies. The main remaining paths include native speech inference, rendering and media, Nearby connections and discovery, platform document handoffs and bundled database and networking libraries. [Boundary inventory](../MEMORY-SAFETY-INVENTORY.md)

## Changes saved privately

The [voice and receive integration](https://github.com/jasontitus/zimru-xapian/commit/f1a6c0890a3348982a4cb3588d85ef86f54296af) combines the Android factory-voice producer, reviewed voice installer and Apple Nearby receive change. The final three-file Linux test correction exercises the production profile layout. The first combined Apple packages used `bec2658`. The [subsequent Apple stream fix](https://github.com/jasontitus/zimru-xapian/commit/ce566218b035613cfd23ee34ded01ee6083bb113) delivers a final payload and preserves clean end-of-stream on the following read.

- The voice installer authenticates downloaded and bundled inputs, uses safe Rust bzip2 decompression and journals installation and removal. It preserves an existing installation when recovery cannot establish which model files belong to the app. Android treats an inconclusive factory-asset lookup as retryable. Native sherpa-onnx and ONNX Runtime still process text and models. [Installer checks and decoder timings](2026-11-18-voice-pack-decoder.md)
- Apple Nearby now binds each incoming read to a single-use ID, validates its size and retires the ID on cancellation. Native code returns a typed dispatch object and the request ID in place of a raw byte pointer and heap-owned Rust callback context. The [next Apple revision](https://github.com/jasontitus/zimru-xapian/commit/09e57a9ec86c3fd828f19b4aa638f490c42606ae) also replaces the raw callback contexts for sends, connection and listener readiness, and incoming connections. One dispatch-owned copy carries each outgoing piece. Native buffer traversal, block support, opaque connection and listener handles, and DNS-SD remain in scope. [Apple receive tests and timings](../benchmarks/2026-11-18-apple-nearby-reads.md)
- The app still pins the MIT archive reader at `ff8b0ab` and GPL search at `380d84d`. These changes affect the GPL app. The optional MIT reader performance switch remains deferred. [Reader performance follow-up](../MEMORY-SAFETY-INVENTORY.md#performance-follow-up)

## Verification

| Platform | Scope and outcome |
|---|---|
| macOS ARM64 | The ad-hoc signed app at `09e57a9` contained both expected executables and all 44 license resources. It upgraded a real Climate spelling index, reopened it with 4,204 terms, and opened and served the Ray Charles article found by full-text search. The final optimized test executable passed 1,254 tests with 25 ignored by default. Separate runs passed four live native network tests and three genuine voice-pack installation, recovery and removal tests. |
| Linux x86-64 | The integrated voice binary, Debian package and AppImage at `7c5abf7` served checked article and media bytes. The GTK reader and the cold spelling-index workflow passed. With the test-only correction, 1,240 optimized tests passed with 16 ignored under each of umasks 002 and 022. All 39 focused voice tests passed under each mask, including three authentic-pack cases. Four launches through the public headless interface confirmed fresh profiles used mode 0700 and existing group-writable profiles kept their permissions and unrelated files kept their contents. |
| iOS ARM64 simulator | The signed `09e57a9` package passed 44 hosted tests and its reader UI test. It restored the saved Climate article before user navigation. Settings offered the standard voice download. No voice was downloaded or played. On the earlier receive revision `ce56621`, the app transport transferred all 600,123 bytes and reported clean EOF over real loopback TCP and QUIC in each of four simulator runs. At `09e57a9`, outgoing QUIC transfers passed against an independent peer. Both versions of the inbound listener reported ready and timed out before the Rust accept callback ran. Live incoming connections on iOS remain unverified. |
| Android | The two-ABI offline appliance APK at `7c5abf7` passed signature and content checks for all 360 voice-pack files, 44 license resources and ten native images. With airplane mode enabled, Wi-Fi and mobile data disabled, and no default network on the ARM64 API 35 emulator, Settings showed the bundled model ready. Removal persisted across a cold launch. After an explicit offline reinstall, the model remained installed through a second cold launch. ARMv7 coverage was build and package inspection. |

The Apple send and receive changes use Apple-only direct dependencies. Independent review verified that the Linux and Android dependency selections and runtime sources used by those platforms match the tested voice integration. On the combined voice source, 381 JavaScript tests passed with two skipped, and 182 Python tests passed. No physical mobile device was used for these checks.

### Package identities

| Artifact | SHA-256 |
|---|---|
| macOS ARM64 app executable, `09e57a9` | `ee82715ae71e2d0cc7b9f3cb999d5ff705185c2c4da394f6f0de597bdade3e47` |
| ZIP of signed iOS ARM64 simulator app, `09e57a9` | `64eddcae8fc7c0b8857673ea2eca38aace7a8382fcce7ec7913b1591060e805c` |
| Android two-ABI APK with factory voices, `7c5abf7` | `d02aaddf267cead15bc110e2d4ab1b8369108b0906f7c5d50fbba6d21c1f788d` |
| Linux x86-64 AppImage, `7c5abf7` | `234a34acaf488d3822203434322ed0eb60573810e060b9df159c075f896b44ac` |
| Linux x86-64 Debian package, `7c5abf7` | `ffd7aa9e3086dee888b9ddbd80eb422982d1b571cceca5408f427452b5a57e9f` |

## Performance and open work

Ten alternating Mac comparisons per archive covered 24 full articles per fresh process. In the latest Mac run the signed app served Wikipedia 100 in 68.04 ms versus 64.01 ms with libzim. Climate took 43.49 ms versus 46.37 ms. The earlier Mac run favored the opposite build on each archive. On Linux the same full-article workloads took 95.30 ms versus 89.51 ms on Wikipedia 100 and 60.19 ms versus 54.46 ms on Climate. The earlier direct-library comparison still favored libzim on the random-article workload on all four platforms. [Complete-article measurements](../benchmarks/2026-11-18-served-articles.md) · [Direct reader timings](../benchmarks/2026-11-18-retained-clusters.md)

Continue with opaque Apple connection and listener handles, their thread-safety declarations and DNS-SD callback ownership, then the remaining platform and dependency boundaries in the inventory. Verify real-device performance before enabling the optional reader setting. The Apple receive timing fixture showed a regression for contiguous 16 KiB reads. In the Mac send fixture, eight outstanding 16 KiB pieces added 0.51 µs per piece. The iOS simulator also completed 48 local QUIC sender trials against an independent peer. The results vary with message size and timing method. [Send measurements](../benchmarks/2026-11-18-apple-nearby-sends.md) Physical transport throughput remains open.
