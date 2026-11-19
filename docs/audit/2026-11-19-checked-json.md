# Checked JSON string handling

Kiwix and the MCP relay select the same local serde_json 1.0.154. The reader
uses checked offsets for scanning chunks and initialized storage for escaped
bytes. Unescaped input from a Rust string can still be borrowed directly.
Byte and stream inputs keep their UTF-8 validation and error positions.
For Rust string input, the reader now validates the buffer created by escape
decoding. Byte-oriented deserialization still represents lone UTF-16 surrogates
as WTF-8. The writer slices the original string at ASCII escape
boundaries and checks UTF-8 before returning owned strings or displaying a
JSON value.

This removes nine explicit unsafe blocks from the reader, writer and value formatter.
The crate denies unsafe Rust except in four functions associated with the
`raw_value` feature. Three convert references or boxes between a string and
its transparent `RawValue` wrapper. The fourth is the existing public unsafe
unchecked constructor. Tauri's callback formatter calls the validating
`RawValue::from_string` constructor, which still uses the representation
conversions. Eight locked app graphs select `raw_value`. Four locked desktop
relay graphs leave it off. The remaining raw-value functions, the float
formatter, and other dependencies are separate parts of the app-wide audit.

Both lockfiles change only the source of serde_json. The vendored copy retains
the upstream MIT and Apache-2.0 license texts and the app resource configuration
includes both along with the source notice. [Source revision and crate checksum](../../src-tauri/vendor/serde_json-1.0.154/SOURCE.txt)

## Verification

- The Mac release suites passed 144 default-feature and 159 all-feature runtime
  tests, plus 97 and 106 doctests respectively. The compiler diagnostic tests
  require nightly and were skipped. The crate also compiled with only allocation
  support, with and without `raw_value`. The all-feature library passed Clippy
  with warnings treated as errors. The regression test covers escaped
  and unescaped multibyte strings, borrowed spans, and control-character error
  columns across 40 offsets.
- The pinned Tauri callback formatter ran in a standalone fixture. An isolated
  Node.js JavaScript context received the original values from both a small literal
  callback and a large callback using JSON.parse. The Rust fixture also rejected
  incomplete JSON before producing a callback.
- The signed Mac app library test binary passed 327 focused catalog, archive,
  download, service and connector tests and skipped six. The release MCP relay
  passed nine tests and returned the same signed and unsigned integer limits
  over its normal standard-input and standard-output interface.
- The Linux app library test binary passed 326 focused tests and skipped six.
  The release MCP relay passed nine tests. Both desktop app checks used the
  locked custom protocol feature.
- App library compile checks passed for ARM64 iOS devices, ARM64 iOS
  Simulator and ARM64 Android with the evaluation feature. The JSON crate
  also compiled by itself for 32-bit Android ARMv7. Both iOS checks reported
  the existing unused Apple PDF import. Mobile verification was compile-only.

## Performance

Each pair used the same locked dependencies except the source of serde_json.
The release fixture alternated which executable ran first. The main payload
contains 4,096 synthetic
catalog records with mixed-language strings, escapes and large sizes. The
display and raw-value rows use a separate long article body. The relay row
parses and serializes 1,000 small synthetic JSON-RPC messages in memory.

| Operation | Mac ARM64 | Linux x86-64 |
| --- | ---: | ---: |
| Parse catalog from a string | 2.893 → 2.931 ms | 6.861 → 7.084 ms |
| Parse catalog from bytes | 3.450 → 3.451 ms | 7.617 → 7.698 ms |
| Parse catalog through an in-memory reader | 6.131 → 6.172 ms | 12.750 → 14.059 ms |
| Write catalog into a reusable byte buffer | 1.078 → 1.104 ms | 1.202 → 1.274 ms |
| Serialize catalog to an owned string | 1.096 → 1.276 ms | 1.236 → 1.544 ms |
| Display a JSON article value | 0.084 → 0.202 ms | 0.101 → 0.255 ms |
| Borrow the raw JSON article | 0.051 → 0.051 ms | 0.051 → 0.051 ms |
| Parse and serialize JSON-RPC messages | 0.708 → 0.719 ms | 0.830 → 0.835 ms |

Each table entry is the median of nine measurements, each averaged over
100 repetitions of the listed workload. Upstream appears first. The largest
increase between medians was 1.31 ms for the Linux in-memory reader. Compact and
pretty serialization of three prepared values produced the same 2,964,226
bytes on both hosts and versions. The SHA-256 is
`4873ba44673ad875e2abacfd074f4cbaa64e1df8e3fc52900cecace9dfbec014`.
The timings cover in-memory serialization, parsing and display formatting.

## Independent review

Review covered base `827899e874d97c3a66c24db16d57caccb927c394` and the
92-file source manifest with SHA-256
`ea10ab63496cd91cfa5e03d60dffa1ccaa781b40b974fb970910da3bbcb50da5`.
It checked scanner offsets, the borrowed slice contract, WTF-8 byte generation,
serializer boundaries, the pinned dependency consumers and license resources.
A missing zero-copy test and the spelling of the Cargo feature were corrected.
The desktop app library builds, mobile compile checks and paired fixtures
used the earlier snapshot `3cd6b48f7986cf1eae269ab7729f2c2b9fe771713b622768e4431bdc4418ee7f`.
Only the regression test and source notice changed afterward. The final release
suites exercised the added assertions.
