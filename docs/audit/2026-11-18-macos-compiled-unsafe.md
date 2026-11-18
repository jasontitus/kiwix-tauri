# Unsafe Rust compiled for the Apple silicon desktop app

The release build at `eaffd58cca7ba67430c4bbce222c43d20a8fe847` emitted **125 unsafe-code diagnostics at 125 distinct sites in the application crate**. The compiler reported **17,850 distinct target-side sites** across 254 of the 426 packages compiled for the product. These include generated system bindings, dependency internals and code the linker can discard. Establishing which sites handle an archive, a network request or a native callback requires tracing the caller. [All package counts and app source locations](2026-11-18-macos-compiled-unsafe.json)

| Compiler role | Fresh units | Packages | Packages with warnings | Raw warnings | Distinct sites |
|---|---:|---:|---:|---:|---:|
| Apple silicon product target | 427 | 426 | 254 | 22,909 | 17,850 |
| Host build scripts, procedural macros and their dependencies | 369 | 323 | 134 | 6,966 | 5,106 |
| Combined, with shared packages and sites counted once | 796 | 531 | 290 | 29,875 | 19,135 |

The MIT archive reader, its vendored Avarok Zstandard codec and the GPL search crate each produced zero unsafe-code diagnostics in their compiled target units. The selected XZ decoder also produced zero target warnings. Its separate host library compilation produced seven. HTML handling reaches tendril 0.5.1, with 120 sites in its target library. The app-wide inventory also includes native compression and cryptographic build inputs selected elsewhere in the graph. [Reader-specific dependency analysis](../MEMORY-SAFETY-INVENTORY.md)

## Replacement work in the app crate

Paths below are relative to `src-tauri/src`. The receipt includes file, line, column, compiler message and compilation role for each app site at this revision. Counts refer to the Rust side of each boundary.

| Files | Sites | Behavior to preserve when replacing the unsafe Rust |
|---|---:|---|
| `localswarm/apple_transport.rs`, `localswarm/ios_discovery.rs`, `localswarm/library_route.rs` | 62 | Move nearby connection buffers, discovery callbacks, interface lookup and connection ownership behind checked APIs. Exercise cancellation, reconnection and late callbacks. The discovery file also compiles on macOS. |
| `native_pdf/apple.rs`, `notifications.rs`, `permissions.rs`, `share.rs` | 36 | Replace raw callback contexts, native strings and cancellation handles for PDF, printing, notifications, location and sharing. Verify completion and cancellation release each retained context once. |
| `windows.rs`, `menus.rs`, `power_activity.rs` | 13 | Replace Apple window, WebView, menu and process-activity handoffs with typed platform bindings. Preserve UI-thread ownership, activity end and snapshot lifetimes. Here `windows.rs` manages app windows on macOS. |
| `publication.rs`, `library_files.rs`, `localswarm/storage.rs`, `maps/partials.rs`, `torrent/publication.rs` | 6 | Consolidate descriptor-relative exclusive rename and replacement behind a checked filesystem API. Preserve no-clobber behavior, identity checks and recovery from interrupted publication. |
| `storage.rs`, `sandbox.rs`, `instance.rs` | 8 | Replace raw filesystem path/type queries, account directory lookup, security-scoped bookmarks and Unix peer identity calls with typed equivalents. Exercise revoked grants and peer rejection. |

The app crate had no host-side unsafe-code diagnostic. Four maintained vendor packages have separate rows. Wry has 105 distinct target sites, tauri-runtime-wry has 13, librqbit-dualstack-sockets has two, and mdns-sd has zero. Their code remains part of the review surface. Plugins compiled by Cargo appear under their own package identities. The separately packaged MCP relay has its own build and requires its own inventory.

## How to reproduce the evidence

Use the measured application revision and the audit scripts together. A checkout of the documentation commit includes the tools. To reproduce the exact application revision, copy the three audit scripts into `scripts/compiled-unsafe/` in a separate checkout of the revision above and retain the wrapper's executable mode. Prepare the locked Cargo registry and private Git dependencies, build the desktop frontend and MCP sidecar, and provision the sherpa-onnx 2.0.1 macOS ARM64 static libraries. The committed JSON identifies the host, exact Rust toolchain and Cargo.lock checksum. [Packaging prerequisites](../PACKAGING.md)

```sh
npm ci
npm run build
npm run build:mcp
export SHERPA_ONNX_LIB_DIR=/absolute/path/to/the/prepared/static/lib
python3 scripts/compiled-unsafe/capture.py /tmp/kiwix-unsafe-fresh
python3 scripts/compiled-unsafe/summarize.py /tmp/kiwix-unsafe-fresh /tmp/kiwix-unsafe.json
```

The capture requires a new output directory and clean application sources. It builds the locked release desktop binary offline with the custom-protocol feature and an explicit Apple silicon target. The rustc wrapper forces the unsafe-code warning on host and target compilations. The summary requires successful compiler exits, a successful Cargo build, a recorded executable, and a one-to-one match of fresh Cargo artifacts to compiler units by manifest, crate, role and features. It reconciles the full multiset of unsafe diagnostics and their primary spans with Cargo's independent event stream. Eight auxiliary compiler probes from build scripts completed without unsafe warnings and produced no Cargo artifacts.

A site is the compiler message and its primary source range within a package identity. Multiple expansions can emit the same range, and the same source can compile for both roles. Generated Rust files use an output-relative name and content digest. The unit counts measure compilation before linker dead-code elimination. The receipt fingerprints the locally built instrumented executable. Two fresh builds of this revision reproduced all package rows, source-site counts, link requests and toolchain fields. Their locally built executable hashes differed. The receipt records the second build. Signing, bundle contents, frontend inputs and build paths can change executable bytes.

The Rust lint sees selected Rust syntax and some generated Rust bindings. It cannot inspect C, C++, Objective-C++, assembly, precompiled Rust standard libraries or OS implementations. Compiler handling of external macros and generated code can affect which constructs produce a warning. Cargo build-script link requests in the receipt identify inputs including bundled SQLite, native speech/model libraries, crypto libraries and the app's Apple bridges. Link requests can include unused libraries and omit inputs added through other mechanisms. The capture covers one desktop target and profile. Mobile, Windows, Linux, tests, optional features, packaged binaries and their final native linkage require separate checks. Reported sites identify code for review and make no finding of a memory-safety defect.
