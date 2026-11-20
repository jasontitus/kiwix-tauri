# Current reader vs libzim on OSM archives — 2026-11-20

These runs check the merged memory-safety build against the native reader at the application's own cache
size. They use the repository's standalone drivers and the public pins: Zimru `cbafcb41` (reader, default
features disabled) and libzim 9.8.2 from the retained SDK. The archives are local OSM extracts, not the
recorded Wikipedia fixtures, so the numbers are current-build evidence rather than a replacement for the
earlier corpus.

| Archive | Bytes | SHA-256 |
|---|---:|---|
| `osm-san-francisco-bay-area-2026-04-15.zim` | 250,788,360 | `d827dafadd5c0d4df1cd60373fbf0fc655bbc2583ced8ec1d89ab974dd5f5b9c` |
| `osm-silicon-valley-overture.zim` | 203,804,316 | `ca0f5f3efc7f956fee116e6b21f6a2377b8ee211280e4e8354143619ef9cf442` |
| `osm-palo-alto-overture.zim` | 11,372,846 | `bf4bea35f544012be39cec18bf177bf5168bf9e83bb707df49f30f057550979d` |

Host: macOS 27.2.1 arm64. Rounds: 6 with one warm-up, alternating driver order. Cache: 33,554,432 bytes
(the app's `archive.rs` setting). The collector byte-verified every response.

| Archive | Phase | libzim (ms) | Zimru (ms) | Ratio |
|---|---|---:|---:|---:|
| San Francisco | Open | 1.828 | 0.025 | 0.01× |
| San Francisco | Open + first article (combined) | 1.993 | 0.769 | 0.39× |
| San Francisco | First article only | 0.230 | 0.748 | 3.26× |
| San Francisco | 32 repeated reads | 0.012 | 0.026 | 2.24× |
| San Francisco | 24 random reads | 3.908 | 3.103 | 0.79× |
| Silicon Valley | Open | 1.647 | 0.024 | 0.01× |
| Silicon Valley | Open + first article (combined) | 2.494 | 0.828 | 0.33× |
| Silicon Valley | First article only | 0.855 | 0.809 | 0.95× |
| Silicon Valley | 32 repeated reads | 0.013 | 0.021 | 1.69× |
| Silicon Valley | 24 random reads | 2.466 | 2.045 | 0.83× |
| Palo Alto | Open | 0.992 | 0.021 | 0.02× |
| Palo Alto | Open + first article (combined) | 2.010 | 0.899 | 0.45× |
| Palo Alto | First article only | 1.008 | 0.878 | 0.87× |
| Palo Alto | 32 repeated reads | 0.011 | 0.019 | 1.64× |
| Palo Alto | 24 random reads | 1.040 | 0.896 | 0.86× |

The combined open-plus-first-article timing comes from each process's `api_open_plus_work_ns` sample; it is
not a sum of separate phase medians. Ratios are computed before rounding the millisecond columns.

The app-relevant combined path and the 24-article random batch are faster than libzim on all three archives.
First-article-only is mixed (slower by 0.52 ms on San Francisco, faster on the other two). The 32-repeat
phase is slower by 14.62, 8.77 and 7.31 microseconds absolute; all six San Francisco libzim samples were
10.75–14.66 µs and all six Zimru samples 26.12–39.75 µs, so the reversal is observed rather than a claim
that noise dominates. A local 4 MiB-cache control measured the San Francisco random batch at 9.248 ms
(libzim 7.392 ms); that control changed the cache budget, not the safety boundary. The application uses
32 MiB.

```sh
bench_tools="$PWD/scripts/bench-archive-readers"
bench_lab=$(mktemp -d)
ln -s /path/to/zimru-cbafcb41 "$bench_lab/zimru"
cp -R "$bench_tools/rust-driver" "$bench_lab/rust-driver"
cargo build --release --manifest-path "$bench_lab/rust-driver/Cargo.toml"
c++ -std=c++17 -O3 -DNDEBUG -I"$PWD/.native/desktop-retained/aarch64-apple-darwin/include" \
  -L"$PWD/.native/desktop-retained/aarch64-apple-darwin/lib" \
  "$bench_tools/native.cpp" -lzim -o "$bench_lab/native"
bench_rust="$bench_lab/rust-driver/target/release/positioned-reader-bench"
bench_native="$bench_lab/native"
python3 "$bench_tools/measure.py" "$bench_native" "$bench_rust" \
  /path/to/osm-san-francisco-bay-area-2026-04-15.zim \
  --rounds 6 --cache-bytes 33554432 \
  --phases open first_article random_articles repeat_article_32 \
  --output "$bench_lab/san-francisco.json"
# Repeat the measure command with the other two archive paths and output names.
```

Raw collector output: [San Francisco](2026-11-20-osm-reader-current.json),
[Silicon Valley](2026-11-20-osm-reader-current-silicon-valley.json),
[Palo Alto](2026-11-20-osm-reader-current-palo-alto.json).

Limits: three archives, one host, warm filesystem cache, no app rendering, search, writing, media ranges,
Windows, Linux, iOS or Android. Physical-device and packaged-app performance remain unmeasured here.
