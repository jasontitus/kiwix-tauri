# Opening and serving complete articles on macOS and Linux

The signed Mac app at `3e9e976` served 24 complete Climate articles in a median 36.16 ms, against 35.99 ms for the historical libzim app. Wikipedia 100 took 63.91 ms against 61.40 ms. All 960 responses passed the archived-byte check. Across the ten pairs, the Rust app was faster in four Climate pairs and in none of the Wikipedia 100 pairs. The table gives milliseconds per 24 requests. Each cell is the median of ten fresh-process sums.

| Mac app build and evidence | Climate with libzim | Climate with Rust | Wikipedia 100 with libzim | Wikipedia 100 with Rust |
|---|---:|---:|---:|---:|
| [`bec2658`, voice and receive](2026-11-18-served-articles.json.gz) | 85.36 | 92.07 | 112.84 | 111.30 |
| [`09e57a9`, send callbacks](2026-11-18-served-articles-owned-callbacks.json.gz) | 46.37 | 43.49 | 64.01 | 68.04 |
| [`26ced32`, connection handles](2026-11-18-served-articles-handle-ids.json.gz) | 71.38 | 69.53 | 121.10 | 123.66 |
| [`541725a`, Bonjour discovery](2026-11-18-served-articles-bonjour.json.gz) | 46.49 | 46.49 | 74.69 | 77.66 |
| [`d0c967c`, address checks during builds](2026-11-18-served-articles-address-checks.json.gz) | 212.38 | 200.83 | 235.56 | 236.14 |
| [`d0c967c`, same binaries after builds](2026-11-18-served-articles-address-checks-postbuild.json.gz) | 37.71 | 38.04 | 65.00 | 66.65 |
| [`e46993d`, resolver operation numbers](2026-11-18-served-articles-route-queries.json.gz) | 46.76 | 49.21 | 72.11 | 70.49 |
| [`3e9e976`, interface-bound route checks](2026-11-18-served-articles-bound-routes.json.gz) | 35.99 | 36.16 | 61.40 | 63.91 |

The two builds alternated first position on the same Mac. Each opened its own empty profile and imported the archive before timing started. For each of the 24 distinct paths, the client timed the normal headless `open_article` command and the complete HTTP response to the returned URL. The timer included both local connections and all response bytes. Startup, import, integrity verification and screen rendering were outside the interval. The OS file cache stayed warm and other work shared the host. The two `d0c967c` runs illustrate how much both apps’ absolute times varied with build activity.

All Mac runs used native app `f2875d0` with libzim 9.8.2. The Rust app includes other changes made since that revision. A separate native `zimdump show` supplied the original bytes for each item. The checker verified that removing one contiguous reader insertion from each response recovered those bytes exactly. It checked the insertion for the reader marker. Each receipt identifies the two executable hashes and the requested items and records per-request time and response checks. The compact response rows contain HTTP status, content type, response length, insertion length and response SHA-256 in that order.

## Linux x86-64

The release Linux app at `7c5abf7` used the same two reader revisions as the signed Mac app. We built historical app `f2875d0` with the retained Linux libzim 9.8.2 SDK. Both ran as the same unprivileged test account. Ten alternating trials per archive used the same 48 paths and profile policy as macOS. The native reference driver read each original item in contiguous ranges of up to 64 KiB. All 201 length-framed ranges reconstructed the same 48 complete article hashes that native zimdump produced on the Mac. All 960 Linux app responses preserved those archived bytes.

| Archive | libzim app (ms) | Current app (ms) | Change |
|---|---:|---:|---:|
| Climate | 54.46 | 60.19 | +10.5% |
| Wikipedia 100 | 89.51 | 95.30 | +6.5% |

Each table gives per-process medians for its own machine. OS caches stayed warm, and the Linux samples exclude archive import and screen rendering. [Linux response hashes and individual times](2026-11-18-served-articles-linux.json.gz)

Direct library timings for the same random article paths measure a different workload, capped at 64 KiB per article. Native libzim still won that comparison on all four platforms. The optional retained-cluster setting remains on the worklist. [Four-platform reader measurements](2026-11-18-retained-clusters.md)
