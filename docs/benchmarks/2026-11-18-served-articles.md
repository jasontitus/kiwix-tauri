# Opening and serving complete articles on macOS and Linux

The signed app at `bec2658` served 24 articles from Wikipedia 100 in a median 111.30 ms, compared with 112.84 ms in the older libzim app. On Climate the corresponding times were 92.07 ms and 85.36 ms. Each result is the median of ten sums, one sum per fresh app process. All 960 responses returned HTTP 200 and preserved the complete archived article bytes around the reader’s inserted script.

| Archive | libzim app (ms) | Current app (ms) | Change |
|---|---:|---:|---:|
| Climate | 85.36 | 92.07 | +7.9% |
| Wikipedia 100 | 112.84 | 111.30 | −1.4% |

The two builds alternated first position on the same Mac. Each opened its own empty profile and imported the archive before timing started. For each of the 24 distinct paths, the client timed the normal headless `open_article` command and the complete HTTP response to the returned URL. The timer included both local connections and all response bytes. Startup, import, integrity verification and screen rendering were outside the interval. The OS file cache stayed warm and other work shared the host.

The comparison used native app `f2875d0` with libzim 9.8.2. The current app includes other changes made since that revision. A separate native `zimdump show` supplied the original bytes for each item. The checker verified that removing one contiguous reader insertion from each response recovered those bytes exactly. It checked the insertion for the reader marker. The receipt includes each item’s archive hash, per-request elapsed time, response size, insertion length, HTTP status, content type and response hash. [Compressed JSON measurements](2026-11-18-served-articles.json.gz)

## Mac repeat on the Apple Nearby update

The signed app at `09e57a9` and the same libzim app repeated the ten alternating trials per archive. All 960 responses passed the original byte check. The faster app switched on both archives between the two Mac runs. Both used a shared host, and the archive reader revisions were identical in the two Rust builds. The response arrays list HTTP status, content type, response length, insertion length and response SHA-256 in that order. [Individual times and response checks](2026-11-18-served-articles-owned-callbacks.json.gz)

| Archive | libzim app (ms) | Updated app (ms) | Change |
|---|---:|---:|---:|
| Climate | 46.37 | 43.49 | −6.2% |
| Wikipedia 100 | 64.01 | 68.04 | +6.3% |

The signed handle-ID build at `26ced32` repeated the same test with ten new process pairs per archive. All 960 responses passed the archived-byte check. Climate took 69.53 ms with the Rust app and 71.38 ms with libzim (2.6% faster). Wikipedia 100 took 123.66 and 121.10 ms respectively (2.1% slower). The reader pins were unchanged. [Handle-ID build measurements](2026-11-18-served-articles-handle-ids.json.gz)

## Linux x86-64

The release Linux app at `7c5abf7` used the same two reader revisions as the signed Mac app. We built historical app `f2875d0` with the retained Linux libzim 9.8.2 SDK. Both ran as the same unprivileged test account. Ten alternating trials per archive used the same 48 paths and profile policy as macOS. The native reference driver read each original item in contiguous ranges of up to 64 KiB. All 201 length-framed ranges reconstructed the same 48 complete article hashes that native zimdump produced on the Mac. All 960 Linux app responses preserved those archived bytes.

| Archive | libzim app (ms) | Current app (ms) | Change |
|---|---:|---:|---:|
| Climate | 54.46 | 60.19 | +10.5% |
| Wikipedia 100 | 89.51 | 95.30 | +6.5% |

Each table gives per-process medians for its own machine. OS caches stayed warm, and the Linux samples exclude archive import and screen rendering. [Linux response hashes and individual times](2026-11-18-served-articles-linux.json.gz)

Direct library timings for the same random article paths measure a different workload, capped at 64 KiB per article. Native libzim still won that comparison on all four platforms. The optional retained-cluster setting remains on the worklist. [Four-platform reader measurements](2026-11-18-retained-clusters.md)
