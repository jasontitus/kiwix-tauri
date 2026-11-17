#!/usr/bin/env bash
# Fast local iteration for the Kiwix workbench.
#
# The desktop build needs the patched libzim/Xapian SDK, the rustup toolchain
# (Homebrew's rustc has no Android/iOS targets), and enough cargo jobs to use
# the host. This wrapper resolves all three once and exposes the common loops.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$HOME/.cargo/bin:$PATH"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-10}"
export SCCACHE_CACHE_SIZE="${SCCACHE_CACHE_SIZE:-50G}"
export JAVA_HOME="${JAVA_HOME:-$HOME/Library/Java/JavaVirtualMachines/jbr-17.0.14/Contents/Home}"
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
export NDK_HOME="${NDK_HOME:-$ANDROID_HOME/ndk/27.0.12077973}"

host="$(rustc -vV | sed -n 's/^host: //p')"
cmd="${1:-check}"
shift || true

need_sdk=0
case "$cmd" in
    check|test|nextest|build|run|bacon|doctor) need_sdk=1 ;;
esac

if [[ "$need_sdk" == 1 ]]; then
    if [[ -z "${KIWIX_DESKTOP_SDK_ROOT:-}" ]]; then
        for candidate in \
            "$REPO/.native/desktop-retained-v2/$host" \
            "$REPO/.native/desktop-retained/$host" \
            "$REPO/../desktop-retained-v2/$host" \
            "$REPO/../desktop-retained/$host"; do
            if [[ -f "$candidate/kiwix-sdk.json" ]]; then
                export KIWIX_DESKTOP_SDK_ROOT="$(cd "$candidate" && pwd)"
                break
            fi
        done
    fi
    if [[ -z "${KIWIX_DESKTOP_SDK_ROOT:-}" || ! -f "$KIWIX_DESKTOP_SDK_ROOT/kiwix-sdk.json" ]]; then
        echo "error: no patched desktop SDK for $host." >&2
        echo "  build it with: python3 scripts/build-desktop-sdk.py" >&2
        echo "  or set KIWIX_DESKTOP_SDK_ROOT to a directory containing kiwix-sdk.json" >&2
        exit 1
    fi
    export DYLD_LIBRARY_PATH="$KIWIX_DESKTOP_SDK_ROOT/lib${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}"
fi

case "$cmd" in
    check)
        cd "$REPO/src-tauri"
        exec cargo check --lib "$@"
        ;;
    test)
        cd "$REPO/src-tauri"
        exec cargo test --lib "$@"
        ;;
    nextest)
        cd "$REPO/src-tauri"
        exec cargo nextest run --lib "$@"
        ;;
    build)
        cd "$REPO/src-tauri"
        exec cargo build --bin kiwix-workbench "$@"
        ;;
    run)
        if [[ "${1:-}" == "--" ]]; then shift; fi
        cd "$REPO"
        exec npm run desktop -- "$@"
        ;;
    bacon)
        cd "$REPO/src-tauri"
        exec bacon "$@"
        ;;
    ios)
        if [[ "${1:-}" == "--" ]]; then shift; fi
        cd "$REPO"
        exec npm run iphone -- "$@"
        ;;
    android-debug)
        if [[ "${1:-}" == "--" ]]; then shift; fi
        cd "$REPO"
        exec npm run tauri -- android build --debug --target aarch64 --apk "$@"
        ;;
    android)
        if [[ "${1:-}" == "--" ]]; then shift; fi
        cd "$REPO"
        exec npm run tauri -- android build --target aarch64 --apk --ci "$@"
        ;;
    doctor)
        echo "host:        $host"
        echo "sdk:         $KIWIX_DESKTOP_SDK_ROOT"
        echo "cargo:       $(cargo -V)"
        echo "rustc:       $(rustc -V)"
        echo "jobs:        $CARGO_BUILD_JOBS"
        echo "sccache:     $(sccache --version 2>/dev/null || echo missing)"
        echo "bacon:       $(command -v bacon || echo missing)"
        echo "nextest:     $(command -v cargo-nextest || echo missing)"
        sccache --show-stats 2>/dev/null | grep -E 'Cache hits|Cache misses|Max cache size' || true
        ;;
    *)
        echo "usage: scripts/dev.sh <command> [args]" >&2
        echo "  check              cargo check --lib (fastest Rust feedback)" >&2
        echo "  test [filter]      cargo test --lib" >&2
        echo "  nextest [filter]   cargo nextest run --lib (faster test runner)" >&2
        echo "  build              debug app binary" >&2
        echo "  run                tauri dev (frontend HMR + debug Rust)" >&2
        echo "  bacon              auto-check on save" >&2
        echo "  ios [args]         scripts/iphone.py (release device build)" >&2
        echo "  android-debug      debug APK (fast device loop)" >&2
        echo "  android            release APK" >&2
        echo "  doctor             print resolved toolchain and cache state" >&2
        exit 2
        ;;
esac
