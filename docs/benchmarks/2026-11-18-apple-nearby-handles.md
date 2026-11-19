# Apple Nearby connection and listener IDs

At `26ced32`, the app exchanges process-local integer IDs with the Apple transport. Rust keeps one lease per connection or listener and shares it with outstanding jobs. The native table looks up the exact ID under a lock, then releases the lock before calling the network stack, invoking a callback or waiting for shutdown. Each queued operation owns the connection it selected. Dropping the last lease removes its ID and queues cancellation. Connections receive odd IDs and listeners receive even IDs. Both counters stop at exhaustion.

This removes raw connection and listener pointers from the Rust interface and removes four manual thread-safety declarations. The Objective-C++ implementation still owns native connections and callbacks. The DNS-SD callback contexts, route address buffers and dispatch bindings remain in scope.

## Lifetime checks

The native regression verified that a pending write completed on its original connection, a late start failed after cancellation and stale and wrong-kind IDs failed without affecting a live replacement. The table tests exercised both counter ranges at exhaustion. An independent Rust-to-Objective-C++ run raced join, cancel, duplicate retirement and route lookup across 4,096 IDs. A separate AddressSanitizer and UndefinedBehaviorSanitizer run checked 1,024 shutdown lifecycles in 32 batches of 32 concurrent connections. Each join waited for its own transport, and all observed connection and listener objects were reclaimed.

## Mac queue timings

The earlier send and read fixtures exercised the same production Rust and Objective-C++ methods on the previous and current revisions. Each process verified the bytes after the operation. The network completion was supplied locally. Runs alternated build order and discarded the first two pairs. Each cell is the median elapsed microseconds per piece from 12 processes for sends and 10 for reads. Builds and other work shared the host. [Raw process timings, CPU, peak memory and executable hashes](2026-11-18-apple-nearby-handles.json)

| Send size | Pieces in flight | Previous (µs) | With IDs (µs) |
|---|---:|---:|---:|
| 81 bytes | 1 | 11.89 | 13.38 |
| 16 KiB | 1 | 13.33 | 11.67 |
| 256 KiB | 1 | 27.99 | 26.39 |
| 16 KiB | 8 | 3.30 | 3.16 |
| 256 KiB | 8 | 12.79 | 12.39 |

| Read size | Dispatch regions | Previous (µs) | With IDs (µs) |
|---|---:|---:|---:|
| 81 bytes | 1 | 36.30 | 36.51 |
| 16 KiB | 1 | 41.02 | 38.11 |
| 256 KiB | 1 | 55.20 | 59.24 |
| 256 KiB | 16 | 59.85 | 61.13 |
| 256 KiB | 128 | 75.32 | 83.25 |

The 256 KiB read with 128 regions was 10.5% slower by the two per-build medians. Calculating each paired change before taking the median gave a 3.8% increase for that case. Queue timings exclude the radio, archive decoding, disk access and screen rendering.

## iOS simulator QUIC transfer

An independent Quinn recipient checked each payload before acknowledging it over the encrypted loopback connection. The sender overwrote its source after submission. Each process checked cancellation, upload accounting and refusal of a post-close write. Six alternating pairs per workload gave 48 fresh processes. The table compares the median per-message round trip from each build’s six processes. The final column is the median change within matched pairs. [Individual observations and 81-byte repeat](2026-11-18-ios-nearby-handle-ids.json)

| Message | Previous (µs) | With IDs (µs) | Paired change |
|---|---:|---:|---:|
| 81 bytes | 374.99 | 402.34 | +15.7% |
| 16 KiB | 848.49 | 848.88 | −6.6% |
| 256 KiB | 5,382.11 | 5,294.52 | −2.9% |
| 1 MiB | 16,710.15 | 17,002.80 | −0.1% |

The shortest message was repeated in 18 additional pairs after the iOS app build and hosted suite finished. Its round-trip medians were 217.33 and 218.25 µs, with a median paired change of −0.3%. Local submission rose from 3.36 to 3.67 µs. Other workloads shared the host in both runs. These measurements cover the iOS simulator’s loopback network. Physical wireless performance remains open.
