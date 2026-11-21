#!/bin/sh
set -eu
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")"
mkdir -p work/hand
# Reuse the dev build's dependency cache.
cargo build --offline --locked --release --manifest-path host/Cargo.toml --target-dir work/dev/host -q
DEPS=work/dev/host/release/deps
SERDE=$(ls -t "$DEPS"/libserde_json-*.rlib | head -1)
rustc -O --edition 2021 hand.rs --extern "serde_json=$SERDE" -L "dependency=$DEPS" -o work/hand/harness
