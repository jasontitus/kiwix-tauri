# Voice-pack decompression and installation review

The app at `810e997` uses `bzip2 0.6.1` and `libbz2-rs-sys 0.2.6` to unpack the optional English Kokoro download. A Mac arm64 comparison found a faster decoder that forbids unsafe Rust. The installation changes are still experimental.

| Streaming decoder | Median elapsed | Median CPU | Median peak process RSS |
| --- | ---: | ---: | ---: |
| App baseline, bzip2 0.6.1 | 16.08 s | 14.58 s | 5.72 MiB |
| bzip2-turbo-rs 0.1.0 with its std feature | 10.77 s | 10.65 s | 10.12 MiB |

Each decoder streamed into the same buffered output. A separate process hashed the bytes. One warm-up per decoder preceded four measured fresh processes per decoder, alternating order. The host also ran builds and emulator work. The 181,886,349-byte compressed file matched the app's pinned SHA-256. All ten outputs were 221,801,472 bytes and had SHA-256 `ce23b8a10c500f7897abdcfbe1e35d711235fc6e8abb1228914dc71bb5fd192e`. The measurements cover decoding, output and resource use before installation or speech. [Individual samples](../benchmarks/2026-11-18-voice-pack-decode.json)

The candidate has no runtime crates beyond the Rust standard library in this configuration. It compiled with unsafe code forbidden. It decoded the full distributor file and correctly handled small single, empty and concatenated streams. Truncation, a changed block checksum and trailing garbage returned errors when the stream was read through EOF. A tar consumer can stop before the codec reaches the final checksum. The candidate installation therefore caps the decompressed bytes, consumes remaining tar padding, checks the expected expanded length and reads through codec EOF.

The compressed download in the candidate uses one anonymous file from hashing through extraction. Review found that the next handoff still resolves the extracted model by pathname. Publication, cleanup and later native inference require separate ownership checks. The following items remain open.

- Bind staged files and their parent to the verified installation through publication. A same-name replacement can otherwise be published or deleted. Preserve unrelated files and support recovery of an interrupted install.
- Revalidate the installed model before passing its files to sherpa-onnx. File existence alone cannot establish which model is loaded. Review bundled models, downloaded models and upgrades.
- Serialize cancellation and privacy changes with publication and the automatic performance probe, including a request arriving after the final cancellation check.
- Verify recovery, disk-full behavior and cold reopening on all supported platforms. Speech synthesis and the native ONNX runtime remain in the app's memory-safety inventory. [Application inventory](../MEMORY-SAFETY-INVENTORY.md)
