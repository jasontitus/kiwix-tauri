# Checked Java string codecs

Android's two JNI bindings now use checked local versions of cesu8 1.1.0
and simd_cesu8 1.2.0. The legacy codec validates the UTF-8 it produces.
The newer codec uses the standard UTF-8 validator, checked word and
array reads, and a buffer that grows if lossy replacements require
more than the input length. Both crates forbid unsafe Rust. The app's
lockfile drops simdutf8 0.1.5.

Canonical modified UTF-8 uses two bytes for NUL and encodes each
UTF-16 surrogate separately. Android ART emits four-byte UTF-8
for supplementary characters while retaining the modified NUL.
The published JNI codecs can lose the NUL when a string contains
both forms. The checked lenient Java decoders validate and accept
four-byte UTF-8 in the same string as modified sequences. Strict
modified UTF-8 decoding retains the canonical rules. [JNI string format](https://docs.oracle.com/en/java/javase/25/docs/specs/jni/types.html#modified-utf-8-strings) · [Android ART JNI implementation](https://android.googlesource.com/platform/art/+/3eb64751e5a7e69b06a0979edbdfe9c4d503ea57/runtime/jni/jni_internal.cc#226)

The ordinary Android ARM64 and ARMv7 dependency graphs select both codecs.
JNI 0.21 reaches the legacy codec through Android runtime dependencies.
JNI 0.22 uses the modern codec in runtime string conversion and through
its host-executed macros. JNI handles, borrowed Java string buffers and
the bindings' unchecked CString constructors remain separate boundaries.
The Mac, Linux and iOS app graphs have no JNI dependency in the checked
configurations. [Legacy source and notice](../../src-tauri/vendor/cesu8-1.1.0/SOURCE.txt) · [Modern source and notices](../../src-tauri/vendor/simd_cesu8-1.2.0/SOURCE.txt)

## Verification

- Both desktop hosts compared the published and checked codecs across
  all 1,112,064 Unicode scalars and 67,513 additional byte sequences.
  The comparison checked encoded output and Unicode round trips,
  plus decoded output, errors and borrowing. All 67,513 samples
  covered standard CESU-8 and the strict Java decoders. Three lenient
  Java APIs were compared on 67,497 samples. Sixteen samples with
  complete four-byte UTF-8 units were excluded from those three
  comparisons because that is the changed case. The independent
  modern-codec probe verified all 1,048,576 supplementary scalars
  between modified NULs.
- The legacy suite passed three unit tests. The modern codec passed
  two unit tests and 33 doctests, and compiled without `std`.
  An additional release probe reran the comparison with the modern
  codec's optional portable SIMD path and unstable features enabled.
- An ARM64 Android compiler probe reported 72 unsafe-code locations
  in the three published crates. The counts were two in cesu8, 27 in
  simd_cesu8 and 43 in simdutf8. The two checked codecs each reported
  zero in the same selected configuration.
- OpenJDK 17 on Mac ARM64 exercised both JNI bindings with
  `-Xcheck:jni`. The two builds produced identical Java strings,
  including embedded NUL and supplementary Unicode, and exact
  UTF-16 units for the unpaired-surrogate fallback cases. JNI returned
  `c080eda0bdedb38d` for NUL followed by U+1F4CD.
- An instrumented APK on an owned Android 15 API 35 ARM64 emulator
  exercised both JNI bindings. The same NUL and U+1F4CD fixture
  returned `c080f09f938d` from JNI. Both checked decoders preserved
  exact text across 1,061 sampled scalars. The published JNI paths
  returned two replacement characters in place of NUL in the 1,000
  cases that also contained a supplementary scalar. Exact fallback
  outputs for unpaired UTF-16 matched. The probe APK and emulator
  were removed after the check.
- The Mac and Linux app suites passed 463 and 462 selected tests,
  respectively, with six ignored on each. The app library passed
  compile checks for ARM64 Android, iOS Simulator and iOS device.
  The standalone JNI libraries also linked for ARMv7 Android.
  No ARMv7 or iOS runtime was exercised in this checkpoint.

## Performance

The probe measures 1,024 JNI string round trips, including Rust
conversion in both directions and Java equality checks. Each sample
warms up with 200 calls, then records five timed batches. The table
reports the median of the per-process medians from seven alternating
published and checked pairs.

The Android pairs cover ASCII and Basic Multilingual Plane text. The
mixed Android workload is timed only on the checked build because the
published codec loses the NUL in that input.

| JNI binding and input per round trip | Mac ARM64, published → checked | Android ART API 35 ARM64 emulator, published → checked |
| --- | ---: | ---: |
| JNI 0.21, 8,193 ASCII characters | 22.82 → 22.66 ms | 214.81 → 204.31 ms |
| JNI 0.21, 2,732 Japanese BMP characters | 33.80 → 33.12 ms | 137.42 → 136.26 ms |
| JNI 0.21, mixed NUL and supplementary text, 5,464 UTF-16 units | 59.35 → 63.97 ms | 250.02 ms checked |
| JNI 0.22, 8,193 ASCII characters | 17.56 → 17.67 ms | 184.02 → 176.94 ms |
| JNI 0.22, 2,732 Japanese BMP characters | 26.13 → 30.42 ms | 117.90 → 127.63 ms |
| JNI 0.22, mixed NUL and supplementary text, 5,464 UTF-16 units | 56.80 → 60.65 ms | 231.77 ms checked |

These results cover Java string conversion. The MIT archive reader and
GPL search companion keep separate source histories. Their reference
reader measurements are recorded separately.
[Matched-cache comparison with libzim](../benchmarks/2026-11-19-mac-pinned-reader.md)

## Independent review

Review covered both codecs, including surrogate and NUL handling,
checked ranges, exact outputs and borrowing. The modern codec no
longer reserves three times the input size on lossy conversion. Both
README copies identify the historical upstream benchmark results and
describe the checked implementation. A separate review verified the
JNI callers, runtime and build dependency selection, the original
publisher notices and the matched Apache license text.

The 30-path source manifest against base
`a2aa08947dab4d031b6933b4b1eb0bfc6f5c9fa1` has SHA-256
`dbe13fab632788eb95f53578cdc8d8f5f5904ee52bae3ecf62e5669bc34154eb`.
An independent review checked the disposable emulator's transport pin,
artifact identity verification and cleanup across failure boundaries.

An initial Android shell probe stopped before producing codec output.
An instrumented APK reached the JNI call and found the published
decoder's mixed-string failure. That earlier window and its test
package and emulator were withdrawn before the corrected probe.
