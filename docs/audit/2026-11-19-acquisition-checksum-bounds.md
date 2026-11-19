# Bounded download checksum reads

The live HTTP worker and the durable finalizer now read at most the admitted download length plus one byte during the checksum pass. The additional byte detects growth. An early EOF or any extra byte fails verification with the existing length-change error. Only an exact-length stream can proceed to the retained-file identity check, the expected digest comparison and publication.

The durable finalizer reads directly from the retained file, avoiding buffered application read-ahead when the admitted length is nearly exhausted. Both paths keep their existing stop checks and receipt rules. The four-byte signature is read separately before the cursor is reset for the checksum pass.

The new tests extend an owned temporary part while the live worker or recovery sweep is waiting at an existing test gate. They check the retained file position, failed job status, absence of a verification receipt, and preservation of the changed part without creating the final archive. The tests exercise an exact read-buffer boundary and a partial final buffer.

The independent review covered the five Rust files below against base `08cd6030fedca2c85421c51d2482c6b423ef8ec7`. The full-index source diff has SHA-256 `a9c6b88acf601283e3d94f33f86f6306bf8b15e295a507b8605c1267be507819`.

- `src-tauri/src/acquisition.rs`
- `src-tauri/src/acquisition/finalize.rs`
- `src-tauri/src/acquisition/manager.rs`
- `src-tauri/src/acquisition/transfer.rs`
- `src-tauri/src/acquisition_tests.rs`

The reviewer reproduced an evidence gap in the first test version. A buffered reader could return the requested bytes yet advance the underlying file farther. The revised tests query the retained file. The reviewer checked the fix and traced short reads, end-of-file, the Tokio read limit and stop handling, durable receipts, and Unix and Windows ownership semantics.

The optimized acquisition tests passed 98 tests with three ignored on both macOS ARM64 and Ubuntu 24.04 x86-64. Both runs passed the new regressions. The unfiltered Rust app library test binaries passed 1,259 tests with 25 ignored on macOS and 1,242 tests with 16 ignored on Ubuntu. The first Ubuntu run lacked the frontend dependencies needed by a PDF fixture. The fixture and the full rerun passed after restoring the dependencies from the matching lockfile. The checksum regressions exercise the normal worker and sweep via isolated temporary profiles and a loopback fixture. The application read bound is separate from operating-system cache and storage readahead.

Eight alternating pairs of the existing 256 MiB loopback transfer benchmark completed with four range requests per run on macOS ARM64. Median elapsed time was 1.075 s before the change and 1.030 s after it. Median process CPU time was 0.69 s before and 0.65 s after. This sample covers the complete local transfer, checksum and publication in the Rust test binary.

The app library also passed compile checks for iOS ARM64, the iOS ARM64 simulator, Android ARM64 and Android ARMv7. Runtime results in this checkpoint cover macOS and Ubuntu. Windows ownership was reviewed from source.

Run the two regressions and the acquisition suite with an optimized build and the app's normal native dependencies.

```sh
cargo test --manifest-path src-tauri/Cargo.toml --locked --release --lib --features custom-protocol reads_at_most_one_byte
cargo test --manifest-path src-tauri/Cargo.toml --locked --release --lib --features custom-protocol acquisition::
```
