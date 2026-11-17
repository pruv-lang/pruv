#!/bin/sh
# Profile leg B with macOS `sample` (1 ms interval) on the req list no-match input, then print the top self-time frames.
#   sh profile_b.sh [S=20000] [M=10] [secs=6]      (needs run_ab.py to have written work/list_S_M.jsonl)
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
S=${1:-20000}; M=${2:-10}; SECS=${3:-6}
IN="$HERE/work/list_${S}_${M}.jsonl"
OUT="$HERE/work/sample_b_${S}_${M}.txt"
cd "$HERE/../req"
"$HERE/work/b/harness" < "$IN" > /dev/null 2>&1 &
PID=$!
sleep 0.3
sample "$PID" "$SECS" 1 -file "$OUT" > /dev/null 2>&1 || true
wait "$PID" || true
# "Sort by top of stack" section: self samples per frame
awk '/Sort by top of stack/{f=1;next} f&&NF' "$OUT" | head -30
