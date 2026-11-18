# Android direct reader comparison

The Android runner replays the recorded archive requests on a named, task-owned emulator. It uploads standalone measurement executables to a new temporary directory, verifies the input hashes, and checks both the length and SHA-256 of every response against the recorded reference. It removes the temporary directory when finished.

The November 18 run used the ARM64 Android 15/API 35 image with 4 KiB pages and the public Climate change mini and Wikipedia 100 maxi archives. The production Rust reader was `a6d1b4ccd968c29420135fa3b70ced578509a1e8`. A third driver added an experimental fixed-width copy in the Avarok Zstandard decoder. The receipt identifies the patch and modified source by SHA-256. This experiment was isolated from the APK. [Results and binary hashes](../../docs/benchmarks/2026-11-18-android-archive-readers.json)

## Preparation

Use the project’s `scripts/build-android-sdk.py --target aarch64-linux-android` to prepare the historical patched libzim 9.8.2 SDK. Build the existing `native.cpp` benchmark with the NDK API 24 ARM64 C++ compiler and the static library group in the generated manifest. Use the Android NDK path and SDK prefix from your environment.

```bash
tools="$PWD/scripts/bench-archive-readers"
ndkbin="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$NDK_HOST_TAG/bin"
"$ndkbin/aarch64-linux-android24-clang++" -std=c++17 -O3 -DNDEBUG -static-libstdc++ \
  -I"$sdk/include" -L"$sdk/lib" "$tools/native.cpp" -Wl,--start-group \
  -lzim -lxapian -llzma -lzstd -lz -licui18n -licuuc -licudata \
  -lc++_static -lc++abi -lunwind -Wl,--end-group -lm -ldl -llog \
  -Wl,-z,max-page-size=16384 -o "$scratch/native"

export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$ndkbin/aarch64-linux-android24-clang"
cargo build --release --locked --target aarch64-linux-android \
  --manifest-path "$tools/android-sample/Cargo.toml" --target-dir "$scratch/timer"
```

Build the Rust driver from the reader revision under test in a scratch directory, using the layout in the [driver instructions](README.md). The copy of `rust-driver/Cargo.lock` for the original macOS run includes dependencies removed before the Android reader pin. Refresh only the scratch lock with `cargo update --workspace --offline --manifest-path "$scratch/rust-driver/Cargo.toml"` when its pinned packages are cached. Then run the locked release build with the same Android cross target and linker. Save each reader executable separately.

## Running

Supply the expected AVD name and online emulator serial. The collector pins the ADB transport ID for the duration. The archive filenames identify records in the supplied manifest, and their SHA-256 hashes must match. The following command compares two readers. Add `--candidate /path/to/other-reader` to include a third, as in the November run.

```bash
python3 "$tools/android.py" --serial "$emulator_serial" --avd "$avd_name" \
  --timer "$scratch/timer/aarch64-linux-android/release/android-reader-sample" \
  --native "$scratch/native" --rust "$scratch/rust-reader" --rounds 12 \
  --cache-bytes 4194304 --manifest docs/benchmarks/2026-11-18-linux-archive-readers.json \
  --output "$scratch/android-results.json" \
  "$archives/wikipedia_en_climate_change_mini_2024-06.zim" \
  "$archives/wikipedia_en_100_maxi_2024-01.zim"
```

For each phase, the collector discards one warm-up per reader, then runs every ordering equally often. Both readers use a 4 MiB cache target. The native reader defers Xapian loading. Matching inputs and outputs warm the guest filesystem cache. The driver records time spent inside the reader API, including resolving paths and copying output. A separate guest launcher starts one child per sample, waits for it and records guest wall time, user plus system CPU, and peak resident memory. The receipt reports nanoseconds and bytes. The CPU and memory values include child startup, response serialization and exit. Timing excludes transfer over ADB. The measurements describe these archives and the emulator's ARM64 image. Physical phone performance and WebView rendering require separate tests.
