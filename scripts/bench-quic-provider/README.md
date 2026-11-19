# RustCrypto QUIC transport experiment

This standalone, unpublished probe exercises Quinn's Rustls adapter with separate reference and candidate binaries. It uses one loopback UDP listener per trial, short-lived test certificates, and synthetic data. It has no dependency on the app, Zimru or the Xapian companion. [Methods, results and review](../../docs/audit/2026-11-19-quic-provider.md)

The two patches apply to the published crates listed below. Quinn's patch is available under MIT or Apache-2.0, matching [Quinn](https://crates.io/crates/quinn-proto/0.11.19). The OxiQUIC patch is Apache-2.0, matching [OxiQUIC](https://crates.io/crates/oxiquic-crypto/0.2.1) by COOLJAPAN OU (Team Kitasan). License texts are in [licenses](licenses). The probe source follows the repository's [GPL-3.0 license](../../LICENSE).

## Reproduce in a fresh directory

The commands below use macOS, Python 3.11 or newer, OpenSSL and Rust. Start in this directory. The downloads are the public registry archives. Certificate generation writes disposable fixture keys into a private temporary directory. Keep that directory out of Git.

```sh
set -eu
umask 077
source_dir="$PWD"
work="$(mktemp -d)"
work="$(cd "$work" && pwd -P)"
cp "$source_dir"/Cargo.toml "$source_dir"/Cargo.lock "$source_dir"/run-wire.py "$source_dir"/loopback.sb "$work/"
cp -R "$source_dir/src" "$work/"
mkdir "$work/vendor" "$work/bin" "$work/fixture"
cd "$work"
curl -fsSLo quinn-proto.crate https://static.crates.io/crates/quinn-proto/quinn-proto-0.11.19.crate
curl -fsSLo oxiquic-crypto.crate https://static.crates.io/crates/oxiquic-crypto/oxiquic-crypto-0.2.1.crate
shasum -a 256 -c <<'SHA256'
0e750cca55fe4f0439a15d0bb529da9651e79993e8e72c61a899a36d462befbe  quinn-proto.crate
eda47a8450216387405776893d20a268767731b6c15be10ce8fc09709246953d  oxiquic-crypto.crate
SHA256
tar -xzf quinn-proto.crate -C vendor
tar -xzf oxiquic-crypto.crate -C vendor
patch -F 0 -p1 < "$source_dir/quinn-proto-0.11.19.patch"
patch -F 0 -p1 < "$source_dir/oxiquic-crypto-0.2.1.patch"
for label in paired wrong; do
  openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out "fixture/$label.key.pem"
  openssl req -new -x509 -key "fixture/$label.key.pem" -subj /CN=localhost \
    -days 1 -addext 'subjectAltName=DNS:localhost' -out "fixture/$label.cert.pem"
done
for mode in reference candidate; do
  cargo build --locked --release --features "$mode"
  cp target/release/kiwix-quic-wire-probe "bin/$mode"
done
shasum -a 256 "$PWD/bin/reference" "$PWD/bin/candidate" > wire-binaries.sha256
python3 run-wire.py > results.jsonl
```

The earlier macOS timing run also applied [earlier-retry-precedence.patch](earlier-retry-precedence.patch) after the Quinn patch. It affects only builds that enable both candidate and native backend features. Use the current patch for further mixed builds. `cargo build --locked --release --features candidate,quinn/rustls-ring` builds the mixed candidate with native Retry handling. Save it separately if comparing it with the candidate-only binary.

On Linux, build with a compiler and C runtime compatible with the test image. In a read-only container started with external networking disabled, use the same driver after replacing its macOS sandbox launcher line with `sandbox = []`. Regenerate the binary manifest with the two absolute paths as seen inside the container. The launcher binds both endpoints to IPv4 loopback. Remove the temporary directory and fixture keys when finished.

## Compare typed AES storage

The [AES patch](typed-aes-0.9.4.patch) keeps the published AES 0.9.4 implementation's MIT or Apache-2.0 terms. The RustCrypto Developers and the original notices are preserved in [licenses/aes](licenses/aes). [Measurements and review](../../docs/audit/2026-11-19-aes-storage.md)

Start in the fresh probe directory from the earlier instructions, with the current Quinn and OxiQUIC patches and test fixture ready. Run the following once in a fresh directory for each AES configuration. First run `export RUSTFLAGS=''` for default dispatch or `export RUSTFLAGS='--cfg aes_backend="soft"'` for forced software. Both builds use the candidate QUIC provider. The two labels compare the AES package sources.

```sh
cargo build --locked --release --features candidate
cp target/release/kiwix-quic-wire-probe bin/reference
curl -fsSLo aes.crate https://static.crates.io/crates/aes/aes-0.9.4.crate
printf '%s\n' 'fc0fc36d7440d015b45468ba08a21e80e056ebbd014c611c6298bb7503e04bd1  aes.crate' | shasum -a 256 -c
tar -xzf aes.crate -C vendor
patch -p1 -F 0 < "$source_dir/typed-aes-0.9.4.patch"
printf '\naes = { path = "vendor/aes-0.9.4" }\n' >> Cargo.toml
cargo update -p aes --offline
cargo build --locked --release --features candidate
cp target/release/kiwix-quic-wire-probe bin/candidate
shasum -a 256 "$PWD/bin/reference" "$PWD/bin/candidate" > wire-binaries.sha256
python3 run-wire.py > aes-storage-results.jsonl
```

The AES lockfile entry should lose its registry source and checksum while retaining all dependencies. To run the [direct operation benchmark](aes-storage-micro.rs), put it at `src/main.rs` in a separate Rust 2024 package. Use `aes = { version = "=0.9.4", features = ["zeroize"] }`, and build with and without the same local patch. The recorded Linux run used `inout` 0.2.6, `zeroize` 1.9.2, Rust 1.100.0 and one CPU.
