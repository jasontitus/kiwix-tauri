# Checked ownership for Apple Bonjour callback payloads

The Apple discovery worker now uses the standard Rust C-string view for successful browse and resolve callbacks. The app removed its handwritten pointer scan and raw string slice construction. It validates text as UTF-8 and copies it into an owned event. The 256-byte browse allowance and 1,009-byte resolve allowance include the NUL. These limits apply to accepted text. The initial C-string view relies on the trusted SDK supplying a readable string through its NUL for the lifetime of the callback. Apple's published client stub decodes these fields into NUL-terminated arrays before calling the app. [Application callback adapter](../../src-tauri/src/localswarm/ios_discovery.rs) · [Apple client stub](https://github.com/apple-oss-distributions/mDNSResponder/blob/main/mDNSShared/dnssd_clientstub.c) · [Apple string decoder](https://github.com/apple-oss-distributions/mDNSResponder/blob/main/mDNSShared/dnssd_ipc.c)

The active operation cookie and queue limits are checked before reading a payload. A failed SDK reply enqueues its error without inspecting the undefined payload. A successful resolve checks the TXT length against 2,048 bytes, accepts an empty record with a null pointer and rejects a nonempty record with a null pointer. The remaining raw TXT view relies on the SDK's length for initialized bytes and copies them before returning. The three callbacks that use only scalar fields and cookie values are safe Rust functions with the same C calling convention. [Apple resolve callback contract](https://developer.apple.com/documentation/dnssd/dnsserviceresolvereply)

## Runtime verification

Tests checked the text acceptance limit, invalid UTF-8, empty and missing TXT, port byte order, and error callbacks with undefined payloads. One test overwrote the input strings and TXT buffer after the callback to verify that the queued event retained its own bytes. All 17 tests passed in the macOS worker fixture.

The release fixture compiled the exact production discovery source in both lanes. Separate build directories and depfiles tie each executable to its source. Both lanes passed live system Bonjour on ARM64 macOS and an owned ARM64 iOS 26.5 simulator. The same checked test sequence from the owned-name checkpoint verified each trial's escaped dotted name, complete endpoint, two publication tokens, final withdrawal and both worker shutdowns. The fixture socket was bound to a private IPv4 address. The simulator was removed after the trials, and the worker compiled for ARM64 iOS device. [Fixture design](2026-11-19-bonjour-owned-names.md#discovery-and-withdrawal)

The complete macOS app test binary passed all 13 Apple discovery-worker tests, the live source-revocation test, and the native route and concurrent QUIC regressions. The release verification build used 16 code-generation units for the app crate. Its owned test profiles were removed. The app library also passed locked compile checks for iOS ARM64 device and simulator with `custom-protocol` enabled. The simulator runtime evidence comes from the discovery fixture. [App build and test receipts](../benchmarks/2026-11-19-bonjour-callback-payloads.json)

## Callback processing cost

The Mac release replay delivered ordinary C callback data to the baseline and candidate. It compared every accepted event, including all copied TXT bytes. Each sample covered 49,152 events and included C dispatch, decoding, allocation, queue handling and output verification. The 96 measured samples followed a warm-up pair per scenario. The paired ratio compares candidate to baseline within each pair before taking the median.

| Event | Callbacks per replay batch | Baseline ns/event | Candidate ns/event | Median paired ratio |
| --- | ---: | ---: | ---: | ---: |
| Browse | 1 | 423.453 | 424.016 | 0.991 |
| Browse | 32 | 398.190 | 400.032 | 0.996 |
| Resolve, 32-byte TXT | 1 | 488.193 | 459.638 | 0.951 |
| Resolve, 32-byte TXT | 32 | 429.384 | 401.315 | 0.946 |
| Resolve, 2,048-byte TXT | 1 | 782.772 | 745.515 | 0.940 |
| Resolve, 2,048-byte TXT | 32 | 731.586 | 708.534 | 0.969 |

The replay describes local callback processing. Real discovery timing also includes the system daemon, registration, withdrawal and worker shutdown. Six Mac live pairs had baseline and candidate elapsed medians of 3,066.114 ms and 2,524.784 ms, with a median paired ratio of 0.818. The two simulator ratios were 1.343 and 0.883. These small samples include background build work and OS discovery time. They establish successful discovery and cleanup and give limited evidence about end-to-end latency. [Raw measurements and binary identities](../benchmarks/2026-11-19-bonjour-callback-payloads.json)

## Review and remaining boundaries

Independent source review checked the callback contract, data and queue ownership, port byte order, and early rejection. A separate review checked that the runtime and replay executables used the exact source in separate build directories. The SDK-owned C-string and TXT borrows, DNS-SD reference lifetime and native polling remain in scope. This checkpoint concerns Apple discovery. Whole-app memory safety and archive performance against libzim require separate verification. [Whole-app inventory](../MEMORY-SAFETY-INVENTORY.md)
