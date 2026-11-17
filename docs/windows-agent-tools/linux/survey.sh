#!/usr/bin/env bash
head -2 /etc/os-release
python3 --version
for t in cc c++ make meson ninja pkg-config patch curl; do printf '%s: ' "$t"; command -v "$t" || echo MISSING; done
for p in icu-i18n liblzma libzstd zlib gtest; do printf '%s: ' "$p"; pkg-config --modversion "$p" 2>/dev/null || echo MISSING; done
if sudo -n true 2>/dev/null; then echo SUDO_OK; else echo SUDO_NEEDS_PASSWORD; fi
nproc
df -h /mnt/e | tail -1
