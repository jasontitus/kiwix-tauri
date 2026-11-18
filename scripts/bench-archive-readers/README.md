# Compare public archive readers

These standalone drivers call native libzim and Rust Zimru directly. Each opens a regular ZIM from a path in a new process. The harness targets trusted Wikipedia test archives. The application, WebView and search engine are outside these measurements.

On the recorded arm64 macOS host, the native executable linked the retained libzim 9.8.2 SDK, built from upstream `26ec526f74e8342a40da5ab14e364546988e0e1a` with the application’s split-range and retained-file-descriptor patches. The Rust executable used `30e171038660edcac518e6144be2466dd8084d8f` with default features disabled. The lock file pins the reader’s dependency versions. Build tools were Apple clang 21.0.0 and Homebrew Rust 1.99.1. Both benchmark drivers live in this application repository.

## Build and run

Use the prepared SDK and a clean copy of the matching MIT reader revision. From the application repository root, the commands below create a new scratch directory. Set `archives` to the absolute paths to the local ZIMs. The recorded corpus contains Simple English May 2026 in both maxi and nopic editions, Chinese Chemistry mini, Climate mini June 2024 and Nollywood maxi October 2026. Archive identities, selected paths, query inputs, output hashes, medians and individual samples are in the [comparison data](../../docs/benchmarks/2026-11-18-direct-archive-readers.json).

```bash
bench_tools="$PWD/scripts/bench-archive-readers"
bench_sdk="$PWD/.native/desktop-retained/aarch64-apple-darwin"
bench_mit="$PWD/.native/integration-branches/zimru"
test "$(git -C "$bench_mit" rev-parse HEAD)" = 30e171038660edcac518e6144be2466dd8084d8f || exit 1
bench_lab=$(mktemp -d)
ln -s "$bench_mit" "$bench_lab/zimru"
cp -R "$bench_tools/rust-driver" "$bench_lab/rust-driver"
cargo build --release --locked --manifest-path "$bench_lab/rust-driver/Cargo.toml" --target-dir "$bench_lab/rust-driver/target"
c++ -std=c++17 -O3 -DNDEBUG -I"$bench_sdk/include" -L"$bench_sdk/lib" "$bench_tools/native.cpp" -lzim -o "$bench_lab/native"
otool -L "$bench_lab/native"
bench_rust="$bench_lab/rust-driver/target/release/positioned-reader-bench"
archives=(/path/to/simple-nopic.zim /path/to/simple-maxi.zim /path/to/chemistry.zim /path/to/climate.zim /path/to/nollywood.zim)
python3 "$bench_tools/measure.py" "$bench_lab/native" "$bench_rust" "${archives[@]}" --output "$bench_lab/default.json"
python3 "$bench_tools/measure.py" "$bench_lab/native" "$bench_rust" "${archives[@]}" --cache-bytes 4194304 --phases first_article random_articles media_ranges --output "$bench_lab/matched.json"
python3 "$bench_tools/measure.py" "$bench_lab/native" "$bench_rust" "${archives[@]}" --eager-native --phases open --output "$bench_lab/eager.json"
python3 -B "$bench_tools/summarize.py" "$bench_lab/default.json" "$bench_lab/matched.json" "$bench_lab/eager.json" --native-library "$bench_sdk/lib/libzim.9.dylib" --sdk-manifest "$bench_sdk/kiwix-sdk.json" --rust-revision 30e171038660edcac518e6144be2466dd8084d8f --output "$bench_lab/comparison.json"
```

The Python collector uses Unix child resource accounting on macOS and Linux. A Linux replay requires a corresponding native SDK and a matching compiler/linker configuration. Output publication creates new files in the chosen directory and preserves existing files. Native selection and both drivers run with a 90-second timeout per invocation. Use scratch storage that supports hard links and has room for the sampled responses.

## Sampling and counters

A fixed random seed selects up to 24 distinct nonredirected HTML entries and eight images, audio or video entries of at least 8 KiB from the native public path listing, outside timing. Both libraries then receive identical paths, offsets, lengths and title prefixes. Each fresh output includes length framing. The harness compares every byte and records a SHA-256 digest. Prefix checks compare the total and the paths/titles of the first four hits.

| Phase | Operation |
| --- | --- |
| Open | Construct one archive and verify its all-entry count. The API timer covers the constructor. |
| First article | Resolve the first article path and copy up to 64 KiB from offset zero. |
| Repeat article | Prime that path once, then measure 32 resolves and copies from the same open archive. |
| Random articles | Resolve the selected distinct article paths in their original deterministic random order and copy up to 64 KiB each. |
| Title prefixes | Look up as many as eight distinct three-character prefixes derived from sampled titles and retrieve the first four results. |
| Media ranges | Resolve each selected media path twice. Copy 4 KiB at halfway and 4 KiB at the end. Compressed clusters can require substantially more internal reading and decoding than the requested payload. |

There is one discarded warm-up and ten measured fresh processes per reader, phase and archive. Alternating order gives each reader five first and five second positions. Checksumming the input and priming each case warms the OS file cache. Machine activity and scheduling can affect timings. Each sample's `api_ns` covers its named archive operation, including lookup and copying. Response serialization happens after each operation's timer. `open_ns` records construction. For a first read, `api_open_plus_work_ns` sums the construction and article operation from that same process. `process_ns`, `cpu_ns` and peak RSS include driver startup, input, output, cleanup and any warm request. Raw RSS units are normalized to bytes.

The main comparison disables Xapian preloading in libzim, matching the application’s archive-open setting. The explicit eager-open control enables it. With default cache policies the drivers report a 16 MiB cache budget for libzim and 64 MiB for Zimru. The 4 MiB control applies an equal requested cache target to both libraries. Each library reports its own cluster-cache accounting after the phase. Zimru may retain a final cluster larger than the target. Native accounting also accounts for cluster offsets and a portion of decoder state. The counters and whole-process peak RSS measure different costs. All reads use the libraries directly, including Rust article and media reads, while the application’s article/media route still uses libzim.
