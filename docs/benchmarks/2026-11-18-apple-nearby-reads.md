# Apple Nearby receive checks

Revision `60ea4a5` replaces a raw callback context and raw byte slice on incoming Apple QUIC reads with a single-use request ID and typed dispatch data. Optimized macOS tests exercised simultaneous native QUIC requests, a connected peer that never opened a request stream, retrieval of a live connection’s interface name, and Quinn clients reading from a native listener. All four live tests passed. Five additional tests passed for receive limits, cancellation, duplicate callbacks, empty data, errors, identity and service tokens. Independent cross-thread checks also delivered callbacks after cancellation. The bridge still calls native code.

## Receive timing

The table shows median microseconds per read. Each process used the production Rust read method, Objective-C++ receive bridge and dispatch queue. A fixture supplied an immutable buffer with known bytes in place of the network receive. Each run checked every returned length and the final buffer contents. Runs alternated the baseline and candidate, discarding two warm-up pairs per case. The same Mac hosted ten measured pairs on macOS and six on an ARM64 iOS 26.5 simulator. Other builds shared this host.

| Read size | Dispatch regions | Mac before | Mac after | iOS simulator before | iOS simulator after |
|---|---:|---:|---:|---:|---:|
| 81 bytes | 1 | 63.82 | 64.71 | 18.01 | 18.19 |
| 16 KiB | 1 | 88.12 | 95.87 | 61.49 | 88.59 |
| 256 KiB | 1 | 63.99 | 56.20 | 128.79 | 132.10 |
| 256 KiB | 16 | 100.95 | 94.23 | 118.50 | 89.27 |
| 256 KiB | 128 | 108.79 | 104.79 | 86.16 | 63.57 |

Fragmented 256 KiB reads improved on both targets. Contiguous 16 KiB reads took 8.8% longer on macOS and 44.1% longer in the simulator. Physical-device transfer throughput and latency under sustained traffic remain open. Compare these figures within each workload. Each includes queue scheduling, allocation and copying. The macOS process CPU and peak memory samples also include process startup. [Individual timings and executable hashes](2026-11-18-apple-nearby-reads.json)

The changes leave the MIT reader and GPL search pins intact. Archive comparisons against native libzim use distinct article and media workloads. [Reader results](2026-11-18-retained-clusters.md)
