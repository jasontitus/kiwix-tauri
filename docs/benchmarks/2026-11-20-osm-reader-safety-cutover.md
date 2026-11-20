# OSM reader performance after the safety cutover — 2026-11-20

This run rechecks the app-relevant reader path after the Rust reader, checked decoders, single
ring TLS provider and iOS proof-carrying C paths landed. It uses the same three OSM archives,
host, 32 MiB cache, six measured rounds per reader and alternating reader order as the earlier
[OSM comparison](2026-11-20-osm-reader-current.md).

The app-relevant combined open-plus-first-article path and the 24-article random batch remain
faster than libzim on all three archives. Every phase is within 10% of the earlier run; most are
faster. The 32-repeat phase remains slower by tens of microseconds in absolute terms, as before.

| Archive | Phase | libzim (ms) | Zimru (ms) | Ratio |
|---|---|---:|---:|---:|
| San Francisco | Open | 1.973 | 0.021 | 0.01× |
| San Francisco | Open + first article (combined) | 2.110 | 0.758 | 0.36× |
| San Francisco | First article only | 0.211 | 0.738 | 3.50× |
| San Francisco | 32 repeated reads | 0.011 | 0.027 | 2.45× |
| San Francisco | 24 random reads | 3.779 | 3.036 | 0.80× |
| Silicon Valley | Open | 1.597 | 0.020 | 0.01× |
| Silicon Valley | Open + first article (combined) | 2.521 | 0.824 | 0.33× |
| Silicon Valley | First article only | 0.848 | 0.805 | 0.95× |
| Silicon Valley | 32 repeated reads | 0.011 | 0.022 | 2.00× |
| Silicon Valley | 24 random reads | 2.395 | 2.002 | 0.84× |
| Palo Alto | Open | 0.972 | 0.019 | 0.02× |
| Palo Alto | Open + first article (combined) | 1.925 | 0.895 | 0.46× |
| Palo Alto | First article only | 0.963 | 0.876 | 0.91× |
| Palo Alto | 32 repeated reads | 0.010 | 0.019 | 1.90× |
| Palo Alto | 24 random reads | 1.024 | 0.880 | 0.86× |

The combined timing is each process's `api_open_plus_work_ns` sample, not a sum of separate phase
medians. Ratios are computed before rounding. The 32-repeat absolute differences are 16.0, 11.0
and 9.0 microseconds for San Francisco, Silicon Valley and Palo Alto.

```sh
bench_tools="$PWD/scripts/bench-archive-readers"
bench_lab=$(mktemp -d)
ln -s /path/to/zimru-cbafcb41 "$bench_lab/zimru"
cp -R "$bench_tools/rust-driver" "$bench_lab/rust-driver"
cargo build --release --manifest-path "$bench_lab/rust-driver/Cargo.toml"
c++ -std=c++17 -O3 -DNDEBUG -I"$PWD/.native/desktop-retained/aarch64-apple-darwin/include" \
  -L"$PWD/.native/desktop-retained/aarch64-apple-darwin/lib" \
  "$bench_tools/native.cpp" -lzim -o "$bench_lab/native"
python3 "$bench_tools/measure.py" "$bench_lab/native" \
  "$bench_lab/rust-driver/target/release/positioned-reader-bench" \
  /path/to/osm-san-francisco-bay-area-2026-04-15.zim \
  --rounds 6 --cache-bytes 33554432 \
  --phases open first_article random_articles repeat_article_32 \
  --output "$bench_lab/san-francisco.json"
```

Repeat with the Silicon Valley and Palo Alto archive paths. Raw collector output:
[San Francisco](2026-11-20-osm-reader-safety-cutover.json),
[Silicon Valley](2026-11-20-osm-reader-safety-cutover-silicon-valley.json),
[Palo Alto](2026-11-20-osm-reader-safety-cutover-palo-alto.json).

Limits: three archives, one macOS ARM64 host, warm filesystem cache, no app rendering, search,
writing, media ranges, Windows, Linux, iOS or Android. The TLS provider and iOS C-path changes do
not run in this reader-only comparison.
