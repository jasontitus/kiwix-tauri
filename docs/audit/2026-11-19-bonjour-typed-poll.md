# Borrowed socket polling in the Apple Bonjour worker

The Apple discovery worker now uses the typed `rustix::event::poll` API. The application owns each live DNS-SD reference. Apple owns the underlying socket and lends its descriptor for readiness checks. The worker borrows that descriptor for a single poll, then releases every borrow before processing, retiring or deallocating an operation. A failed native descriptor lookup produces a missing-socket result. [Worker implementation](../../src-tauri/src/localswarm/ios_discovery.rs) · [Apple descriptor contract](https://developer.apple.com/documentation/dnssd/dnsservicerefsockfd(_:))

The readiness list carries operation tags and flags. Before dispatch, the worker rechecks cancellation and looks up the live operation by its non-reused tag. Missing descriptors and error, hangup or invalid flags take precedence over readability. An interrupted poll supplies no ready descriptors, so the worker can still handle missing sockets and run expiry checks. The poll timeout remains 50 ms. The app-owned raw poll call and unchecked descriptor-count conversion were removed. The remaining unsafe operations in this path retrieve the Apple descriptor and borrow it while its reference is live. The pinned Rustix backend owns the native polling call.

The Apple targets enable the `event` feature in the existing locked Rustix 1.1.6 crate. The resolved feature selections for Linux x86-64 and Android ARM64 and ARMv7 match the baseline. Cargo.lock has the same bytes. [Dependency feature receipts](../benchmarks/2026-11-19-bonjour-typed-poll.json)

## Behavior

The Mac worker fixture passed 18 tests. The new socket-pair regression interleaves missing, idle and readable sockets so a readiness flag must reach the correct tag. It reuses the output vector on a second call and verifies that old flags disappear.

Separate baseline and candidate release fixtures compiled the exact corresponding discovery source. Real system Bonjour passed on ARM64 macOS and an owned ARM64 iOS 26.5 simulator. Trials verified the escaped service name and endpoint, withdrawal, replacement isolation after use of a stale token, and both workers shutting down. The fixture's socket used a private IPv4 address. Simulator removal was confirmed. The worker also compiled for ARM64 iOS device. [Fixture and ownership protocol](2026-11-19-bonjour-owned-names.md#discovery-and-withdrawal)

The Mac application test executable passed all 14 Apple discovery-worker tests, source revocation, native route verification and concurrent QUIC requests. The release verification build used 16 code-generation units for the app crate. Its owned temporary profiles were removed. The complete app library passed locked iOS ARM64 device and simulator compile checks with `custom-protocol`. The live simulator evidence comes from the discovery fixture. [App build and test receipts](../benchmarks/2026-11-19-bonjour-typed-poll.json)

## Polling cost

The Mac microbenchmark used real Unix socket pairs. Only the last socket was readable, so the kernel could return immediately. The fixture checked that both paths returned the expected flags for every socket before timing. Baseline descriptor vectors were reused. The typed helper retains its result vector and allocates a temporary descriptor array for each call. The timer includes preparing and polling the descriptors and collecting the results. It excludes Apple descriptor lookup, callback dispatch and daemon activity. Each width ran eight alternating pairs of 20,000 calls after warmup.

| Sockets | Baseline median ns/call | Typed median ns/call | Median paired increase |
| --- | ---: | ---: | ---: |
| 1 | 817.430 | 872.057 | 6.47% |
| 8 | 2,682.889 | 2,758.024 | 2.71% |
| 64 | 15,886.713 | 16,179.480 | 1.51% |

The difference between the medians was about 55 ns at one socket and 293 ns at 64. The two Mac lifecycle series had median paired ratios of 1.163 and 1.033, with six pairs per series. The simulator pairs had ratios of 0.978 and 1.903. Their timer includes registration, discovery, withdrawal and worker shutdown. The runs shared the Mac with background builds. The microbenchmark isolates polling cost, and these small lifecycle samples give limited evidence on how the change affects real discovery time. [Raw trials and build identities](../benchmarks/2026-11-19-bonjour-typed-poll.json)

## Review and remaining boundaries

Independent source review checked descriptor ownership, the Rustix Darwin backend, missing sockets, interruption, error precedence, callback lifetimes, cancellation and feature scope. Separate review verified the Mac and iOS test binaries and required the microbenchmark to confirm the expected ready socket before timing. DNS-SD reference management and borrowed callback payloads remain in scope. Archive performance against libzim and app-wide memory safety require separate verification. [Whole-app inventory](../MEMORY-SAFETY-INVENTORY.md)
