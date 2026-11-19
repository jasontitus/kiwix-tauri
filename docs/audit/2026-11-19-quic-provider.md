# RustCrypto QUIC provider experiment

Kiwix Tauri's Rust Nearby transport uses Quinn 0.11.12 with Rustls 0.23.46 and ring. Quinn's Retry integrity, stateless-reset key and validation-token key also have native crypto paths. The experiment replaces those paths in a standalone binary. The app's QUIC, Apple networking, HTTPS and torrent TLS configuration is unchanged. [Application transport](../../src-tauri/src/localswarm/network.rs) · [Probe source and reproduction](../../scripts/bench-quic-provider/README.md)

The candidate uses Quinn's Rustls adapter with an injected provider. The published OxiQUIC 0.2.1 dependency graph failed to build with the resolved RSA release. The probe uses `oxicrypto-cipher` 0.3.0 for OxiQUIC's AES helpers and `rustls-pure` 0.15.4 for key exchange, handshake signature verification and signing. The probe's certificate verifier compares the server leaf to the pinned bytes and rejects extra certificates. OxiQUIC supplies the QUIC cipher suites. The patched adapter uses RustCrypto AES-128-GCM for Retry in the candidate-only build. Quinn retains its existing Retry backend when ring or AWS-LC is enabled. The probe supplies HMAC-SHA256 reset keys and HKDF-SHA256 plus AES-256-GCM for validation tokens. [Dependency patches and license attribution](../../scripts/bench-quic-provider/README.md)

## Runtime results

Separate release binaries ran on Apple Silicon macOS and x86-64 Linux, with Rust 1.99.1 and 1.100.0 respectively. Each host passed 24 QUIC v1 cases across all client/server pairings, three TLS 1.3 cipher suites and both Retry modes. Forced Retry verified address validation before server acceptance. Each successful case transferred and checked all 65,537 bytes. All four wrong-certificate cases failed as expected per host. Those cases used AES-128 with Retry disabled. The test used a pinned P-256 server certificate, X25519 key exchange and the app's ALPN. Authentication covered the server. Native-peer interoperability excludes the Apple endpoint, other QUIC versions, emitted stateless resets and token validation across shared endpoint secrets. [Case records and binary hashes](../benchmarks/2026-11-19-quic-provider.json)

Each platform ran six alternating same-provider pairs per suite, for 36 trials. The client timed connection through the final read of 8 MiB. The driver checked every byte. The table gives medians for each binary and the median of the candidate/reference ratios within each pair. Linux ran on an Intel Core i7-8809G in a temporary container with external networking disabled. macOS restricted traffic to loopback.

| Host and suite | Reference median | Candidate median | Median paired ratio |
| --- | ---: | ---: | ---: |
| macOS ARM64, AES-128-GCM | 76.145 ms | 76.472 ms | 1.028 |
| macOS ARM64, AES-256-GCM | 75.107 ms | 77.169 ms | 1.026 |
| macOS ARM64, ChaCha20-Poly1305 | 84.771 ms | 100.741 ms | 1.203 |
| Linux x86-64, AES-128-GCM | 34.036 ms | 44.599 ms | 1.337 |
| Linux x86-64, AES-256-GCM | 45.694 ms | 44.440 ms | 1.010 |
| Linux x86-64, ChaCha20-Poly1305 | 44.266 ms | 61.008 ms | 1.210 |

Connection-only medians were below 3 ms in these trials. The measurements cover local synthetic transport. App startup, archive reads, libzim parity, real Wi-Fi and mobile-device performance require separate tests. [Earlier archive reader comparisons against libzim](../benchmarks/2026-11-18-portable-decoder.md)

## Memory-safety and platform limits

The candidate-only normal and build dependency graphs on both hosts exclude ring, AWS-LC and OpenSSL. The probe's checked Rust token module forbids unsafe code. The wider graph still contains unsafe Rust. On ARM64 macOS, `--force-warn unsafe_code` produced 1,961 diagnostics in default dispatch and 1,870 with selected AES, ChaCha20, Polyval, Poly1305, SHA-2 and Curve25519 software paths enabled. Diagnostics include build-time crates and count compiler reports. The remaining crates include AES, crypto-bigint, inout and hybrid-array, along with Quinn, its UDP adapter and general-purpose dependencies. Software dispatch leaves AES with 27 diagnostics in this configuration. An unsafe declaration or block requires source and caller analysis to determine its role and reachability. [Per-package diagnostics and build configuration](../benchmarks/2026-11-19-quic-provider.json)

Locked `cargo check` passed for the candidate on ARM64 iOS, ARM64 iOS Simulator, ARM64 Android and ARMv7 Android with Rust 1.95.0. These are standalone compile checks. No app package was built or installed for this experiment. The open work includes a dependency audit of packet processing, compatibility with real app endpoints, and transport performance on mobile. App-wide HTTP trust and verification use separate paths. [Networking worklist](../MEMORY-SAFETY-INVENTORY.md)

## Independent review

The reviewed application base was `b22c6299eb808ae21baba7c67431da31cb6e5fa8`. Independent review found a boundary problem in the candidate's ChaCha20 header protection. The fix uses one block from the existing RustCrypto primitive. Six reference comparisons include both endpoints of the valid block-counter range. The subsequent review verified that path, the Retry and token contracts, and the separate binary results. It found a second issue in mixed Cargo features. The final patch retains Quinn's existing native Retry precedence. An independent follow-up checked the fix and the mixed ring build. The mixed ring binary then passed the same 24 wire cases and four wrong-certificate tests against the reference on macOS. AWS-LC mixed precedence was verified from source. Both ordinary candidate-only and reference-only execution paths retain their previous backend selection. [Reviewed manifests and results](../benchmarks/2026-11-19-quic-provider.json)
