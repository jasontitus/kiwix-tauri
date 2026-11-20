#!/usr/bin/env bash
# Build and run the LocalSwarm Objective-C++ bridge under ASan+UBSan.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
clang++ -std=c++17 -fobjc-arc -fblocks \
  -fsanitize=address,undefined -fno-omit-frame-pointer \
  -I "$root/src-tauri" \
  -DLOCALSWARM_BRIDGE_SOURCE='"native/localswarm.mm"' \
  "$root/src-tauri/native/localswarm_tests.mm" \
  -framework Foundation -framework Network -framework Security -framework CFNetwork \
  -o "$out/localswarm_tests"
ASAN_OPTIONS=detect_leaks=0 UBSAN_OPTIONS=halt_on_error=1 "$out/localswarm_tests"
