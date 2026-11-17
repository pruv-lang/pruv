#!/bin/sh
# Build leg D: verify+compile kernel.pruv with the Rust Pruv compiler and link it with hand.rs into ../work/d/main.
set -e
export PATH="$HOME/.cargo/bin:$PATH"
HERE=$(cd "$(dirname "$0")" && pwd)
OUT="$HERE/../work/d"
mkdir -p "$OUT"
cd "$HERE"
/Users/spica4345/Projects/pruv/pruvc-rs/target/release/pruvc kernel.pruv -o "$OUT/sol.rs"
sed -e "s|@SOL@|$OUT/sol.rs|" -e "s|@HAND@|$HERE/hand.rs|" main.rs > "$OUT/main.rs"
rustc -O -g --edition 2021 "$OUT/main.rs" -o "$OUT/main" 2>&1 | grep -E '^error' -A8 || true
test -x "$OUT/main" && echo "built $OUT/main"
