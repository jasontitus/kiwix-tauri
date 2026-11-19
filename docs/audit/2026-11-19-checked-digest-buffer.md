# Checked digest buffering

The app uses a local RustCrypto `block-buffer` 0.12.1 for the modern digest stack. The buffer now holds an initialized array and uses checked slices to store partial blocks, pad messages, and read or update the cursor. This removes the crate's raw copies, unchecked cursor reads and unchecked initialization. The crate forbids unsafe Rust. The app's SHA-1 web-seed piece checks and its RustCrypto SHA-256 paths use this buffer. The app's lower-level CPU selection and generic array machinery continue to provide acceleration and zero-copy access to bulk input. [Upstream archive and license information](../../src-tauri/licenses/block-buffer/SOURCE.txt)

The changed code preserves the eager cursor in the last byte, the lazy cursor's separate byte, and the read buffer's first-byte cursor. A lazy buffer can retain a full block. The format still writes only the valid prefix or suffix and checks serialized padding. A guard resets the cursor if a compression, generation or read callback panics. The optional zeroization feature clears backing bytes and the cursor. The public buffer methods and formats preserve tested behavior. Two undocumented inherited cursor helpers now require an initialized array, and the setter accepts the cursor as a byte. The repository has no callers of those helpers outside this crate.

## Independent review and tests

The review covered 21 changed or new paths on `9a944382ead837aa78e41aed8bd61c49c4c543a4`. The sorted SHA-256 and path manifest of those files hashes to `d9ca071b257175b956a268de007cc36d6b719a9aeebc2b7ea12df34f52ef0319`. The published archive and both original licenses accompany the local copy. The reviewer found that the first zeroization test could pass without wiping the stored bytes because serialization suppresses that storage after reset. The revised test directly checks every initialized backing byte and both cursors. The reviewer verified the correction and re-ran the tests.

An independent differential compared the published and local buffers across 312,022 position and input-length combinations at 12 block sizes. It checked callback batches, padding, serialized states and read-cache behavior. The vendored crate passed one unit test, 12 integration tests and three documentation tests with all features. Miri passed the same tests under its default and Tree Borrows modes using the app's `hybrid-array` 0.4.15 and `zeroize` 1.9.0. These results cover the tested executions.

The SHA-256 differential matched the pinned reference with ARM acceleration, forced software and Ubuntu's native software path. A separate 32-bit ARM Linux test binary ran under QEMU 8.2.2. It matched three independent digest values immediately before, at and after the 4 GiB boundary. The emulator result covers the 32-bit counter and software digest path.

The optimized Rust app library tests passed 1,259 cases on macOS with 25 ignored and 1,242 on Ubuntu with 16 ignored. The app library passed compile checks for iOS ARM64 and its ARM64 simulator, Android ARM64 and Android ARMv7. The final source reviews cover two test-only changes made after freezing build inputs. The tested runtime code remains equivalent.

The ad-hoc signed Mac app accepted both real archives and rejected the incorrect advertised checksums without publishing those files. It reopened the earlier build's download records. The Ubuntu AppImage and bare executable matched the recorded archive service outputs for single and split archives, subsequent opens, and a ranged image. The Mac package and Ubuntu AppImage contained the three block-buffer notice files byte for byte.

## Performance

The checked and published buffers used the same local SHA-256 implementation in the in-memory Mac run. The 111-byte digest took a median 244.98 ns with the published buffer and 244.72 ns with the checked buffer. The two preloaded archives took 7.54 and 31.04 ms with the published buffer and 7.52 and 30.59 ms with the checked buffer.

The app's 256 MiB loopback download used four range requests per run. The median elapsed time went from 1.355 to 1.380 s on macOS and from 1.700 to 1.710 s on Ubuntu. Median CPU time went from 0.745 to 0.735 s on macOS and from 1.675 to 1.660 s on Ubuntu. The signed Mac app's median completion times for two real downloads went from 222.60 to 234.75 ms and from 311.39 to 316.53 ms in six alternating pairs per archive. These timings measure local fixtures.

The signed Mac app matched the earlier app and the historical libzim reference on previews and normalized MHTML outputs. Median preview time was 4.87 ms for the checked build and 5.02 ms for libzim. Median MHTML export time was 26.54 and 32.31 ms, respectively, across 12 exports per build. All checked and native timings in this reader sample stayed below 50 ms. The reader timings cover the selected local commands and archives. [Samples, inputs and package digests](../benchmarks/2026-11-19-checked-digest-buffer-app.json)

## Remaining memory boundaries

The local buffer calls `hybrid-array` to construct initialized arrays and obtain zero-copy block views. That dependency still uses unchecked initialization internally and casts slices between component bytes, transparent array wrappers and native arrays. For the checked paths, the size mapping fixes the array length. The block count is the input length divided by a positive block size. The wrapped block and tail views borrow disjoint parts of the original storage. They do not outlive it. The SHA compressors receive whole blocks. These representation and initialization operations remain under review. Hardware feature selection and the separate older digest buffer also remain in scope.

The main `librqbit` torrent engine still uses native AWS-LC through `librqbit-sha1-wrapper`. Its SHA-1 implementation remains a separate boundary. [Native dependency inventory](../DEPENDENCIES.md)

```sh
cargo test --locked --release --manifest-path src-tauri/vendor/block-buffer-0.12.1/Cargo.toml --all-features
cargo test --locked --release --manifest-path scripts/check-sha256-loads/Cargo.toml -- --include-ignored
RUSTFLAGS='--cfg sha2_256_backend="soft"' cargo test --locked --release --manifest-path scripts/check-sha256-loads/Cargo.toml -- --include-ignored
```
