#!/bin/sh
# Build the L3-9 forge harness (forge.rs) next to work/dev/harness; run dev_build.sh first.
set -e
export PATH="$HOME/.cargo/bin:$PATH"
HERE=$(cd "$(dirname "$0")" && pwd)
OUT="$HERE/work/dev"
DEPS="$OUT/host/release/deps"
LIB=$(ls -t "$DEPS"/libreq_host-*.rlib | head -1)
SJ=$(ls -t "$DEPS"/libserde_json-*.rlib | head -1)
sed "s|@SOL@|$OUT/sol.rs|" "$HERE/forge.rs" > "$OUT/forge_main.rs"
rustc -O --edition 2021 "$OUT/forge_main.rs" -o "$OUT/forge_harness" --extern "req_host=$LIB" --extern "serde_json=$SJ" -L "dependency=$DEPS" 2>&1 | grep -E '^error' -A8 || true
test -x "$OUT/forge_harness" && echo "built $OUT/forge_harness"
