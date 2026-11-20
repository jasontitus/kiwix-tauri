# Checked torrent receive buffer

Kiwix pins librqbit 9.1.0 to local source. The receive module replaces
its raw-pointer borrow with checked slices and now forbids unsafe Rust.
It checks whether the next frame fits and whether its layout is ready
for decoding. After the final socket read or compaction, it decodes
from the original slices and updates the separate cursor fields.
A piece split at the end of the ring still borrows both original segments.

The engine rejects an oversized frame as soon as its length is known.
If the length fits the platform's integer width, the error reports how
many bytes exceed the ring's capacity. Socket read errors retain their
original cause. When an extension ID ends one segment and the whole
body starts in the next, the engine decodes the body once. A wrapped
metadata piece may still parse its small header twice. The existing
limit on unusually long optional metadata headers remains a separate
compatibility issue.

The library crate denies unsafe Rust by default. Its Windows sparse-file
system call and optional memory-mapped storage example have scoped
exceptions. Kiwix leaves the example disabled and uses its own storage
backends for torrent payload downloads and seeding. The built-in Windows
backend now supplies the byte-count output required for its synchronous
call. Tokio, the transport libraries, TLS and storage dependencies stay
in scope for the app-wide audit. [Published engine and bundled notices](../../src-tauri/vendor/librqbit-9.1.0/SOURCE.txt) · [Windows API contract](https://learn.microsoft.com/en-us/windows/win32/api/ioapiset/nf-ioapiset-deviceiocontrol)

## Verification

- The receive-buffer suite passed 11 tests. It exercised a real loopback
  TCP metadata transfer, a partial handshake, cancellation during
  a frame prefix, exact buffer capacity, a transport error, and
  pointer identity for both parts of a wrapped piece. The peer codec
  passed 29 tests with the local bencode and byte-order crates.
- The Mac ARM64 app passed 417 selected tests and the Linux x86-64 app
  passed 416. Each run had six ignored tests. The selection covers
  torrent, acquisition, archive, service and catalog cache operations,
  including a local magnet transfer and a local web seed.
- The app library passed compile checks for ARM64 Android, iOS
  Simulator and iOS device targets. This batch collected no mobile
  runtime evidence.
- The Windows sparse-file helper compiled for x86_64-pc-windows-gnu
  with windows 0.62.2 and the engine's declared Windows features.
  No Windows runtime, packaging or filesystem result was collected.

## Performance

The release benchmark alternated the published and checked receive-buffer
implementations in one process. Both used the checked peer codec and
read from the same in-memory vectored stream. Each host ran 17 pairs
per workload. The medians below exclude the first three warm-up pairs.
The two paths matched the expected checks on frame lengths, identifiers
and sampled payload bytes in every run.
Mac used rustc 1.99.1 and Linux used rustc 1.100.0.
The harness adapted the published reader to the shared error type by
widening its existing buffer-overflow count from u16 to usize.
The timed valid-frame workloads never reached that error.

| Buffered workload | Mac ARM64, published → checked | Linux x86-64, published → checked |
| --- | ---: | ---: |
| 4,096 16 KiB piece frames | 1.57 → 1.58 ms | 4.41 → 4.40 ms |
| 4,096 16 KiB metadata frames | 1.82 → 1.97 ms | 4.74 → 4.94 ms |
| 1,000,000 mixed control and bitfield frames | 17.13 → 20.53 ms | 41.40 → 45.84 ms |

The two implementations had similar piece-batch medians within each
host. The control-frame batch took longer with the checked receive buffer.
These timings cover buffered frame reads and decoding. Network
transfer, disk writes and complete torrent operations have separate
costs.

Zimru and its GPL search companion keep their existing pins and source
boundaries. Reader comparisons with native libzim are recorded separately.
[Pinned reader comparison](../benchmarks/2026-11-19-mac-pinned-reader.md)

## Independent review

Reviewers checked the full 186-path source scope against base
`28d3f5818f3fc18dcb48b90983c981455a445562`. The initial receive review
found delayed rejection of oversized frames, a read failure labeled
as a write failure and an unnecessary second decode for some
extensions. The reviewer rechecked all three fixes and independently
exercised 31,306 partial-prefix and frame-split cases, including 1,566
splits that required compaction. The expected frame lengths and
payloads matched throughout. Separate review covered the Windows call,
feature selection, upstream source and bundled notices.

The runtime-tested source manifest had SHA-256
`872cbed9804b1e063e6dbed6ca3cd371c20b98e2316701b8277094958066d62b`.
The reviewed source-only manifest after eight attribution comments
had SHA-256
`43becc229ba996549d7c911e925248a07cd596b07108dc246931ed0d73c759bf`.
