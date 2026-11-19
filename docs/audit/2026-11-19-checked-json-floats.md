# Checked floating-point formatting

Kiwix and the MCP relay use the same local zmij 1.0.23. The formatter retains
upstream's binary-to-decimal and rounding arithmetic. It reads the power
tables with checked indices and assembles decimal digits in initialized
stack storage. It uses scalar operations for digit assembly and retains the
compressed power tables for size-optimized builds. The library forbids
unsafe Rust and has no runtime dependencies of its own.

Both application lockfiles change only the formatter's source. Twelve locked
app and relay graphs select the local crate with default features. The fork
omits the optional upstream `no-panic` instrumentation, which the shipping
graphs did not select. The published crate's MIT notice and the source notice
are included in the app resource configuration. [Version and provenance](../../src-tauri/vendor/zmij-1.0.23/SOURCE.txt)

## Verification

- On Linux x86-64, the release comparison matched the published formatter
  for all 4,294,967,296 `f32` bit patterns and 100,028,672 `f64` cases.
  The 64-bit cases include fixed samples at every exponent and a seeded
  sample across the full bit range. The comparison checks the public
  formatting path for all cases and also checks `format_finite` on the
  nonfinite `f32` patterns and the fixed `f64` cases. The size-optimized
  build matched 8,243,700 sampled `f32` and 5,028,672 `f64` cases.
- On macOS ARM64, direct release parity matched 8,243,700 sampled `f32`
  and 100,028,672 `f64` cases. The release suite also passed its separate
  100-million-case `f64` comparison against Ryu. The debug and release suites
  passed 19 tests each. The integer oracle verified all 618 power table
  entries and passed with the expanded and compressed tables. Final
  doctests and the Clippy check with warnings treated as errors passed.
- The signed Mac app test executable passed 327 focused catalog, archive,
  acquisition, service and connector tests and skipped six. The Linux app
  test executable passed 326 focused tests and skipped six. Both used the
  locked custom protocol feature.
- The release relay passed nine tests per desktop. An isolated same-user
  backend returned decimal fields through a valid tool call. Both relays
  returned the expected numbers in structured output and JSON text. The
  fixture initially expected a fractional request ID to be accepted. The
  corrected fixture checks the protocol rejection and the next valid call.
- App library compile checks passed for ARM64 iOS devices, ARM64 iOS
  Simulator and ARM64 Android with the evaluation feature. The formatter
  also compiled by itself for Android ARMv7. Both iOS checks reported
  the existing unused Apple PDF import. Mobile verification was compile-only.

## Performance

Each pair uses the same locked dependencies except the source of zmij.
The release fixture alternates which implementation runs first. Each entry
is the median of nine measurements, each averaged over 300 repetitions.
The formatter rows cover 4,096 values of the named type. The JSON rows
serialize 4,096 synthetic catalog records containing elapsed times, rates,
fractions and mixed-language text.

| Complete fixture batch | Mac ARM64 | Linux x86-64 |
| --- | ---: | ---: |
| Format f64 list | 0.036 → 0.129 ms | 0.074 → 0.168 ms |
| Format f32 list | 0.048 → 0.129 ms | 0.072 → 0.159 ms |
| Serialize JSON into a reused byte buffer | 1.049 → 1.172 ms | 1.029 → 1.396 ms |
| Serialize JSON to an owned string | 1.149 → 1.278 ms | 1.184 → 1.529 ms |

The published version appears first. Both versions produced identical JSON
and standalone float text on both hosts. The 903,557-byte comparison receipt
has SHA-256 `5935ed0aa1028cec74574e4ddb74b8a61268f5e42075501006afc7fa93068564`.
The largest increase between JSON serialization medians was 0.37 ms.

## Independent review

Review covered base `7bb0b68e67ffdaadea95724e1c22eb7cfa99bde8` and the
19-path implementation manifest with SHA-256
`8389717e0c4b1f7627818348e376bee0f4857c841f37923089dc320d3ec25156`.
It checked table bounds for both code paths, signed output length, rounding,
the pinned consumers and MIT provenance. The reviewer also independently
computed the entire power table and replayed another 528,672 `f64` cases.
Review corrected the upstream benchmark attribution and one exponent comment.
The platform builds used the source before shortening a documentation example.
The standalone parity and benchmark inputs used the same executable source,
before the README and source-comment corrections.
