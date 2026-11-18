#!/usr/bin/env bash
# Fast local iteration for the Kiwix workbench.
#
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
