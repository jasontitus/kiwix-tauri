# Retaining decoded clusters for article reads

The MIT reader at `15ea9926af90be6cc4dcc345f676cfe0539c014b` reduced random-article latency on macOS, Linux and the iOS simulator. Android results varied by archive and run. Native libzim remained faster for the 24-article random workload in every comparison. The application benchmark baseline was MIT `ff8b0ab` with its tuned release profile. [Private MIT implementation](https://github.com/jasontitus/zimru-xapian/commit/15ea9926af90be6cc4dcc345f676cfe0539c014b) · [Earlier four-platform baseline](2026-11-18-reader-build-profiles.md)

## Results

Each number is the median time in milliseconds for resolving 24 distinct paths and returning up to 64 KiB per article, after opening the archive. All three readers returned the same length-framed bytes. Lower is faster.

| Platform | Archive | libzim | Baseline Rust | Retained buffers |
|---|---|---:|---:|---:|
| macOS ARM64 | Climate | 28.41 | 34.54 | 30.93 |
| macOS ARM64 | Wikipedia 100 | 58.81 | 67.99 | 66.76 |
| Linux x86-64 | Climate | 25.13 | 35.55 | 31.87 |
| Linux x86-64 | Wikipedia 100 | 48.76 | 56.71 | 55.89 |
| iOS 26.5 ARM64 simulator | Climate | 32.87 | 44.05 | 40.18 |
| iOS 26.5 ARM64 simulator | Wikipedia 100 | 41.98 | 58.21 | 48.32 |
| Android 15 ARM64 emulator | Climate | 81.99 | 147.80 | 149.70 |
| Android 15 ARM64 emulator | Wikipedia 100 | 96.18 | 134.84 | 145.40 |
| Android repeat | Climate | 56.47 | 99.66 | 95.44 |
| Android repeat | Wikipedia 100 | 40.41 | 55.34 | 57.34 |

On Linux, first-article time fell from 3.90 to 2.84 ms on Climate and from 5.14 to 4.13 ms on Wikipedia 100. Native took 2.61 and 1.90 ms respectively. For random articles, median process peak memory fell from 27.74 to 24.74 MiB on Linux Climate and from 38.41 to 30.41 MiB on Mac Climate. Some small media reads already beat libzim. On Linux, Wikipedia 100 media ranges took 0.19 ms with both Rust builds and 0.31 ms with libzim.

The first Android comparison used 18 rounds. A 36-round repeat used a freshly rebuilt baseline, the same candidate binary and the identical random requests. In the repeat, candidate process CPU exceeded baseline on both archives. Mobile timing came from virtual devices on a shared Mac. Physical-device performance is open. The candidate remains on a private topic branch while Android performance is investigated. [Individual times, peak memory, driver digests and response hashes](2026-11-18-retained-clusters.json)

## How storage changes

The reader validates each compressed cluster and its offsets before exposing an item. It can now pin a boxed slice of decoded bytes for ordinary item reads. This avoids the previous mandatory copy into an inline reference-counted slice. Shrinking the decoder's vector can still move the allocation. The public whole-cluster slice accessor creates its original representation once. Old item pins keep their exact bytes alive across that transition and across cache eviction. Mixed callers can temporarily retain both allocations. The cache accounts for its logical payload bytes. Process peak memory also includes external pins and other allocations.

Independent review covered the Rust and C accessors, concurrent publication, pointer lifetime and cache eviction. A real Climate article kept its original bytes across 1,000 mixed calls on ten threads, a one-byte cache budget and the drop of the archive and item handles. The C and C++ view tests passed. The candidate also compiled for 32-bit ARM. The MIT reader-only, default and all-feature suites passed 118, 189 and 198 tests respectively, including doctests. The GPL workspace passed 67 tests. Six complete archive comparisons reproduced all 314,625,161 framed bytes from a reference using native codecs. Separate Xapian tests reproduced the existing ordered CJK results on versions 1.4 and 2.0, including their previously recorded differences. [CJK comparison](2026-11-18-portable-decoder.md)

## Where the random article reads spend decoded bytes

The 24 random article requests touched 11 compressed clusters on Climate and 15 on Wikipedia 100. For each archive we located the requested blobs in the directory, decoded each distinct cluster once with native Zstandard and verified the length-framed response hash against the original benchmark. All 26 frames lacked a Zstandard content checksum. The prefix column sums the furthest requested byte position within each distinct cluster, including its blob-offset table. [Per-item positions, cluster sizes and byte verification](2026-11-18-random-article-clusters.json)

| Archive | Bytes returned | Decoded bytes across distinct clusters | Bytes through last requested position | Decoded bytes beyond that position |
|---|---:|---:|---:|---:|
| Climate | 185,277 | 22,608,838 | 13,338,598 | 41.0% |
| Wikipedia 100 | 1,572,864 | 28,081,626 | 13,932,988 | 50.4% |

The final column is a ceiling on output that could be left unmaterialized for these particular requests. It gives no timing estimate. A reader experiment would still have to validate the entire compressed stream, blob offsets, output size and history distances, handle checksummed frames, and preserve later reads from the same cluster. The measurements use the existing reader and leave the performance switch on the worklist.

## Reproduction

Build the repository's native and Rust reader drivers, with the two MIT revisions checked out separately. Use Rust release settings with one code-generation unit and the package override of 16 for `zstd-rs`. Configure both readers with a 32 MiB cache and disable native eager Xapian loading. Replay the recorded first-article, random-article and media requests on the exact Climate and Wikipedia 100 archives. Discard one warm-up per reader and phase, alternate the reader order, and verify the length-framed output SHA-256 for every process. The primary comparisons used 18 measured rounds on each desktop and Android, and 12 on the iOS simulator. The native version was 9.8.2 except for the iOS simulator, which used 9.6.0. [Driver setup](../../scripts/bench-archive-readers/README.md) · [Requests and reference responses](2026-11-18-linux-archive-readers.json) · [Android collection procedure](../../scripts/bench-archive-readers/ANDROID.md)

A separate experiment cached an already-checked output bound in the Zstandard decoder. It improved the two Mac random-read medians by 9% and 3%, but both Linux medians regressed by about 2%. It was excluded from the retained-buffer candidate. The sample file identifies those separate runs as `ready`.
