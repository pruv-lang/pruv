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

- Total accessors (`ent_u16::rd/wr`) compile to one bounds test plus an unchecked load; no double check.
- Hash quality: simulated linear-probe length 1.13–1.38 slots per lookup at the measured fill (2048/4096 slots).
- The remaining `find` cost is the multiply chain of the hash and the dependent loads slot → entry → arena key;
  header words (slot count, slot base, entry base, capacity) are re-read and clamped inside the probe loop. Hoisting
  them would save a few ns at most and is not needed for the verdict.

No compiler change is needed for M4.
