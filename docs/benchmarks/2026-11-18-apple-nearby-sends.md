# Apple Nearby send and callback ownership

Revision `09e57a9` replaces the raw Rust callback contexts for writes, readiness and listener acceptance. The new blocks own their captured state, and the native queue retains them through delivery. Each send starts with one synchronous copy from the caller’s slice to immutable dispatch data. The network stack can retain those bytes after the caller drops the slice or the completion fires.

A write completion takes its sender once and checks the returned length against the original piece before counting accepted bytes. Dropping an observer still permits a genuinely accepted send to count. A repeated callback cannot count it twice. Listener cancellation releases its accept block and closes the channel after queued accepts. An accepted connection that encounters a closed or full channel is dropped and cancelled.

Independent cross-language tests delayed 1,400 sends, dropped half the observers and issued 2,800 callbacks. Original bytes and upload accounting matched throughout. Another 400 cycles exercised listener readiness, acceptance and callback disposal. ARC and the block2 and dispatch2 bindings still implement native lifetime operations. Opaque connection and listener handles, their thread-safety declarations and DNS-SD callbacks remain in scope.

## Mac send overhead

The fixture used the actual Rust send method and Objective-C++ queue. A stand-in for the network stack retained the dispatch data and acknowledged each piece. Each fresh process verified every reported send length and the final retained contents after overwriting and dropping the original Rust buffer. Two warm-up pairs preceded 12 measured pairs. Baseline and candidate alternated first position. The table gives median microseconds per piece. Other work shared the host.

| Bytes per piece | Pieces submitted before waiting | Original | Owned callbacks |
|---|---:|---:|---:|
| 81 | 1 | 27.60 | 29.36 |
| 16 KiB | 1 | 28.33 | 29.22 |
| 256 KiB | 1 | 38.09 | 37.74 |
| 16 KiB | 8 | 3.38 | 3.89 |
| 256 KiB | 8 | 14.95 | 14.91 |

The 256 KiB pieces used for unrestricted sends had similar timings. Shorter pieces took longer with the new callbacks, with a 0.51 µs increase per piece for eight 16 KiB sends in flight. These timings cover local submission and completion. [Individual samples and executable hashes](2026-11-18-apple-nearby-sends.json)

## iOS local QUIC transfer

An ARM64 iOS 26.5 simulator sent bytes over an encrypted loopback stream to an independent Quinn peer. The Rust caller overwrote its source after dispatch, and the peer verified all bytes before acknowledging the message. Six paired rounds per workload used 48 fresh processes on a shared host. The new bridge preserved upload accounting when the first observer was dropped, handled an in-flight read cancellation, and rejected a write after closure.

| Message | Original verified round trip (µs) | Owned callbacks (µs) |
|---|---:|---:|
| 81 bytes | 257.24 | 241.89 |
| 16 KiB | 469.11 | 424.71 |
| 256 KiB | 3,546.89 | 3,268.30 |
| 1 MiB | 9,347.16 | 8,853.89 |

The table compares the median of the six per-process means for each build. Pairing trials before taking the median gives a 1.0% increase at 256 KiB, where submission and completion timings also varied. For the 1 MiB workload the two round-trip medians correspond to 106.98 and 112.94 MiB/s. Physical wireless throughput and power use remain unmeasured. [iOS timings and probe hashes](2026-11-18-ios-nearby-sends.json)
