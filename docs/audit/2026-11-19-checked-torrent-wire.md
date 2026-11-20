# Checked torrent metadata and peer frames

The app uses checked local versions of librqbit-bencode 9.1.0,
librqbit-peer-protocol 9.1.0 and byteorder 1.5.0. All three crates forbid
unsafe Rust. The root dependency entries keep the published versions and
select the vendored source. The resource configuration includes their original
license texts and source notices. [Bencode provenance](../../src-tauri/vendor/librqbit-bencode-9.1.0/SOURCE.txt) · [Peer protocol provenance](../../src-tauri/vendor/librqbit-peer-protocol-9.1.0/SOURCE.txt) · [Byte-order provenance](../../src-tauri/vendor/byteorder-1.5.0/SOURCE.txt)

- The bencode parser uses checked byte slices and numeric conversion. Typed
  fields and byte-string lengths reject integers outside their destination
  width. Ignored integers are checked for syntax without a numeric width
  limit. Known fields and the remaining input still follow ignored fields
  inside nested containers. The parser retains its existing nesting limit.
  The app reaches it through torrent metadata, tracker replies, DHT and
  extended peer messages.
- The peer codec checks frame length arithmetic and selects wire IDs through
  exhaustive message arms. A complete extended frame with no extension ID
  returns an invalid-length error. Request, cancel and control frame tests check fixed
  wire bytes at every buffer split. The surrounding engine still
  owns the ring buffer, socket I/O and transfer state.
- Byteorder uses checked slice conversion and preserves floating-point
  bit patterns. Its bulk `Read` methods use a 4 KiB initialized staging
  buffer and publish only fully read chunks. The public destination
  contents remain unspecified after an error. Consumers include
  torrent peer processing and host icon decoding.

## Verification

- The published crates reported 40 unsafe-code diagnostic locations on
  both Mac ARM64 and Linux x86-64 under the selected release features.
  The counts were 37 in byteorder, one in bencode and two in the peer
  codec. The checked crates reported zero on both targets. The counts
  are compiler locations before linker elimination.
- Byteorder passed 683 tests and 122 doctests. The bencode suite passed 24
  tests, and the peer suite passed 28 with both local dependencies linked.
  Tests include the published Ubuntu torrent metadata and extended handshake,
  interrupted and truncated bulk reads, exact signed and float bits, and
  framing split and boundary cases.
- The release comparison produced 238,590 identical bytes on both hosts
  from the published and checked builds. The transcript contains the
  serialized Ubuntu metadata and peer frames, split-handshake outcomes,
  and icon dimensions. Its SHA-256 is
  `f947df07c1784160d78c0185e139dc63b54dfa6da5bffd09d6efcd6f706dea10`.
- On the final source, the app passed 90 torrent tests on Mac ARM64 and
  90 on Linux x86-64. This selection includes local magnet and web-seed
  transfers, cancellation, and publication recovery. The first implementation
  snapshot passed 417 selected app tests on Mac and 416 on Linux, with six
  ignored on each. The final test binary SHA-256 values were
  `98a45ef7fc1c86861854f80b8aef39f3066c5918331bb2746eb4538f4a26355b` (Mac) and
  `7919e027b08fcbc7dd2ca59ecdfb87dc818fa8a10933dd21cdbe512175aed371` (Linux).
- The final app library compile-checked for iOS Simulator ARM64, iOS
  device ARM64 and Android ARM64. The three local crates also compiled
  for ARMv7 Android with byteorder checked separately without std.
  Eight locked app target configurations select the local crates.
  This batch's mobile evidence covers compilation, and its Windows
  evidence covers dependency resolution.

## Performance

The release comparison alternates the published and checked dependency
builds over nine pairs. The benchmark includes the indicated parse and
serialize steps. Each run averages 50 metadata parses, 80 request batches,
75 piece batches, 80 bulk reads and 200 icon directory reads. The frame
batches reuse one request or one 16 KiB piece and an output buffer, varying
the split position.
The benchmarks used rustc 1.99.1 on Mac and 1.100.0 on Linux.

| Complete fixture operation | Mac ARM64 | Linux x86-64 |
| --- | ---: | ---: |
| Parse and serialize Ubuntu torrent metadata | 0.019 → 0.019 ms | 0.260 → 0.258 ms |
| Parse and serialize 4,096 split requests | 0.057 → 0.057 ms | 0.108 → 0.106 ms |
| Parse and serialize 256 split 16 KiB pieces | 0.065 → 0.065 ms | 0.058 → 0.059 ms |
| Read 4 MiB of big-endian u64 data from a cursor | 0.155 → 0.153 ms | 0.500 → 0.738 ms |
| Read the app's ICO directory | 0.004 → 0.004 ms | 0.005 → 0.005 ms |

The largest median increase was 0.238 ms for the 4 MiB bulk read on
Linux. The three torrent fixture medians differed by less than 0.003 ms
on both hosts.

The latest native libzim comparison covers reader operations.
[Pinned reader comparison](../benchmarks/2026-11-19-mac-pinned-reader.md)

## Independent review

Two reviewers checked the implementation and its consumers, including the
tracked and new files. The initial review found a missing extension-ID
length check, an ignored-field compatibility regression, and documentation
and test gaps in byteorder. The fixes preserve unknown large integers and
check complete invalid frames before entering extension parsing. The
byteorder regression verifies both the completed prefix and the untouched
incomplete chunk. The corrected source passed independent review against
base `c11ffa7` with 44 paths and manifest SHA-256
`1b6feef2ddf65a03f070a079050c3f0fe465263e28af1dba02bf1b697fead897`.

The torrent engine's ring-buffer reborrow and third-party transport,
TLS and storage boundaries remain in the whole-app inventory.
