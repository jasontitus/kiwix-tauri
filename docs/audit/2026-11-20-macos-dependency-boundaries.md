# macOS dependency boundaries — 2026-11-20

This records a **default-feature** normal dependency graph for the macOS target at `449da2d919`
(then-current `main`). It is not the packaged feature set: the capture omits `custom-protocol` and other
release-only selections. It is the C3/C5 evidence pass, not a binary or runtime certification.

```sh
cargo tree --locked --manifest-path src-tauri/Cargo.toml --target aarch64-apple-darwin \
  -e normal,features > docs/audit/2026-11-20-macos-dependency-graph.txt
```

The full 5,669-line output is checked in beside this note. The command resolves the lockfile only; it does
not run build scripts, inspect linked archives or prove which code a packaged binary imports.

## Providers that process untrusted input

| Dependency | Reached through | Boundary |
|---|---|---|
| `aws-lc-rs` / `aws-lc-sys` | `librqbit`'s `reqwest` 0.13 and its tracker/UPnP crates, `hyper-rustls`, shared `rustls` | Native C/assembly TLS. The shared rustls graph contains both providers; each consumer's feature selection differs. |
| `ring` | The app's `reqwest` 0.12 (ring + WebPKI roots), direct Quinn (`rustls-ring`), the updater's `rustls-no-provider` + `ring`, and `snow` | Native C/assembly crypto. The torrent hash wrapper uses checked RustCrypto through `sha1-ring`, not ring itself. |
| `sherpa-onnx` / `sherpa-onnx-sys` | `kokoro` offline speech | Static native inference runtime; model files are archive-adjacent input. |
| `zstd-rs` | Zimru reader, Git-pinned at `cbafcb41` | Rust Zstandard implementation in the MIT reader; the app keeps its bounded decoder settings. |
| `lzma-rust2` | Zimru reader, same pin | Rust XZ decoder path. |
| `bzip2-turbo-rs` | Kokoro model installation (`src-tauri/src/kokoro/install.rs:1140`) | Rust bzip2 implementation; check its feature selection on each release profile. |
| `wry` / `tao` / `tauri-runtime-wry` | Desktop shell. `wry` and `tauri-runtime-wry` are vendored; `tao` 0.37.2 comes from crates.io. | OS WebView/window glue; the system renderer is platform-owned. |
| `rodio` / `cpal` | Kokoro playback | OS audio device APIs. |

The graph also contains `librqbit-sha1-wrapper`, which uses checked RustCrypto digest code through its
historical `sha1-ring` feature; AWS-LC remains a TLS provider, not the torrent hash provider.

## What this does not close

- It does not choose a TLS provider or replace `aws-lc-rs`/`ring`. Both remain native crypto boundaries, and
  consumer-level provider selection still needs its own audit.
- It does not isolate `sherpa-onnx` model parsing or inference.
- It does not inspect the packaged executable's imports, static archives, linker maps or the selected native
  SDK manifests. Those are the remaining C5 artifacts for each release target.
- It covers one target and one default feature selection. iOS, Android, Linux, Windows, the release feature
  set and the relay need their own locked graphs.

Selected application modules that call these dependencies carry `#![forbid(unsafe_code)]` where they have no
native boundary (for example the archive, search, LocalSwarm and map modules). That is not crate-wide:
`src-tauri/src/catalog.rs` constructs a `reqwest` client without the lint. The census gate covers the rest.
The dependencies above remain the reason full memory safety is not yet claimed.
