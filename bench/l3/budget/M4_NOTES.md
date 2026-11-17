# M4 performance notes

Machine: shared macOS host, load average 12–50 during the measurements, so single runs swing by up to 2×. All
numbers are medians (5 runs, 200000 reps unless stated). Pruv numbers are `core_ns_per_op` from `driver_m4 perf`
(grow time excluded; no grows happened in the timed loops).

## Finding: the earlier 1.3–2.05× gap was a work-boundary mismatch

The old `perf_ts.mjs` looped over prebuilt key strings and called `Map`/`Set` directly. V8 interns those strings and
caches their hashes, so the TS loop did no op decoding, no validation, no key hashing from units and no result writes.
The Pruv loop does all of these inside `m4_op`. `perf_ts.mjs` now takes the same op word arrays and does the same work
(see `docs/REQUEST_BUDGET_SPEC.md` §9, "Same work boundary"); `node perf_ts.mjs selftest` checks its result words and
invalid-input rejections. The Pruv sources (`bud_store`, `bud_map`, `bud_world`, `root_m4`) are unchanged.

| axis | n | old TS (prebuilt strings) | TS, same work | Pruv core | core / TS (same work) |
|---|---:|---:|---:|---:|---:|
| sessions | 100 | 8.5 | 59.3 | 23.4 | 0.39 |
| sessions | 500 | — | 66.3 | 23.8 | 0.36 |
| sessions | 1000 | 8.0 | 63.8 | 24.6 | 0.39 |
| tools | 100 | 21.2 | 62.9 | 38.1 | 0.61 |
| tools | 500 | — | 64.0 | 41.9 | 0.65 |
| tools | 1000 | 22.5 | 63.2 | 42.4 | 0.67 |
| clocks | 100 | 23.6 | 109.5 | 41.2 | 0.38 |
| clocks | 500 | — | 116.3 | 35.9 | 0.31 |
| clocks | 1000 | 24.3 | 102.8 | 40.4 | 0.39 |

ns per op. `run.py perf --axes sessions,tools,clocks` verdict on two consecutive runs: FLAT, NOT SLOWER (core
n=1000/n=100: sessions 0.99/1.05, tools 1.17/1.11, clocks 0.84/0.98).

The TS cost is dominated by building key strings from the decoded units and hashing them on each call. A faster TS
variant (`String.fromCharCode(a, b, ...)` per key, cheaper integer check) measured the same (53–112 ns/op), so the
result does not depend on a weak TS decoder.

## Where the Pruv time goes (n = 1000)

`sample` profile of a driver that replays one op cycle (top of stack): `bud_map::find` ~30%, `m4_op` with the inlined
decoders and `key_end` ~20–25%, `bud_store::valid` ~10–15%, the rest in `add`/`del`/`fill_entry`/`put_units`/`need`/
`make_room`.

Ablation (1M reps, interleaved, median of 5; variants only in scratch builds):

| axis | base | without `valid()` | without the key-unit range scan |
|---|---:|---:|---:|
| sessions | 23.0 | 20.2 | 21.5 |
| tools | 41.2 | 38.6 | 38.3 |
| clocks | 41.1 | 38.6 | 39.6 |

So the O(1) header check costs about 2.6 ns per op and the unit range scan 1.5–3 ns. Both are required by the
contract (header validation on every entry, units 0..65535) and stay.

Other checks:

- Total accessors (`ent_u16::rd/wr`) compile to one bounds test plus an unchecked load; no double check. Their
  cost is measured separately below.
- Hash quality: simulated linear-probe length 1.13–1.38 slots per lookup at the measured fill (2048/4096 slots).
- The remaining `find` cost is the multiply chain of the hash and the dependent loads slot → entry → arena key;
  header words (slot count, slot base, entry base, capacity) are re-read and clamped inside the probe loop. Hoisting
  them would save a few ns at most and is not needed for the verdict.

No compiler change is needed for M4.

## Range-check cost (cause candidate (a))

What the generated code checks today (`work/m34/gen_m4.rs`, pruvc at main `ea1f146`):

- Every index the verifier proved in range is emitted as `get_unchecked` / `get_unchecked_mut`. `otool -tV` on the
  optimised driver finds **0** `panic_bounds_check` call sites in the generated module (the 45 in the binary are 1 in
  the Rust driver and 44 in std). With plain `a[i]` in the two accessors instead (variant `safeidx`) the generated
  module has 101.
- The remaining range tests are the source-level guards of the total accessors `ent_u16::rd(a, i)` /
  `ent_u16::wr(a, i, v)`: `if i < 0 || i >= len(a) { return 0 / return }`. They are library semantics (a total
  function), not compiler bounds checks, and there are 96 call sites in the M4 module. Every store access in
  `bud_store` / `bud_map` / `bud_world` goes through them.

Variants (scratch only, same generated module otherwise, same driver):

- `base`: as generated.
- `noguard`: both guards removed (load/store through `get_unchecked` directly). This is the upper bound of a
  compiler change that drops the guard at call sites where the index is proven in range.
- `safeidx`: guards replaced by Rust `a[i]` (panicking bounds check), i.e. what plain checked indexing would cost.
- `trap`: guards replaced by `abort()`; run on every perf axis and size, it never aborted, so on these runs `noguard`
  executes the same accesses as `base`.

All variants give the same checksum (`sink` of the result words) on every run. Wall time was unusable during this
measurement (load average 46–128), so the table uses the process counters of `/usr/bin/time -l`, differenced between
100000 and 500000 reps so prefill, encode and process start cancel out (driver patched to encode one op cycle only);
median of 5.

| axis | n | instr/op base | noguard | safeidx | cycles/op base | noguard | safeidx |
|---|---:|---:|---:|---:|---:|---:|---:|
| sessions | 100 | 741.1 | 604.3 | 849.2 | 87.5 | 68.8 | 101.7 |
| sessions | 1000 | 770.7 | 625.4 | 871.6 | 112.5 | 97.6 | 137.3 |
| tools | 100 | 1064.2 | 815.6 | 1022.5 | 172.1 | 143.6 | 179.8 |
| tools | 1000 | 1137.2 | 868.8 | 1084.1 | 211.7 | 172.8 | 198.3 |
| clocks | 100 | 1180.0 | 908.1 | 1120.5 | 200.8 | 160.6 | 175.1 |
| clocks | 1000 | 1169.2 | 899.7 | 1108.8 | 167.8 | 143.1 | 171.0 |

Removing the accessor guards cuts 18–24% of the instructions and 13–21% of the cycles per op. Checked `a[i]` costs
about the same as the guard (−5% to +14% instructions), so the guard is effectively a bounds check paid on every store
access. Interleaved wall-time runs taken earlier at lower load agree in direction (paired median ratio noguard/base
0.63–0.98) but are too noisy to quote.

Options, none applied here (compiler untouched, M4 Pruv sources unchanged):

1. Compiler: at a `rd`/`wr` call (or any total accessor of this shape) whose index the verifier has proven in
   `0 <= i < len(a)`, emit the unchecked access instead of the guarded call. The gain is bounded by the `noguard`
   column; how much of it is reachable depends on how many of the 96 call sites have the range fact in scope.
2. Pruv source: in the hot paths (`find`, `entry_matches`, `slot_pos`, `valid`, the decoders) replace `rd`/`wr` by
   direct `st[i]` where the bound can be proven; the compiler already emits those unchecked. This needs proof work
   (header validity implying the region bounds) and is not needed for the M4 verdict.
