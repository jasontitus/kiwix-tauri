<!-- Prepared 2026-11-05 against ikatson/rqbit main at f23bacdd (after v9.1.0).
     Patch: librqbit-sha1-rustcrypto.patch (apply with `git am`). Not yet filed. -->

# sha1w: add a pure-Rust `sha1-rustcrypto` backend and a `rust-tls-no-provider` feature

## Summary

Today a rustls build of librqbit (`rust-tls`) always compiles aws-lc (aws-lc-rs + aws-lc-sys, with its CMake/C build). It comes in through two paths:

1. `sha1w/sha1-ring`. Despite its name, this backend has been implemented with `aws_lc_rs::digest` since e25612b ("switch ring to aws-lc-rs").
2. `reqwest/rustls`. In reqwest 0.13 this enables reqwest's default rustls provider, which is aws-lc-rs.

Applications that already use a different rustls provider, such as `ring`, therefore carry a second crypto stack and a C toolchain requirement just for librqbit. This PR adds an opt-in, purely additive alternative. Existing feature sets are unchanged.

- **`librqbit-sha1-wrapper`: new `sha1-rustcrypto` backend**, using RustCrypto's `sha1` 0.11 and `sha2` 0.11. It is pure Rust and picks hardware SHA instructions at runtime: SHA-NI on x86/x86_64, and the SHA extensions on aarch64 (new in `sha1` 0.11). `exactly_one!` now covers three backends.
- **`librqbit-core`: `sha1-rustcrypto`** passes the feature through.
- **`librqbit`: new `rust-tls-no-provider`** = `reqwest/rustls-no-provider` + `sha1-rustcrypto`. It follows reqwest's own naming. The application installs the rustls `CryptoProvider` it already uses before creating a `Session`, for example `rustls::crypto::ring::default_provider().install_default()`. The Cargo.toml comment documents this. librqbit does not install a process-global provider itself.
- **Tests:** FIPS 180-1 / RFC 3174 SHA-1 vectors (`abc`, the 448-bit message split across two `update` calls, and one million `a`). These run against whichever backend is selected, so they also cover the existing backends.
- **CI:** clippy for `librqbit` with `--no-default-features --features rust-tls-no-provider`, and the sha1w tests with `sha1-rustcrypto`.

## Performance

SHA-1 is the hot path ("the majority of CPU usage", per sha1w's header). Measured on x86_64 with SHA-NI: 256 MiB buffer, 5 runs, `--release`, identical digests.

| Backend | Throughput |
|---|---|
| aws-lc-rs (`sha1-ring` today) | 1746 MB/s |
| RustCrypto `sha1` 0.10/0.11 (`sha1-rustcrypto`) | 1653 MB/s |
| ring 0.17 | 440 MB/s (no SHA-NI) |

That is why this PR adds a RustCrypto backend rather than pointing `sha1-ring` back at `ring`.

## Compatibility

Purely additive. The `default-tls` and `rust-tls` feature sets and their dependency graphs are byte-for-byte unchanged, and no public API changes. With `rust-tls-no-provider`, `cargo tree --target all` shows no aws-lc and no OpenSSL library in librqbit's normal dependency graph. The only OpenSSL-named crate is `openssl-probe`, which locates CA bundle files and links nothing.

## Testing

- `cargo test -p librqbit-sha1-wrapper --no-default-features --features sha1-rustcrypto`: 2 passed (the new SHA-1 vectors and the existing SHA-256 vector).
- `cargo test -p librqbit-sha1-wrapper` (default `sha1-crypto-hash`): 2 passed. The new vectors also hold for the existing backend.
- `cargo test -p librqbit-core --no-default-features --features sha1-rustcrypto`: 31 passed.
- `cargo clippy -p librqbit --no-default-features --features rust-tls-no-provider --all-targets`: no warnings.
- `cargo check -p librqbit` (default) and `--no-default-features --features rust-tls`: unchanged and clean.
- `cargo fmt --all -- --check`: clean.
- `sha1`/`sha2` are workspace dependencies with `default-features = false`. Cargo.lock gains no packages, only the two new edges from librqbit-sha1-wrapper (`sha1` 0.11 was already locked).
- Note: running librqbit's own test suite under `rust-tls-no-provider` would need the tests to install a provider first, so CI runs clippy for that feature set rather than the full suite.

## Motivation

Kiwix's cross-platform reader embeds librqbit for magnet downloads and seeding on Linux, macOS, Windows, Android and iOS. Its TLS already runs on `ring`. With this feature it can drop aws-lc-rs, aws-lc-sys and their build tooling (cmake, fs_extra, jobserver, untrusted 0.7) without vendoring or patching librqbit.
