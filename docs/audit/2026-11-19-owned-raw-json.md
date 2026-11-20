# Checked raw JSON ownership

The vendored serde_json 1.0.154 library now forbids unsafe Rust with every
feature. This removes the three casts used to treat string allocations and
references as `RawValue`, along with the unchecked public constructor. The app keeps the `raw_value` feature used by Tauri.

Tauri continues to call `RawValue::from_string` and use `Box<RawValue>`.
The constructor validates one JSON value and excludes surrounding whitespace.
For an unpadded, exactly sized string it retains the original character
allocation. The boxed owner uses one additional allocation for metadata.
Serde serialization preserves the same JSON text and an existing `&RawValue`
still implements `Deserializer`.

To deserialize a borrowed field, use `BorrowedRawValue<'a>` by value in
place of the former `&'a RawValue` destination. The new type holds the original
`&str`. Its `into_owned()` method copies the span when the value must outlive
the input. Both forms retain the original display, debug and embedded JSON
formatting. The migration is local to the forked dependency. [Source and
migration notice](../../src-tauri/vendor/serde_json-1.0.154/SOURCE.txt)

The library lint applies to serde_json source. Its dependencies, including
memchr and the floating-point formatter, and the separate JavaScript and
platform runtimes remain in scope for the app-wide audit.

## Verification

- The final Mac release suites passed 144 default-feature and 160 all-feature
  runtime tests plus 97 and 102 doctests. The nightly compiler diagnostic
  harness was skipped in each suite. Both allocation-only configurations
  compiled, with and without `raw_value`. Clippy passed on the all-feature
  library with warnings treated as errors. The focused regressions check
  original input addresses, boxed ownership transfer, malformed text,
  borrowed map keys and debug formatting.
- The pinned Tauri callback formatter produced a small JavaScript literal
  callback and a large callback using JSON.parse. Both returned the original
  values in an isolated Node.js context on each desktop host. The formatter
  returned an error for incomplete JSON before producing a callback.
- The signed Mac app test executable passed 327 focused tests and skipped
  six. The release MCP relay passed nine tests and preserved the unsigned
  and signed integer limits through its standard-input protocol.
- The Linux app test executable passed 326 focused tests and skipped six.
  The release relay passed nine tests. The first app retry reused an earlier
  artifact because unpacking the overlay preserved source timestamps. The
  final run confirmed compilation of the reviewed JSON library before
  executing the suite. Both desktop app builds used the locked custom
  protocol feature.
- App library compile checks passed for ARM64 iOS devices, ARM64 iOS
  Simulator and ARM64 Android with the evaluation feature. The patched JSON
  library also compiled by itself for Android ARMv7. Both iOS checks reported
  the existing unused Apple PDF import. Mobile verification was compile-only.
- Twelve locked app and relay feature graphs match the previous selection.
  The upstream license files and bundle resource mappings are unchanged.

## Performance

The paired release fixture uses the pinned Tauri 2.12.5 callback formatter.
It alternates the preceding checked-string implementation and the new ownership
implementation. Both targets use the same lockfile. Each table entry is the
median of nine averages. The small cases use 10,000 repetitions per sample and
the large cases use 400. Measurements use synthetic JSON containing multibyte
text, quotes, backslashes and line separators.

| Operation | Mac ARM64 | Linux x86-64 |
| --- | ---: | ---: |
| Borrow small JSON from a string | 33 → 33 ns | 37 → 47 ns |
| Borrow large JSON from bytes | 0.225 → 0.226 ms | 0.248 → 0.261 ms |
| Validate and own small JSON | 63 → 88 ns | 57 → 76 ns |
| Validate and own large JSON | 0.112 → 0.113 ms | 0.110 → 0.120 ms |
| Validate and own large JSON with surrounding whitespace | 0.118 → 0.117 ms | 0.284 → 0.296 ms |
| Format small Tauri callback | 160 → 184 ns | 169 → 185 ns |
| Format large Tauri callback | 0.385 → 0.378 ms | 0.714 → 0.848 ms |

The preceding version appears first. The ownership and callback rows include
cloning the prepared input string. Both timed callbacks call `format_raw` on
pre-serialized JSON. Node execution was a separate functional check. Each
version produced a byte-identical 580,311-byte receipt of
callback scripts and expected values on both hosts. Its SHA-256 is
`03725617f69da2703dd18558db6c82ab559c07a00af0bbbf6ba056b6c9d48816`.

## Independent review

The seven-path implementation manifest at base
`0712ccfc2fecf36013efad10b6e14070e1808f07` has SHA-256
`b64a5259d0f373f70dd6afe361f0e2210779a4f78056cf259b52a4d00b50e5ef`.
Review verified ownership, input lifetimes, the retained Tauri constructor and
all 31 deserializer methods and their arguments. It found a changed debug
representation for borrowed values. The shared formatter restores the original
representation. The review also clarified how to migrate borrowed destinations
and when to take ownership. The corrected source passed a second review.
