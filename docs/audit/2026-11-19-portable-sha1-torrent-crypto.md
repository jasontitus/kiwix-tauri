# Portable SHA-1 throughput and torrent crypto migration

The app's vendored RustCrypto SHA-1 now requests inlining of six portable compressor helpers. The rounds, message schedule, data accesses and hardware dispatch retain their existing implementations. The app uses this crate when checking torrent web seeds. The released torrent engine still uses AWS-LC in this checkpoint. [Web seed verification](../../src-tauri/src/torrent/web_seed.rs)

The independently reviewed source change is one file at `6b865ee3e6f5d460cdef1858cdcebc46c7651582`, against `8276cc471ed831afae17e6f3e065eb2753bee63d`. Its SHA-256 is `9461dba28e3b246ee09af4d5de6e08fe5599e1f5458f2fb393e406e16e338d8e`. The reviewer checked all six annotations, original and candidate benchmark inputs and their lockfiles. The existing verifier passed ten checks on macOS with software SHA-1 forced, including the separate SHA-256 4 GiB case. All seven streaming and compressor tests passed on the Linux host without SHA instructions. The standalone SHA-1 consumer checked successfully for ARM64 iOS and iOS Simulator and ARM64 and ARMv7 Android with the software path selected.

## Torrent piece hashing

The measurements use two Wikipedia archives already in memory. The harness hashes 2 MiB pieces with 64 KiB updates and verifies each digest against Python, including the shorter final piece. Each measured process completes three warmed passes after reading its input, with 12 alternating pairs per archive and candidate. The reference is the published librqbit SHA wrapper 9.1.0 with AWS-LC 1.19.0. The candidate wrapper uses the app's modified RustCrypto packages. These are hashing measurements on the sampled CPUs. [Raw samples](../benchmarks/2026-11-19-portable-sha1-torrent-crypto.json)

| Linux SHA-1, median elapsed per archive | AWS-LC reference | Earlier Rust | Inlined Rust |
| --- | ---: | ---: | ---: |
| Climate, 8.98 MB | 7.91–8.03 ms | 33.47 ms | 13.91 ms |
| Wikipedia 100, 36.52 MB | 31.58–31.76 ms | 136.73 ms | 56.66 ms |

The earlier and inlined Rust measurements each paired with the same AWS-LC binary in separate runs. The median of paired Rust-to-AWS elapsed ratios moved from 4.16 to 1.77 on Climate and 4.32 to 1.79 on Wikipedia 100. This x86-64 host exposes AVX2 and BMI2 and lacks SHA instructions. Two alternate checked schedules were slower than inlining. Rebuilding the inlined variant for that specific CPU improved the paired ratios to 1.52 and 1.55. That host-specific build was an experiment.

On Apple ARM64 the original candidate's paired SHA-1 elapsed ratios were 1.057 for Climate and 1.057 for Wikipedia 100, with candidate medians of 3.99 and 16.37 ms. The separate SHA-256 wrapper check measured paired ratios of 1.118 and 1.126. The released torrent engine's checked call sites use SHA-1. The SHA-256 test covers the wrapper interface.

## AWS-LC removal prototype

A separate source snapshot `8c1127da6bcaad72d2cb9171a9fc37c31b90924b5bfcac0f67c34eb7ff6676dd` retains the published librqbit and core 9.1.0, changes the small Apache-licensed SHA wrapper and explicitly selects providerless reqwest 0.13 HTTPS. The standalone and application lockfile graphs were examined. The application graphs for macOS ARM64, Linux x86-64 and ARM64, iOS and its simulator, Android ARM64 and ARMv7, and Windows x86-64 contain neither AWS-LC crate in the candidate. The original wrapper archive, attribution and full license are preserved in the prototype. [Released feature list](https://docs.rs/crate/librqbit/9.1.0/features)

The candidate installs the existing ring provider before building Tauri clients or independent torrent sessions. In the extracted startup probe, eight concurrent calls shared one provider and built a reqwest client without starting a torrent session. Independent review found two Android development startup cases. The corrected source initializes the provider before any mobile webview setup and initializes the existing Android certificate verifier for every development-proxy start, including HTTP starts that can redirect to HTTPS.

An earlier standalone probe using the same SHA wrapper and TLS provider passed 30 SHA comparisons. On Linux, a process-local test root allowed an actual HTTPS tracker announce with the expected info hash. With a different root, the direct request reported an untrusted certificate and no announce reached the server. The macOS run also rejected the untrusted root. Both test servers used loopback and temporary credentials. These runs preceded the final application startup changes and software inlining.

The Linux app-library executable built from the full candidate passed three targeted tests. They covered torrent metadata against an independent info hash, transfer between two local profiles using a magnet link, and a web-seed-only transfer through the engine. The compiled feature receipt shows the checked wrapper and providerless reqwest. The candidate app library also checked successfully on macOS ARM64, in ARM64 Android development and packaged configurations, and on ARM64 iOS and its simulator.

The candidate keeps ring, including its native code. Its TLS algorithm coverage differs from AWS-LC. For example, this ring version lacks the provider's P-521 signature and hybrid X25519MLKEM768 offerings. The follow-up audits the exact Android ARM64 release package and compares whole-torrent publication latency for two Linux workloads. Physical-device timings and broader TLS algorithm compatibility remain open. [Runtime verification](2026-11-19-torrent-runtime.md) · [Whole-app worklist](../MEMORY-SAFETY-INVENTORY.md)
