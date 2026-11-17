#!/bin/sh
# Dev loop for the L3-10 budget host (M5): compile the M4 Pruv root with the Rust Pruv compiler, link it with the
# host crate budget_host, and leave the runner at work/m5build/harness.
# Usage: sh dev_build.sh      then:  work/m5build/harness q --prog <program.jsonl> < scenarios.jsonl
#                                    work/m5build/harness perf <n> <reps>
# Env: PRUVC overrides the compiler path.
set -e
export PATH="$HOME/.cargo/bin:$PATH"
HERE=$(cd "$(dirname "$0")" && pwd)
OUT="$HERE/work/m5build"
PRUVC="${PRUVC:-$HERE/../../../pruvc-rs/target/release/pruvc}"
mkdir -p "$OUT"
cd "$HERE"
if ! "$PRUVC" m34_test/root_m4.pruv -o "$OUT/sol.rs" --lib "$HERE" --lib "$HERE/../lib"; then
  echo "dev_build: pruvc failed on m34_test/root_m4.pruv (compiler: $PRUVC; set PRUVC to override)" >&2
  exit 1
fi
cargo build --offline --locked --release --manifest-path host/Cargo.toml --target-dir "$OUT/host" -q
DEPS="$OUT/host/release/deps"
LIB=$(ls -t "$DEPS"/libbudget_host-*.rlib | head -1)
sed "s|@SOL@|$OUT/sol.rs|" harness.rs > "$OUT/main.rs"
if ! rustc -C opt-level=3 --edition 2021 "$OUT/main.rs" -o "$OUT/harness" --extern "budget_host=$LIB" -L "dependency=$DEPS" 2> "$OUT/rustc.log"; then
  grep -v '^warning' "$OUT/rustc.log" | head -80 >&2
  echo "dev_build: rustc failed (full log: $OUT/rustc.log)" >&2
  exit 1
fi
test -x "$OUT/harness"
echo built
