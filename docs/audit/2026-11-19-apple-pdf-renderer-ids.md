# Apple PDF renderer callback ownership

The macOS and iOS renderer now passes an integer request ID to native code. A checked Rust registry owns the authority check, result sender and private PDF scratch lease. Cancellation retires authority immediately and requests native cancellation. The lease stays alive until native code closes its PDF context and calls back. Duplicate and late callbacks cannot access a retired Rust request or complete a later request. Native completion returns a byte code, and Rust translates it to the existing user-facing message. An unknown code fails the export.

The source census compares the application and MCP relay at base `6acbc92c691950d96259b18cb511cb50f4f2d869` and the renderer patch. It parses Rust syntax across `src-tauri/src` and `kiwix-mcp/src`, excluding blocks under test-only syntax. It counts 158 blocks in the application and 13 in the relay, down from 161 and 13. The three removed blocks dereferenced an authority context, reconstructed an owned `Box`, and read a native C string. The 41 separate declarations with `unsafe` syntax are unchanged. Each block can contain multiple operations and these source counts cover platform-specific code. The earlier macOS compiler census counted 17,251 target-side sites across the application and its dependencies.

Apple's WebKit and Core Graphics still process the live page and PDF. The two renderer calls into the native bridge still use unsafe Rust. The shared scratch lease retains an app-created path and a capacity permit. Checking directory identity at cleanup is a separate filesystem work item.

## Verification

The signed ARM64 macOS fixture linked the exact native renderer. It exercised four request IDs larger than 32 bits on Apple's main queue. After an owned loopback redirect, WebKit produced a one-page, 12,472-byte PDF, and PDFKit read back the expected text. Invalid dimensions, two cancellation calls for the same request, and a policy denial after navigation each produced one terminal callback and left their own output paths empty. The fixture shut down its temporary server and removed its scratch directory. This exercise covered the native renderer with fixture callbacks.

The macOS release app test binary passed all 26 `native_pdf::` tests, including the four new registry tests. The live bridge test requires the workspace's installed TypeScript package. It passed after the isolated test checkout was given access to those installed dependencies. The signed app test binary had SHA-256 `b2d77b722a707f7530fdc2339e6e0c3bfc582b51855a7d4c9bfdb016f0672d38`.

The app library passed the locked, offline compile checks with `custom-protocol` for ARM64 iOS device and simulator, using Rust 1.95.0 and Xcode 27 RC. iOS PDF export, AppKit printing and the cross-platform libzim performance comparison remain open.

## Independent review

The implementation review verified that all 18 native completion sites map to the 14 expected Rust result codes, that the registry fails closed on exhaustion or revoked authority, and that completion takes ownership of a request only once. The first renderer-only revision had an incorrect macOS import. The reviewer verified the corrected source against base `6acbc92` and manifest SHA-256 `5de19418ea5b8cffd9e02e0ee5f1b8a3cbd8526fffdf5e3dec57cc34b8b5254f`. A second reviewer verified the exact native smoke executable SHA-256 `e28a9122d98bf67a2240d3a9e37968c33f03aff544e128e0507fb69d1176ff83` and checked fixes for server shutdown, per-request output isolation and build input verification.

The wider review found separate macOS print-panel issues. Cancellation can end another operation's attached sheet, and the five-second fallback can retire the Rust owner's PDF before AppKit signals completion. The panel uses the current main or key window, which can differ from the originating window. The authority check precedes PDF preparation and needs revalidation at handoff. The native logger also records the suggested article filename. Track these together with the raw print callback context in [work item B6](../MEMORY-SAFETY-INVENTORY.md#first-party-operating-system-and-native-interfaces).
