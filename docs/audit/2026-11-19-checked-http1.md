# Checked HTTP/1 request and response headers

The app selects local builds of httparse 1.10.1 and Hyper 1.12.0.
Httparse uses checked input slices and writes parsed headers into
initialized slots. Token and header data still borrow from the input.
The parser forbids unsafe Rust and retains its allocation-free
`no_std` build. The local fork omits the uninitialized-header APIs.
Hyper's HTTP/1 client and server are the two callers that migrated.
The pinned Hyper proxy tunnel and torrent discovery consumers already
used the initialized API. [Parser source and license](../../src-tauri/vendor/httparse-1.10.1/SOURCE.txt) · [Hyper source and license](../../src-tauri/vendor/hyper-1.12.0/SOURCE.txt)

Hyper records checked URI and header ranges within the parsed head,
before separating it from the request or response body. It uses initialized
range entries and the checked shared-buffer constructor for header values.
The production HTTP/1 role module forbids unsafe Rust. Tokio, HTTP/2,
TLS providers, shared byte buffers and other Hyper modules remain
separate boundaries. Hyper's optional nightly benchmarks still use
unchecked byte-buffer resets. The archive-reader and search pins keep
their separate MIT and GPL source histories.

## Verification

- The exact parser passed 95 unit tests. It also compiled without `std`
  for macOS ARM64 and ARMv7 Android. The independent replay compared
  101,832 prefix, capacity and configuration cases with published
  httparse. It checked partial input and errors, same-object retries,
  preserved slots, byte offsets, empty values and chunked trailers.
  All 203,032 transcript rows matched. A separate parity run matched
  91,016 cases on each desktop host.
- The Mac release compiler reported 50 unsafe-code locations in published
  httparse and zero in the checked crate. The count covers the selected
  `std` build before linker elimination.
- Hyper loopback sessions passed on both desktop hosts with both
  dependency builds. Requests at the configured limits of 100 and 128
  headers succeeded. Requests one over each limit returned HTTP 431.
  The checks also exercised fragmented requests, empty fields, an
  informational response, a folded header, chunked trailers, HTTP/0.9
  fallback and pipelining.
- The app passed 462 selected tests on Mac ARM64 and 461 on Linux
  x86-64, with six ignored on each. The selection covers archive
  reads, catalog, network acquisition, service reader routes and
  torrent paths, including resumed range requests and local web seeds.
- The app library compile-checked for ARM64 Android, iOS Simulator
  and iOS device targets. This checkpoint collected no mobile runtime
  or packaging evidence.

## Performance

Each release measurement alternates the published and checked builds.
The table shows medians from 14 pairs after three warm-up pairs.
The parser comparison runs both crates in one process. A raw TCP client
sends requests to two otherwise matched Hyper servers. The timer runs
from binding the loopback listener through counting the 1,024 expected
payload occurrences in the response stream. Mac used rustc 1.99.1 and
Linux used rustc 1.100.0.

| Measured batch | Mac ARM64, published → checked | Linux x86-64, published → checked |
| --- | ---: | ---: |
| Parse 100,000 requests with a range header | 9.29 → 10.14 ms | 12.14 → 17.32 ms |
| Parse 100,000 partial-content responses | 11.22 → 12.72 ms | 14.08 → 20.62 ms |
| Parse 8,000 responses with an 8 KiB header value | 3.46 → 35.69 ms | 12.18 → 33.44 ms |
| Serve 1,024 pipelined loopback requests | 2.60 → 2.77 ms | 2.90 → 3.28 ms |

The 8 KiB header-value batch took 10.3× the published time on Mac and
2.7× on Linux. The absolute increases were 32.24 ms and 21.26 ms
over 8,000 responses. The ordinary loopback batches completed in under
4 ms on both hosts. Archive loading, TLS and remote network throughput
have separate costs.

This checkpoint measures HTTP/1, including shared application transport
for catalog and download requests. It keeps the existing Zimru and
GPL search pins. [Matched-cache reader comparison with libzim](../benchmarks/2026-11-19-mac-pinned-reader.md)

## Independent review

An independent cursor review caught test comments that still described
the removed SIMD backends. The corrected cursor and scanner passed
review before the header-storage migration. That review covered the
published source delta and confirmed that the parser preserves the
same initialized-API results and borrowing.

The integration reviewer verified both release archives and licenses.
Two fixes bound borrowed intervals to the parsed head and return a
parse error if the header map cannot represent the count. The range
regression passed independently using the exact helper and test with
a minimal error enum. The client and server docs now describe their
actual inline capacity. Review covered the 86-file final source against
`9a868a98947db8c3acaf677a5a56c93cf75cf8b8`, manifest SHA-256
`e48bcaa01a55951fa4095ce90c432ab60a03a07c2b1db22f562241f5340640dd`.
The stock Hyper unit suite did not run because its pinned test
dependency graph could not resolve offline. The checked runtime used
the actual linked HTTP/1 client and server.
