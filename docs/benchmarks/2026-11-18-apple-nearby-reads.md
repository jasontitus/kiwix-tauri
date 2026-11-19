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

## Final payload and stream closure

A simulator test found that both the old bridge and `60ea4a5` could deliver the last bytes and then report an error. Network.framework sometimes delivers the final payload and end-of-stream indication in the same callback. The next native receive then reports POSIX `ENODATA`. A content context can be marked final even while more bytes remain.

The [follow-up fix](https://github.com/jasontitus/zimru-xapian/commit/fc8d2c4e174278206aed837de474e5da4a576ec5) records end-of-stream only after successful completion of the final context. It still delivers that payload and rechecks route readiness before exposing data. An empty non-final message returns control to the Rust future, which can request another message while it remains active. Revocation, other errors and pre-existing concurrent reads preserve their separate outcomes.

On an ARM64 iOS 26.5 simulator, real loopback TCP and pinned QUIC each transferred 600,123 known bytes with either an immediate finish or a finish following an application acknowledgement. The corrected production receive code returned all bytes and clean EOF in all four runs. The QUIC peer used a separate Quinn implementation. These checks exercised the native bridge and Rust read method. On macOS, the native regression also checked non-final completion, final data, repeated reads and a route revoked before delivery.

A fresh ten-pair macOS fixture comparison ran the same five shapes with the final fix against the original baseline. Median 16 KiB contiguous reads took 13.42 µs before and 14.42 µs after. The respective 256 KiB values were 25.90 and 26.11 µs for one region, 33.05 and 26.91 µs for 16 regions, and 32.95 and 31.42 µs for 128 regions. Absolute timings varied substantially between sessions on the shared host. The additional samples are in the linked measurements above.

The changes leave the MIT reader and GPL search pins intact. Archive comparisons against native libzim use distinct article and media workloads. [Reader results](2026-11-18-retained-clusters.md)
