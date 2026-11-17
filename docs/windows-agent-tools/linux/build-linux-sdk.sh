#!/usr/bin/env bash
# Verify the patched libzim on Linux without root: meson in a venv, -dev packages
# unpacked into a local sysroot, then the repo's own scripts/build-desktop-sdk.py.
# Everything lives under /mnt/e/kiwix-win/linux (E:). Log: linux-sdk.log
set -euo pipefail
L=/mnt/e/kiwix-win/linux
REPO=/mnt/e/kiwix-win/lf-check
mkdir -p "$L/debs" "$L/sysroot"
if [ ! -x "$L/venv/bin/meson" ]; then
  python3 -m venv "$L/venv"
  "$L/venv/bin/pip" install -q meson
fi
export PATH="$L/venv/bin:$PATH"
cd "$L/debs"
for pkg in liblzma-dev libzstd-dev; do
  ls ${pkg}_*.deb >/dev/null 2>&1 || apt download "$pkg"
done
for deb in *.deb; do dpkg-deb -x "$deb" "$L/sysroot"; done
ARCH=x86_64-linux-gnu
LIB="$L/sysroot/usr/lib/$ARCH"
# Runtime libraries the -dev symlinks point at (already installed system-wide).
for so in /usr/lib/$ARCH/liblzma.so.5* /usr/lib/$ARCH/libzstd.so.1*; do cp -P "$so" "$LIB/"; done
for pc in "$LIB"/pkgconfig/liblzma.pc "$LIB"/pkgconfig/libzstd.pc; do
  sed -i -e "s|^prefix=.*|prefix=$L/sysroot/usr|" -e "s|^libdir=.*|libdir=$LIB|" -e "s|^includedir=.*|includedir=$L/sysroot/usr/include|" "$pc"
done
export PKG_CONFIG_PATH="$LIB/pkgconfig"
pkg-config --modversion liblzma libzstd icu-i18n zlib
cd "$REPO"
python3 scripts/build-desktop-sdk.py --jobs 16 \
  --work-dir "$L/sdk-build" --prefix "$L/sdk"
echo "SDK BUILD OK"
