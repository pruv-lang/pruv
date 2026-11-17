# Request-budget piece: performance headroom against hand-written Rust

Rule (2026-11-17): beating TS is not the bar. Each hot item is measured against a same-work, hand-written, idiomatic
Rust baseline (release build, same inputs, same result consumption, alternating runs, medians under load).

Bar (tightened 2026-11-17, replaces the earlier 1.5x): Pruv compiles to Rust, so the target is parity (1.0x). Anything
above 1.1x hand Rust must be fixed, in source, lib, host or compiler. 1.0-1.1x is treated as measurement noise, but its
cause is still recorded. "Test-only path" and "trace-only path" are not exemptions. An item may stay above 1.1x only if
the notes show, line by line, the difference between the hand Rust and the generated Rust and why Pruv cannot emit the
hand code.

Details, methods and raw numbers:

- host and resident-state path: [perf_headroom/HOST.md](perf_headroom/HOST.md) (runner `perf_headroom/host/run.py`)
- generated-code items: [perf_headroom/COMPILER.md](perf_headroom/COMPILER.md) (runner
  `perf_headroom/compiler/run_compiler_perf.py`)

## Summary (ns per call; n = 100 / 500 / 1000)

| item | Pruv | hand Rust | TS | Pruv/hand | cause | status |
|---|---|---|---|---|---|---|
| M4 resident state: sessions | 19.0-19.7 | 13.6-15.1 | 54-60 | 1.40 / 1.30 / 1.30 (was 1.60 / 1.52 / 1.51) | was the `ent_u16::rd`/`wr` total-function guard (about 23%); fixed by source fast paths (one range check, plain proven indexing, total fallback). Left: op-decode guards in root_m4 and the word-buffer layout | open (> 1.1x) |
| M4 resident state: tools | 36-40 | 53 | 60 | 0.69 / 0.75 / 0.74 | - | ok |
| M4 resident state: clocks | 34-38 | 38-40 | 99-108 | 0.98 / 0.86 / 0.98 | - | ok |
| M5 store_find, lookup only | 9.6-11.2 | 11.7-13.0 | - | 0.96 / 0.76 / 0.78 | was the UTF-16 key build and the handle read; now a byte-keyed index holding the handle | ok (was 1.18 / 1.21 / 1.29) |
| M5 store_find, JV encode only | 31-39 | 35-39 | - | 0.87 / 0.95 / 1.02 | was the J tree; now one text encoder | ok (was 1.11 / 1.21 / 1.15 vs tree) |
| M5 store_find, lookup + JV text encode | 67-80 | 70-77 (text) | 671 / 1620 / 3392 | 0.96 / 1.01 / 1.04 | was J tree building (about 12 allocations) and String labels; now text encoder and numeric labels | ok (was 1.27-1.48 vs tree, 5.1-5.8 vs text) |
| M5 store_save | 128-130 | 129-136 | - | 0.96 / 1.00 / 0.99 | was `u16s` regrowth and String labels | ok (was 1.30 / 1.35 / 1.32) |
| M6 String(int) `p_int` | 13.8-15.8 | 13.1-15.4 | 24-107 | 1.03 / 1.09 / 1.07 | - | ok (stack small results, KD-M6-3) |
| M6 `p_iso` | 25-30 | 29-32 | 335-373 | 1.04 / 0.96 / 0.86 | - | ok |
| M6 `p_ref` | 34-40 | 39-44 | 60-240 | 0.98 / 0.88 / 0.93 | - | ok |
| amount_text `p_amt` | 26-33 | 29-34 | 27-52 | 0.97 / 0.97 / 0.98 | was 4 heap vectors and 2 copies per call | fixed in `bud_text.pruv` (about 80 -> 26-33) |
| `ent_u16` guard, simple write loop `p_wr` | 6.5-7.1 | 6.1-7.1 | 92-154 | 1.00 / 0.99 / 1.08 | guard branch is predicted; no cost in a tight loop | ok |

The guard result differs between `p_wr` (no cost) and M4 sessions (about 23%): in a tight loop the always-taken branch
is free, while in the M4 state code the guards sit on long dependent chains of reads and block other work. M4 sessions
now avoids most of them in source: one range check per run of state accesses, plain proven indexing inside, and the
total accessor as the fallback (`perf_headroom/HOST.md`, M4 sessions profile).

M11 work cards (from `M11_NOTES.md`): A validate+build 1.02 / 1.09, A error path 1.41, B successive setCard 1.40-1.41
(stand-in store copies the budget per unit), C = B without the stand-in 1.02-1.03. A error path and B are open (> 1.1x).

New budget milestones (M7-M14) report the same three columns in their notes and meet the same bar.
