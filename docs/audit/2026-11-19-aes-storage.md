# Typed key storage in the Rust QUIC prototype

The published AES 0.9.4 wrapper uses a union for each key representation. Software-only builds still read that union through unsafe code. The prototype patch uses tagged storage for software and accelerated keys. CPU detection still gates construction of the hardware variant, and x86 retains the separate VAES checks. AES rounds, key expansion, and the public encrypt, decrypt and conversion APIs come from the existing implementation. With `zeroize` enabled, drop clears the selected backend's active round-key arrays through the typed `Zeroize` API. This covers the stored schedule. Copies and temporaries may have their own lifetime. [Patch, license and reproduction](../../scripts/bench-quic-provider/README.md#compare-typed-aes-storage)

The software-only configurations enforce `forbid(unsafe_code)` in the AES crate. On ARM64 macOS, a locked all-feature library check counted 27 AES `unsafe_code` diagnostics in the published software build and zero in the patched build. Default dispatch went from 89 to 44. Those numbers count compiler reports about the AES crate in this feature set. The hardware paths still call architecture intrinsics, and dependencies such as `zeroize`, `cipher` and `inout` retain unsafe internals. This patch is limited to the standalone QUIC prototype.

## Verification

The final AES source passed 19 release tests per configuration on macOS with default dispatch, forced software, and compact software. Linux passed the same 19 tests with default dispatch and forced software. The suite includes published vectors for all three key sizes, new known-answer tests for borrowed and owned encrypt-only conversions, and a test that checks all words of the software schedules after erasure. Both macOS dispatch choices also compiled with the optional erasure feature off. Seven locked AES library compile checks covered device and simulator iOS ARM64, Android ARM64 with default and forced software dispatch, and Android ARMv7 with its software-only default. Those checks used Rust 1.95.0. Mobile runtime and application packaging were outside this experiment. [Source hashes and runtime records](../benchmarks/2026-11-19-aes-storage.json)

On each host, the two release binaries used the same RustCrypto QUIC provider, Rustls, and Quinn source. The comparison changed only the AES package source in the lockfile. Every run passed 24 QUIC v1 interoperability cases at three cipher suites and both Retry modes, plus four wrong-certificate cases. There were six runs in total, including two complete Linux matrices. Each run timed six same-provider pairs per suite, from connect through the last read of an 8 MiB stream. The driver compared every byte. The Linux container disabled external networking. [Raw trials, binary hashes and receipts](../benchmarks/2026-11-19-aes-storage.json)

## Performance

The values below are the median of the patched/published timing ratios within pairs. All three suites use AES during QUIC Initial processing, including sessions that negotiate ChaCha20 for application traffic.

| Host and AES dispatch | AES-128-GCM | AES-256-GCM | ChaCha20-Poly1305 |
| --- | ---: | ---: | ---: |
| macOS ARM64, default | 1.009 | 0.988 | 0.995 |
| macOS ARM64, software | 1.005 | 0.994 | 0.990 |
| Linux x86-64, default, first | 1.157 | 1.168 | 1.069 |
| Linux x86-64, default, repeat | 1.150 | 0.983 | 1.068 |
| Linux x86-64, software, first | 1.017 | 1.001 | 0.906 |
| Linux x86-64, software, repeat | 1.046 | 0.965 | 1.179 |

The repeated Linux default AES-128 series had a similar paired slowdown. Its candidate medians were 50.539 ms and 44.095 ms across the two runs. AES-256 and software ChaCha20 varied between runs. The single-core Linux AES test confirmed hardware selection in default builds. With one expanded key reused, median paired AES-128 and AES-256 encryption times were within 0.3%. Constructing, using and dropping a fresh key took about 14% and 22% longer, respectively, with candidate medians of 59.691 ns and 82.922 ns per operation. Dropping a vector of 40,000 expanded keys, including freeing its allocation, also took longer. The typed default-dispatch values are 16 bytes larger per key for both sizes tested. These results establish a cost for short-lived keys on this host. The measurements do not isolate how much it contributes to the QUIC transfers. The provider currently expands an AES header-protection key for each mask. Caching an encrypt-only schedule for that key is a concrete follow-up to measure. [Operation samples and benchmark source](../benchmarks/2026-11-19-aes-storage.json) · [Direct benchmark](../../scripts/bench-quic-provider/aes-storage-micro.rs)

## Independent review

Review covered the tag-to-CPU-feature invariant, borrowed and owned key conversions, the software proof and the lifetime of the active key schedules. The first reviewed revision still erased the hardware backend through a flat-type unsafe call. The reviewer identified the existing typed SIMD erasure API. The final revision uses that API for the encryption and decryption arrays on both x86 and AArch64. A second independent review checked all seven final source and build-input hashes and confirmed the erasure feature gate. The shipped provider, real peers, and reader speed against libzim remain separate verification work. [Whole-app inventory](../MEMORY-SAFETY-INVENTORY.md)
