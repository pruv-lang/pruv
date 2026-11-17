#!/bin/sh
# Build leg B (bench/l3/req as committed) and leg C (a copy with the integrity work stubbed) into work/b and work/c.
# Leg C changes exactly two functions of the copied req_world.pruv (see patch_c.py):
#   dig(a, skip)  -> returns 1 without reading a  (every digest, seal and chain-digest check then compares 1 == 1)
#   world_ok(w)   -> returns true                 (no per-step world validation walk)
# Packet building, world re-encoding and the per-step bundle copy stay (they are the step ABI itself); their share comes
# from the profile (profile_b.sh). Both legs link the request host rlib that bench/l3/req/dev_build.sh already built
# (read only), so run `sh ../req/dev_build.sh` first if work/dev/host is missing.
# Usage: sh build_bc.sh [b|c|bc]   (default bc)
set -e
export PATH="$HOME/.cargo/bin:$PATH"
HERE=$(cd "$(dirname "$0")" && pwd)
REQ="$HERE/../req"
PRUVC=/Users/spica4345/Projects/pruv/pruvc-rs/target/release/pruvc
DEPS="$REQ/work/dev/host/release/deps"
LIB=$(ls -t "$DEPS"/libreq_host-*.rlib | head -1)
WHICH=${1:-bc}
for leg in b c; do
  case "$WHICH" in *$leg*) ;; *) continue ;; esac
  OUT="$HERE/work/$leg"
  rm -rf "$OUT/src"; mkdir -p "$OUT/src"
  cp "$REQ"/*.pruv "$OUT/src/"
  [ "$leg" = c ] && python3 "$HERE/patch_c.py" "$OUT/src/req_world.pruv"
  (cd "$OUT/src" && "$PRUVC" root.pruv -o "$OUT/sol.rs" --lib "$REQ/../lib")
  sed "s|@SOL@|$OUT/sol.rs|" "$REQ/harness.rs" > "$OUT/main.rs"
  # -g keeps symbols and line tables for the profiler; optimisation is the same -O as dev_build.sh
  rustc -O -g --edition 2021 "$OUT/main.rs" -o "$OUT/harness" --extern "req_host=$LIB" -L "dependency=$DEPS" 2>&1 | grep -E '^error' -A5 || true
  test -x "$OUT/harness" && echo "built $OUT/harness"
done
