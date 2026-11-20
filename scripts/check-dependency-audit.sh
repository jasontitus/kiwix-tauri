#!/usr/bin/env bash
# Check the app, relay and shared-crate lockfiles against the cached RustSec DB.
# Requires `cargo install cargo-audit`; use `cargo audit update-db` first when online.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
for lock in src-tauri/Cargo.lock kiwix-mcp/Cargo.lock win-security/Cargo.lock; do
  echo "== $lock"
  cargo audit --no-fetch --file "$lock"
done
