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

### Source fast paths (L3-10 follow-up)

The hot state code now takes one range check per run of accesses and uses plain `a[i]` inside it. The verifier
proves those accesses, so they compile to `get_unchecked`. When the check fails, the original `rd`/`wr` code runs, so
every buffer (short or corrupt included) gives the same result as before. The pattern:

```
if o + n <= len(st) && s0 + n <= len(src) {
  while k < n invariant 0 <= k && k <= n && 0 <= o && o + n <= len(st) && 0 <= s0 && s0 + n <= len(src) ... {
    st[o + k] = src[s0 + k]; k = k + 1;
  }
} else { /* original wr/rd loop */ }
```

Sites: `hash_key`, `entry_matches`, `write_at`, `slot_pos`, `fits_region`, `blob_off` (`bud_map`), `blob_out`,
`ret_one`, `session_get` (`bud_world`), `ea`, `need`, `valid` (`bud_store`). Sessions went from 739/768 to 645/668
instructions per op (n = 100/1000) and from 1.51–1.60× to 1.30–1.40× hand Rust in wall time
(`perf_headroom/HOST.md`). No compiler change.

## Against hand-written Rust (perf headroom)

`perf_headroom/HOST.md` adds a std-only hand Rust port of the same `exec()` (`perf_headroom/host/src/bin/m4_hand.rs`),
with the same timed loop as `driver_m4 perf`. Median Pruv/hand over 5 alternating rounds: sessions 1.51–1.60, tools
0.69–0.75, clocks 0.86–0.98.

Sessions is over the 1.5× bar. With the `rd`/`wr` accessor guards removed (the `noguard` variant above), it is 1.17–1.24×.
So the cause is the guards, and the fix is compiler option 1. The host cannot remove them.

### Under the 1.1x bar (2026-11-17)

More fast paths in `root_m4.pruv` (`key_end`, decoders), `bud_map` (`find`, `set_blob`, `blob_fits`) and
`bud_world::session_get` brought sessions to 1.22 / 1.17 / 1.16x (n = 100 / 500 / 1000), from 1.40 / 1.30 / 1.30.
The rest is no inlining of the generated call chain (compiler) plus the per-op `valid()` header check (raw-buffer
ABI). With both removed in a scratch build, it is at parity. Line-by-line diagnosis: `perf_headroom/HOST.md`, "M4
sessions under the 1.1x bar".

### Pass 3: inline hints and `eq_range` (2026-11-17)

The compiler now emits `#[inline]` on generated functions and `bud_map::entry_matches` uses the new `eq_range`
builtin (slice `==`, `memcmp` in the binary). Sessions 1.22 / 1.13 / 1.14x, tools 0.69-0.74, clocks 0.92-1.04.
The rest is the per-op `valid()` of the raw-buffer ABI; the typed-handle design that removes it is in
`perf_headroom/HOST.md`, "Design: a typed resident-state handle". Checks: 300/300, stats 1922/1922.

### Map hash, probe and key compare (2026-11-17, input from the M9 profile)

M9 found `bud_map::find` at 37% of its clock calls. Its 36-unit budget ids went through one dependent multiply per
unit. The changes in `bud_map.pruv` (public signatures unchanged):

- **Hash.**
  - `hash_key` now packs 4 key units into one word per `mix`.
  - `mix` multiplies by the 64-bit golden-ratio constant.
  - Owner and owner generation share one word.
  - The key length goes in by xor.
  - A splitmix64 finalizer `fin` follows.
  - Dependent multiplies per lookup: 2 + n before, 1 + ceil(n/4) + 1 now. For a 36-unit id that is 38 → 11.
  - The finalizer is needed, not optional. A multiply only carries bits upward, so with packed units and the old FNV
    prime, the high units never reached the slot mask. A first try without it gave 43-50 average probes per key at
    n = 1000 (Python model of the probe table, keys `s{i}` / `a{i}` / `c{i}`), and M4 sessions at n = 1000 went
    to 4.1x.
  - With `fin`, the model gives 1.14-1.20 probes per key: the same as random 64-bit hashes (1.13-1.15) and the old
    hash (1.13-1.15).
  - Every add / find / remove / rehash path calls `hash_key`, and entries store its result (field 6), so they stay
    consistent. `grow_copy` re-adds through `add`.
  - This is a change of the map format. A state buffer built by an older build would not be found by the new hash.
    Buffers are not persisted across builds in L3-10.
- **Probe.**
  - `find` already reads the slot base and mask once (fast path above).
  - `put_key` (`insert` / `add`) now does the same instead of calling `slot_pos` per probe step.
  - `free_slot` / `slot_of` (remove path) still use `slot_pos`. `slot_pos` itself reads header words 3 and 4 directly
    after one length check.
- **Key compare.**
  - `entry_matches` already has one range check, then a plain indexed loop (`get_unchecked` in the generated Rust).
  - M9's measurement used the per-unit `rd` loop, which is now only the fallback.
  - The release asm of the M4 build (`rustc -C opt-level=3 --emit asm`) has no `memcmp` / `bcmp` call. The loop
    stays a scalar early-exit compare of i64 words. LLVM's loop-idiom pass turns plain copy and fill loops into
    `memcpy` / `memset`, but not an early-exit compare loop into `bcmp`.
  - The hand code compares `&[u8]` / `&str` with `==`, which calls `memcmp` on 1 byte per unit. Pruv keys are i64
    words, 8 bytes per u16 unit.
  - The fix is a lib or compiler slice-equality primitive. Pruv source can state the bounds once (it does) but cannot
    emit `a[x..x+n] == b[y..y+n]`. This was not changed here (the compiler and lib are out of scope).

Results are unchanged: M4 300/300 (stats checks 1922/1922 ok), M5 exact 300 / unexplained 0, M11 exact 213 /
prefix-ok 87 (py and rs identical).

M4 perf: M4 keys are 2-5 units, so there is little to save. In alternating A/B runs (old hash vs new, 5 rounds each,
machine load average 30-200 during the runs), Pruv sessions were 19.5-20.3 ns for both and clocks 39-51 for both.
The difference is inside the noise. The M9 gain (36-unit ids) is for the M9 sub to measure on its driver.
