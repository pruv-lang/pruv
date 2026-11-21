#!/bin/sh
# Build the WA forge harness (forge.rs) next to work/dev/harness; run dev_build.sh first.
set -e
export PATH="$HOME/.cargo/bin:$PATH"
HERE=$(cd "$(dirname "$0")" && pwd)
OUT="$HERE/work/dev"
DEPS="$OUT/host/release/deps"
LIB=$(ls -t "$DEPS"/libentry_host-*.rlib | head -1)
SJ=$(ls -t "$DEPS"/libserde_json-*.rlib | head -1)
sed -e "s|@SOL@|$OUT/sol.rs|" -e "s|@FORGE@|$HERE/forge.rs|" "$HERE/harness.rs" > "$OUT/forge_main.rs"
rustc --cfg entry_forge -O --edition 2021 "$OUT/forge_main.rs" -o "$OUT/forge_harness" --extern "entry_host=$LIB" --extern "serde_json=$SJ" -L "dependency=$DEPS" 2>&1 | grep -E '^error' -A8 || true
test -x "$OUT/forge_harness" && echo "built $OUT/forge_harness"
