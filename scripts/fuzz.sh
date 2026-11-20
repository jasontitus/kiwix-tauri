#!/usr/bin/env bash
# Run a bounded cargo-fuzz target. Requires nightly Rust and `cargo install cargo-fuzz`.
set -euo pipefail
target=${1:-zim_archive}
runs=${RUNS:-100000}
seconds=${MAX_TOTAL_TIME:-30}
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root/src-tauri"
toolchain=$(dirname "$(rustup which --toolchain nightly rustc)")
export PATH="$HOME/.cargo/bin:$toolchain:$PATH"
export RUSTC
RUSTC=$(rustup which --toolchain nightly rustc)
mkdir -p fuzz/corpus/zim_archive fuzz/corpus/mcp_protocol
cp tests/fixtures/*.zim fuzz/corpus/zim_archive/
cp fuzz/seeds/mcp_protocol/* fuzz/corpus/mcp_protocol/
cargo fuzz run "$target" -- -runs="$runs" -max_total_time="$seconds"
