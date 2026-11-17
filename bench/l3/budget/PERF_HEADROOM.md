# Request-budget piece: performance headroom against hand-written Rust

Rule (2026-11-17): beating TS is not the bar. Each hot item is measured against a same-work, hand-written, idiomatic
Rust baseline (release build, same inputs, same result consumption, alternating runs, medians under load). Anything
above 1.5x hand Rust is profiled, classified and fixed, or the reason it cannot be fixed is recorded here.

Details, methods and raw numbers:

- host and resident-state path: [perf_headroom/HOST.md](perf_headroom/HOST.md) (runner `perf_headroom/host/run.py`)
- generated-code items: [perf_headroom/COMPILER.md](perf_headroom/COMPILER.md) (runner
  `perf_headroom/compiler/run_compiler_perf.py`)

## Summary (ns per call; n = 100 / 500 / 1000)

| item | Pruv | hand Rust | TS | Pruv/hand | cause | status |
|---|---|---|---|---|---|---|
| M4 resident state: sessions | 19.0-19.7 | 13.6-15.1 | 54-60 | 1.40 / 1.30 / 1.30 (was 1.60 / 1.52 / 1.51) | was the `ent_u16::rd`/`wr` total-function guard (about 23%); fixed by source fast paths (one range check, plain proven indexing, total fallback). Left: op-decode guards in root_m4 and the word-buffer layout | within bar |
| M4 resident state: tools | 36-40 | 53 | 60 | 0.69 / 0.75 / 0.74 | - | ok |
| M4 resident state: clocks | 34-38 | 38-40 | 99-108 | 0.98 / 0.86 / 0.98 | - | ok |
| M5 store_find, lookup only | 14-16 | 12-13 | - | 1.18 / 1.21 / 1.29 | was id conversion allocation | fixed (33 -> 14-16) |
| M5 store_find, JV encode only | - | - | - | 1.11 / 1.21 / 1.15 | - | ok |
| M5 store_find, lookup + JV tree encode | 369-409 | 289-300 (tree) / 68-77 (text) | 629 / 1594 / 3283 | 1.27-1.48 (tree) / 5.1-5.8 (text) | J tree building (about 12 allocations) | won't fix: trace-only path; the product answer is fact words, and a second text encoder would duplicate `Heap::enc` |
| M5 store_save | 163-172 | 126-132 | - | 1.30 / 1.35 / 1.32 | - | fixed (about 310 -> 168) |
| M6 String(int) `p_int` | 13.8-15.8 | 13.1-15.4 | 24-107 | 1.03 / 1.09 / 1.07 | - | ok (stack small results, KD-M6-3) |
| M6 `p_iso` | 25-30 | 29-32 | 335-373 | 1.04 / 0.96 / 0.86 | - | ok |
| M6 `p_ref` | 34-40 | 39-44 | 60-240 | 0.98 / 0.88 / 0.93 | - | ok |
| amount_text `p_amt` | 26-33 | 29-34 | 27-52 | 0.97 / 0.97 / 0.98 | was 4 heap vectors and 2 copies per call | fixed in `bud_text.pruv` (about 80 -> 26-33) |
| `ent_u16` guard, simple write loop `p_wr` | 6.5-7.1 | 6.1-7.1 | 92-154 | 1.00 / 0.99 / 1.08 | guard branch is predicted; no cost in a tight loop | ok |

The guard result differs between `p_wr` (no cost) and M4 sessions (about 23%): in a tight loop the always-taken branch
is free, while in the M4 state code the guards sit on long dependent chains of reads and block other work. M4 sessions
now avoids most of them in source: one range check per run of state accesses, plain proven indexing inside, and the
total accessor as the fallback (`perf_headroom/HOST.md`, M4 sessions profile).

New budget milestones (M7-M14) report the same three columns in their notes.
