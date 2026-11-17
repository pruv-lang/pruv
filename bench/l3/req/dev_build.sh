#!/bin/sh
# Dev loop for the L3-9 request handlers: verify+compile root.pruv with the Rust Pruv compiler (rs/rust only),
# link it with the host crate, and leave the runner at work/dev/harness.
# Usage: sh dev_build.sh [--no-link]    then:  work/dev/harness < cases.jsonl
set -e
export PATH="$HOME/.cargo/bin:$PATH"
HERE=$(cd "$(dirname "$0")" && pwd)
OUT="$HERE/work/dev"
mkdir -p "$OUT"
cd "$HERE"
/Users/spica4345/Projects/pruv/pruvc-rs/target/release/pruvc root.pruv -o "$OUT/sol.rs" --lib "$HERE/../lib"
[ "$1" = "--no-link" ] && exit 0
cargo build --offline --locked --release --manifest-path host/Cargo.toml --target-dir "$OUT/host" -q
DEPS="$OUT/host/release/deps"
LIB=$(ls -t "$DEPS"/libreq_host-*.rlib | head -1)
sed "s|@SOL@|$OUT/sol.rs|" harness.rs > "$OUT/main.rs"
rustc -O --edition 2021 "$OUT/main.rs" -o "$OUT/harness" --extern "req_host=$LIB" -L "dependency=$DEPS" 2>&1 | grep -v '^warning' | grep -E 'error' -A5 || true
test -x "$OUT/harness" && echo "built $OUT/harness"
