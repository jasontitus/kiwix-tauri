#!/usr/bin/env bash
# Run libzim's own test suite on Linux: fresh pinned source + the repo's current
# split-ranges and retained-fd patches, against the patched Xapian from the SDK build.
set -euo pipefail
L=/mnt/e/kiwix-win/linux
REPO=/mnt/e/kiwix-win/lf-check
export PATH="$L/venv/bin:$PATH"
ARCH=x86_64-linux-gnu
export PKG_CONFIG_PATH="$L/sdk/lib/pkgconfig:$L/sysroot/usr/lib/$ARCH/pkgconfig"
export LD_LIBRARY_PATH="$L/sdk/lib:$L/sysroot/usr/lib/$ARCH"
SRC="$L/zim-src"
BUILD="$L/zim-tests"
rm -rf "$SRC" "$BUILD"
mkdir -p "$SRC"
tar -xzf /mnt/e/kiwix-win/downloads/libzim-26ec526f74e8342a40da5ab14e364546988e0e1a.tar.gz -C "$SRC" --strip-components=1
for p in libzim-split-ranges.patch libzim-retained-fd.patch; do
  patch --batch --fuzz=0 -p1 -d "$SRC" --input "$REPO/src-tauri/native/$p"
done
ZTS=$(ls -d /mnt/e/kiwix-win/kb/SOURCE/zim-testing-suite* | head -1)
meson setup "$BUILD" "$SRC" --buildtype=release --default-library=shared \
  -Dwith_xapian=true -Dwerror=false -Dexamples=false -Ddoc=false -Dtests=true \
  -Dtest_data_dir="$ZTS"
meson compile -C "$BUILD" -j 16
SKIP_BIG_MEMORY_TEST=1 meson test -C "$BUILD" --print-errorlogs -t 8
echo "LINUX LIBZIM TESTS OK"
