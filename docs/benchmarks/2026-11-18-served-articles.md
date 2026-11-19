# Opening and serving complete articles on macOS

The signed app at `bec2658` served 24 articles from Wikipedia 100 in a median 111.30 ms, compared with 112.84 ms in the older libzim app. On Climate the corresponding times were 92.07 ms and 85.36 ms. Each result is the median of ten sums, one sum per fresh app process. All 960 responses returned HTTP 200 and preserved the complete archived article bytes around the reader’s inserted script.

| Archive | libzim app (ms) | Current app (ms) | Change |
|---|---:|---:|---:|
| Climate | 85.36 | 92.07 | +7.9% |
| Wikipedia 100 | 112.84 | 111.30 | −1.4% |

The two builds alternated first position on the same Mac. Each opened its own empty profile and imported the archive before timing started. For each of the 24 distinct paths, the client timed the normal headless `open_article` command and the complete HTTP response to the returned URL. The timer included both local connections and all response bytes. Startup, import, integrity verification and screen rendering were outside the interval. The OS file cache stayed warm and other work shared the host.

The comparison used native app `f2875d0` with libzim 9.8.2. The current app includes other changes made since that revision. A separate native `zimdump show` supplied the original bytes for each item. The checker verified that removing one contiguous reader insertion from each response recovered those bytes exactly. It checked the insertion for the reader marker. The receipt includes each item’s archive hash, per-request elapsed time, response size, insertion length, HTTP status, content type and response hash. [Compressed JSON measurements](2026-11-18-served-articles.json.gz)

Direct library timings for the same random article paths measure a different workload, capped at 64 KiB per article. Native libzim still won that comparison on all four platforms. The optional retained-cluster setting remains on the worklist. [Four-platform reader measurements](2026-11-18-retained-clusters.md)
