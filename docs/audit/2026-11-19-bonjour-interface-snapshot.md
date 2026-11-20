# Apple Bonjour interface selection

Apple Nearby now uses a typed snapshot of the system's interface names when it needs to choose between resolved sources. It prefers the first candidate whose exact interface index maps to a name beginning with `awdl` or `llw`. It uses the first candidate when the query fails, when no candidate matches or when direct mode is disabled. A single candidate needs no interface query. The name controls source preference. Route and peer authority come from the existing connection path. [Discovery worker](../../src-tauri/src/localswarm/ios_discovery.rs) · [Apple interface snapshot contract](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man3/if_nameindex.3.html)

Compared with `bf132b6f4bf2d365608d103aef4c1648633ec8ed`, this removes one production Rust `unsafe` block from the app. The census is 118 app blocks and 13 MCP relay blocks, or 131 total, plus 35 separate declarations. The pinned nix library holds the system allocation and frees it when the snapshot goes out of scope. The native wrapper and the DNS-SD reference and callback boundaries remain in scope.

## Runtime checks

The ARM64 macOS and iOS 26.5 Simulator release fixtures compiled the exact production worker for each revision, along with discovery entry points and publisher validation. Two temporary workers used system Bonjour to resolve an escaped dotted name, observe withdrawal, restart the reader, resolve a replacement registration at the same name and confirm that a stale withdrawal could not remove it. Both revisions passed four runs each on macOS and two runs each on the simulator. The runner checked worker shutdown and confirmed deletion of the owned simulator. The fixture verifies observed generations and endpoint metadata. Actual AWDL packet routing and physical iPhone coverage remain open.

The standalone macOS worker fixture passed 18 tests. The signed, unoptimized macOS app test executable passed 136 LocalSwarm tests with 14 ignored. A focused run covered 14 discovery tests, and separate invocations passed the native revocation, interface-name ownership and concurrent-request cases. The runs overlap. The binary's SHA-256 was `aa7c4703481cbacb10a330c5efc5f13e6d8eea617bd469d77ea0b16576a808f8`. Both test profile directories were empty after their runs.

The app library passed locked, offline ARM64 iOS device and simulator compile checks with `custom-protocol` using Rust 1.95.0 and Xcode 27 RC. The initial attempt was blocked by the nested Swift package sandbox. The same source and configuration passed when Xcode could start its build sandbox. The iOS app checks cover compilation. Runtime results above come from the standalone fixture.

## Performance

The median paired candidate-to-baseline ratio for the full discovery fixture was 0.994 across four alternating macOS pairs and 1.100 across two iOS Simulator pairs. The timings include system registration, withdrawal and worker shutdown. Individual runs ranged from 2.40 to 3.75 seconds. These small, variable samples ran alongside compilation activity and leave the end-to-end performance change inconclusive.

An ARM64 macOS release probe compiled the old and new selection expressions. It checked a real `awdl0` interface, loopback, missing indices, the first-candidate fallback and direct mode disabled. The table gives medians of eight alternating pairs of 4,000 calls per case. It measures only candidate selection and any system interface query. The 17-candidate case uses 16 synthetic missing indices followed by the real AWDL interface.

| Selector case | Baseline, µs per call | Snapshot, µs per call |
|---|---:|---:|
| Direct mode disabled | 0.002 | 0.002 |
| One ordinary candidate | 152.519 | 0.002 |
| Two candidates and no direct interface | 302.886 | 152.202 |
| Two candidates with AWDL second | 304.832 | 152.248 |
| Seventeen candidates with AWDL last | 5,714.799 | 466.202 |

## Independent review

Review caught a mismatch between the pinned nix per-index wrapper's error check and Darwin's null return. The revised code uses the snapshot API and handles a failed snapshot with the existing first-candidate fallback. Independent source review checked ownership, exact index matching, fallback order and the distinction between source preference and route authority. The reviewer compared 43 interfaces with direct system lookups, including two peer-to-peer names. Separate review checked the exact fixture and application inputs, binary identities, sample calculations, runtime results and simulator cleanup. [Whole-app inventory](../MEMORY-SAFETY-INVENTORY.md)
