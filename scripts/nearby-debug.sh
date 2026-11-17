#!/bin/sh
# Stream Nearby/LocalSwarm diagnostics from both phones.
#
# Android: Kotlin logs under the KiwixNearby tag and Rust `[nearby]` lines
# forwarded by RustStdoutStderr.
# iOS: the app appends `[nearby]` lines to a bounded file in its data
# container. Copy it after a run (or use --console for a live launch).
set -eu
UDID="${KIWIX_IPHONE_UDID:-}"
BUNDLE="${KIWIX_BUNDLE_ID:-com.tiltastech.zimreader}"
LOG="Library/Application Support/$BUNDLE/nearby-debug.log"

pull_ios() {
  : "${UDID:?Set KIWIX_IPHONE_UDID to the explicit paired iPhone UDID}"
  xcrun devicectl device copy from --device "$UDID" \
    --domain-type appDataContainer --domain-identifier "$BUNDLE" \
    --source "$LOG" --destination /tmp/nearby-ios.log >/dev/null
  tail -n 80 /tmp/nearby-ios.log
}

case "${1:-stream}" in
  pull)
    pull_ios
    ;;
  *)
    echo "iOS log pull (after a run):  KIWIX_IPHONE_UDID=<explicit-iphone-udid> sh scripts/nearby-debug.sh pull"
    echo "iOS live console (another terminal):"
    echo "  xcrun devicectl device process launch --console --terminate-existing --device ${UDID:-<explicit-iphone-udid>} $BUNDLE"
    echo
    echo "Android (this terminal, Ctrl+C to stop):"
    adb logcat -v time -s KiwixNearby RustStdoutStderr | awk '{ print; fflush() }'
    ;;
esac
