#!/usr/bin/env bash
# Run a bounded cargo-fuzz target: zim_archive (default), search_index or mcp_protocol.
# Requires nightly Rust, `cargo install cargo-fuzz` and python3 (seed extraction).
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
mkdir -p fuzz/corpus/zim_archive fuzz/corpus/mcp_protocol fuzz/corpus/search_index
cp tests/fixtures/*.zim fuzz/corpus/zim_archive/
cp fuzz/seeds/mcp_protocol/* fuzz/corpus/mcp_protocol/
# search_index takes one embedded Glass index as its whole input.
python3 fuzz/extract-index-seeds.py tests/fixtures/indexed_search.zim fuzz/corpus/search_index
cargo fuzz run "$target" -- -runs="$runs" -max_total_time="$seconds"
