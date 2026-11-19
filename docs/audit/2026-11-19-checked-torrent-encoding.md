# Checked torrent filename encoding

Kiwix uses checked local versions of chardetng 1.0.0 and encoding_rs 0.8.44
to identify and decode filenames in BitTorrent metadata. The torrent engine
continues to use the published scoring, encoding tables, and codec state
machines. The detector counts high bits in checked words. The decoder reads
UTF-16 from borrowed byte pairs, validates UTF-8 through Rust's core library,
and writes into initialized storage. Both crates forbid unsafe Rust.
[Detector provenance](../../src-tauri/vendor/chardetng-1.0.0/SOURCE.txt) · [Codec provenance](../../src-tauri/vendor/encoding_rs-0.8.44/SOURCE.txt)

The locked application dependency graph selects serial detection. The codec fork omits
six unused functions that wrote directly into a fixed-length mutable string
slice. Its `simd-accel` feature is a no-op. Byte-slice, owned-string and
streaming decoder interfaces remain available. Streaming string output
preserves the caller's buffer capacity and bounds initialized scratch by the
decoder's worst-case output for that chunk, with room for a complete scalar.
The application's complete filename decode uses one initialized output vector
when it needs owned text. The desktop resource configuration includes the
original license and copyright notices. Neither Zimru source tree changed.

## Verification

- Default-feature release compiler checks of these crates reported 145
  unsafe-code diagnostic locations on Mac ARM64 and 148 on Linux x86-64
  in the published versions. The checked versions reported zero on both.
  Linux's published count includes two locations in the detector. These
  are per-crate compiler locations before linker elimination.
- The final codec passed 155 tests and three doctests with all features.
  The detector passed 91 tests with all features against the final codec
  and checked byte-search dependency. The codec also compile-checked for
  Android ARMv7 without default features. Locked app configurations for
  seven target triples resolve both local crates.
- The release comparisons on Mac ARM64 and Linux x86-64 covered all 65,536
  two-byte inputs for each of the 40 encodings and 20,000 generated inputs
  fed to the detector with three chunk sizes. Both versions produced the
  same comparison fingerprint on both machines. The normal torrent parser,
  validator and filename iterator returned byte-identical output for 891
  generated filenames across nine fixtures. The emitted names and charset
  labels had SHA-256
  `47d6db048244c10c9f24684990009359a79a0a6d74dfb0ab61843afbc124de9b`.
- On the exact final source, 90 application torrent tests passed on macOS
  ARM64 and 90 passed on Linux x86-64. The initial implementation also
  passed 417 selected application tests on Mac and 416 on Linux, with six
  ignored on each. The final Mac and Linux app test binaries had SHA-256
  `585b9ce9c83e9297b36985abab259cc92a808040741723058ef95655ff834a4a`
  and `61c2bddfdf9e32d2b45ce42fd3c2e129bad1376a5ac77f8897d728047ce6c987`,
  respectively.
- The final application library compiled for ARM64 iOS Simulator, ARM64
  iOS device and ARM64 Android. The mobile evidence covers compilation.
  Windows evidence covers dependency resolution.

## Performance

Each measurement includes bencode parsing, metadata hashing, filename
encoding detection, validation, and decoding 2,048 filenames through
librqbit-core. Test inputs are generated in the listed source encodings.
The two release builds use the same benchmark and hash implementation.
Each number is the median of nine alternating runs, each averaging 12
operations. The benchmark used rustc 1.99.1 on Mac and 1.100.0 on Linux.

| Fixture encoding | Mac ARM64 | Linux x86-64 |
| --- | ---: | ---: |
| ASCII | 0.567 → 0.586 ms | 1.019 → 0.993 ms |
| UTF-8 | 3.557 → 3.615 ms | 5.935 → 5.867 ms |
| Shift_JIS | 3.559 → 3.661 ms | 5.847 → 5.999 ms |
| EUC-JP | 8.988 → 8.849 ms | 14.095 → 14.481 ms |
| GBK | 9.719 → 9.771 ms | 16.081 → 16.705 ms |
| Big5 | 7.349 → 7.356 ms | 12.085 → 12.623 ms |
| EUC-KR | 7.921 → 7.890 ms | 12.891 → 13.295 ms |
| Windows-1252 | 4.206 → 4.319 ms | 7.403 → 7.811 ms |
| Windows-1251 | 4.383 → 4.421 ms | 8.046 → 8.323 ms |

The largest checked median was 16.705 ms. The largest median increase
was 0.62 ms for the Linux GBK fixture. These are complete metadata
parse and filename measurements for the generated inputs.

The latest archive benchmark against libzim covers separate reader
operations. [Pinned reader comparison](../benchmarks/2026-11-19-mac-pinned-reader.md)

## Independent review

The source review compared all 44 changed paths against base `7dc0f31` and the
published archives. The final implementation manifest had SHA-256
`bebedc3ec2585d6ca6fad5af2f840b736b10f11db2c631f0e8722ebc082f713d`.
The reviewer caught incorrect allocation documentation, a reference to a
removed method, and scratch allocation that scaled with the caller's total
spare capacity. The final implementation bounds scratch by the chunk and
decoder state and updates the affected guides.

On re-review, 605,040 streaming calls per implementation matched across
40 encodings, both replacement modes, split byte-order marks, malformed
input and destination sizes up to 1 MiB. Status, progress and output were
compared from the exact reviewed source. The rest of the torrent and
whole-app memory-safety work stays in the inventory.
