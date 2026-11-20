# Checked decimal integer formatting

The application and its separate MCP relay select the same local itoa 1.0.19. JSON
responses, settings and download journals use it through serde_json. Integer HTTP
headers use it through http. The application also selects it for CSS integer output.
The replacement keeps the allocation-free, reusable stack buffer and decimal
grouping. It writes into initialized bytes and checks slice indices and UTF-8.
The crate forbids unsafe Rust.

The upstream crate archive, revision and dual MIT or Apache-2.0 license are
recorded in the source notice. Tauri includes both notices and the source notice
in its bundle resources. No archive reader, search engine or native dependency pin
changed. The private fork drops upstream's optional `no-panic` instrumentation.
The checker could not eliminate every failure branch introduced by bounds and
UTF-8 checks. No application or relay configuration inspected selected it.

The app and relay lockfiles change only the source of itoa. Locked inverse
dependency graphs selected the local crate with default features on eight app
targets and four desktop relay targets. The [earlier ARM64 Mac compiled inventory](2026-11-18-macos-checked-html-compiled-unsafe.json)
reported 16 distinct target-side itoa sites. The replacement crate forbids them.
The counts of app-owned source blocks and declarations remain 132 and 35.
This checkpoint does not cover the remaining unsafe operations in serde_json,
its float formatter or other JSON and HTTP dependencies.

## Tests and measurement

- The formatter passed 13 release tests and two doctests. The added tests compare
  every 8- and 16-bit representation, decimal width boundaries through 128 bits,
  and 100,000 deterministic values over eight wider signed and unsigned types
  using a reused buffer. The Linux build also passed the 13 runtime cases.
- The signed ARM64 Mac app library test executable passed 375 focused tests
  and skipped nine. The release MCP relay passed nine tests. It also echoed
  the largest unsigned and smallest signed 64-bit request IDs over its standard
  input and output, with successful discovery and an ordinary method error.
  Four packaging configuration tests passed and one Linux-only test skipped.
- The Linux x86-64 app library test executable passed 374 focused tests and
  skipped nine. The release MCP relay passed nine tests. The first app build
  lacked the pinned archive dependency in its offline cache. The retry
  supplied the two pinned archive and search commits to a fresh cache and kept
  the lockfile unchanged.
- Five additional reader tests passed on each desktop. They exercised GET,
  HEAD, partial and unsatisfiable responses with actual media data and checked
  the HTTP Content-Length and Content-Range behavior.
- App library compile checks passed for ARM64 iOS device, ARM64 iOS simulator
  and ARM64 Android with the evaluation feature. The formatter alone compiled
  with unsafe code forbidden on those targets and 32-bit Android ARMv7. The iOS
  checks retained an existing unused Apple PDF import warning. These are compile
  checks without mobile device execution.

The paired release fixture uses the pinned serde_json and http versions. It
formats 4,096 synthetic download records, including sizes and offsets above
4 GiB, into a reused output buffer. Separate loops format 4,096 integers and
construct 4,096 integer HTTP header values. The measuring process repeats each
loop 250 times per sample. Nine pairs alternate which binary runs first.
Both binaries write the same 753,650 JSON and header bytes on both hosts.
The output SHA-256 is
`32826a18244ee60de24ee9903cdca9a5a109895acd974b452ed86800d728af9b`.

| Host | JSON serialize | Integers in isolation | HTTP header values |
| --- | ---: | ---: | ---: |
| Mac ARM64 | 0.700 → 0.886 ms | 0.011 → 0.033 ms | 0.184 → 0.210 ms |
| Linux x86-64 | 0.797 → 1.037 ms | 0.020 → 0.054 ms | 0.129 → 0.172 ms |

Times are medians per complete fixture batch, with the upstream implementation
first. The extra checked work increases serialization time. In this fixture it
adds under 1 ms to thousands of records on both hosts. The fixture measures
serialization and header construction without network or disk time. It does
not exercise mobile UI latency or libzim.

Independent review used base `e6a86588e6bf201d990c91c26fe9d9e37fe8e859`. The
15-path source manifest, including all added source, tests and notices, has
SHA-256 `d25e2da7436f11ac0eab1cb5922fa8b7f6432e5f6ace2f67262a1e76dad01d00`.
Review checked the signed minima, decimal group boundaries, reciprocal division,
checked output slices, inherited performance characteristics and both dependency
consumers. The reviewer independently reproduced the formatter tests and output
parity. The only finding was an inaccurate buffer initialization comment. The
Mac app, Mac relay and three mobile app checks ran before that comment correction.
Both benchmarks and the Linux app used the corrected source. The comment was
the only source change between those runs.
