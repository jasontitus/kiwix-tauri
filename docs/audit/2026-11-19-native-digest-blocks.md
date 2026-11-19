# Borrowed native blocks for SHA-1 and SHA-2

SHA-1 and SHA-2 now receive complete runs of streaming input as borrowed bytes. The digest buffer still owns an incomplete block. It sends a completed partial block first, then sends the contiguous run directly from the input and retains the tail. The lazy buffer retains the last full block when there is no trailing partial block. The existing wrapped-block interface and its default digest core implementation still support callers such as HMAC. [Digest source and notices](../../src-tauri/licenses/digest/SOURCE.txt)

On ARM64 and x86, the Rust SHA-1 compressors read checked big-endian words into register lanes and extract the resulting state words directly. The x86 SHA-1 instruction layout places the first message word in the high lane. ARM64 uses forward lane order. CPU feature checks still guard dispatch. The packaged forks carry their original MIT and Apache licenses. [SHA-1 source and notices](../../src-tauri/licenses/sha1/SOURCE.txt)

## Independent review and verification

The independent source review covered 65 implementation, test, dependency and notice paths against `039a70c16008340807d8f2d84414d7a545305a1c`. The sorted path and SHA-256 manifest hashes to `f60ad3598be897a995e688abd8e3177aaffa92d328d858233abe6e5f6446f722`. It verified both complete published archives and the packaged license texts. The review caught a standalone build failure because the vendored SHA crates selected the registry digest API when built directly. Both standalone manifests and lockfiles now select the paired local crates. The reviewer repeated the locked standalone tests.

The application verifier passed nine tests on macOS ARM64 with its default, forced hardware and forced software paths. It exercises the 4 GiB boundary, all six SHA-2 variants, SHA-1 compression from asymmetric states and unaligned input, and SHA-1 streaming against independent expected digests. It also checks reset, the preserved core interface, native batch grouping, direct input borrows and retry after a callback panic. The forced x86 SHA builds passed the six integration tests and two shorter SHA-256 differential tests in QEMU. The independent HMAC consumer matched Python on all 14 comparisons across SHA-1 and SHA-256 and exercised the generic fallback used by the host build.

Standalone SHA-1 passed five tests, including doc tests, with the exact Rust 1.85 compiler. Standalone SHA-2 passed sixteen tests, including doc tests. The checked buffer passed its locked all-feature suite.

## Remaining memory boundaries

The generic digest implementations still accept wrapped arrays through their existing block callback. The public low-level SHA-2 cores still use the corresponding layout conversion for callers that supply wrapped blocks directly. Those operations rely on the fixed array size, the transparent wrapper and the input slice's borrow. Some serialization and read-buffer conversions also remain in `hybrid-array`. On LoongArch, the SHA-1 backend still uses assembly with a pointer derived from one live byte or wrapped-block slice and a block count derived from the length. Both input forms retain one assembly call per batch. The CPU feature and architecture-specific boundaries remain part of the worklist. [Memory-safety worklist](../MEMORY-SAFETY-INVENTORY.md)

```sh
cargo test --locked --release --manifest-path src-tauri/vendor/sha1-0.11.0/Cargo.toml --all-features
cargo test --locked --release --manifest-path src-tauri/vendor/sha2-0.11.1/Cargo.toml --all-features
cargo test --locked --release --manifest-path scripts/check-sha256-loads/Cargo.toml -- --include-ignored
```
