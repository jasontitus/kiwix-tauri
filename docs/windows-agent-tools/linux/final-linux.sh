#!/usr/bin/env bash
# Final Linux verification of the current patch: libzim tests, then the repo's SDK recipe from scratch.
set -uo pipefail
L=/mnt/e/kiwix-win/linux
bash "$L/test-linux-libzim.sh" > "$L/final-test.log" 2>&1; echo "tests=$?" > "$L/final.status"
rm -rf "$L/sdk-build" "$L/sdk"
bash "$L/build-linux-sdk.sh" > "$L/final-sdk.log" 2>&1; echo "sdk=$?" >> "$L/final.status"
echo done >> "$L/final.status"
