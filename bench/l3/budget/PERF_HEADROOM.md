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
| M4 resident state: sessions | 18.0-18.6 | 14.7-16.4 | 59-66 | 1.22 / 1.13 / 1.14 (pass 3; was 1.22 / 1.17 / 1.16, 1.40 / 1.30 / 1.30) | compiler now emits `#[inline]` on every generated fn and `bud_map` compares keys with the `eq_range` builtin. Left: the per-op `valid()` header check of the raw-buffer ABI (about 2 ns of 2-3 ns; scratch build without it is at parity). `valid` already runs once per root call. Line by line and the typed-handle design that would remove it: `perf_headroom/HOST.md` | open (> 1.1x), written reason: ABI header check |
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
| M9 clock refresh R = 10 / 50 / 100 | 153-155 | 143-148 | 2900-3260 | 1.05 / 1.07 / 1.07 (was 1.10-1.11 after the map hash; 1.39-1.44 before) | `#[inline]` hints and `eq_range` key compare in `bud_map::entry_matches` | ok |
| M9 cycle / fire | 157 / 224 | 168 / 209 | 3025 / 5506 | 0.93 / 1.07 | same | ok |
| M7 current | 27.5-28.4 | 10.0-12.4 | 24-25 | 2.22 / 2.80 / 2.84 (main before this pass: 2.88 / 3.02 / 2.96) | root-call overhead of the M7 harness: dispatch 7.6 ns + raw call 4.6 ns, then `valid()` and one `bud_map::find` (see "M7" below) | open (> 1.1x), ABI / harness |
| M7 isHalted | 27.0-27.6 | 9.9-10.0 | 24-25 | 2.76 / 2.73 / 2.74 (was 2.90 / 2.83 / 2.96) | same | open |
| M7 maybeClose + reopen | 29.5-30.0 | 17.6-18.3 | 275-281 | 1.61 / 1.68 / 1.70 (was 1.74 / 1.79 / 1.78) | same, two root calls per pair | open |
| M7 attach / replace | 21 / 124 | 53 / 205-212 | 6-9 / 1134-1166 | 0.40 / 0.58-0.61 (was 0.41-0.45 / 0.58-0.65) | - | ok |
| M8 charge | 34.6-41.3 | 17.5-19.0 | 138-158 | 1.84 / 2.36 / 2.17 (main before this pass: 2.29 / 2.39 / 2.39) | see "M8" below | open (> 1.1x), outside this pass's files |
| M8 noteFailure | 24.9-33.3 | 14.2-18.5 | 30-36 | 1.35 / 2.25 / 2.07 (was 1.93 / 2.37 / 2.05) | same | open |
| M8 noteTool dedup hit | 33.9-48.7 | 25.4-28.5 | 13-16 | 1.19 / 1.92 / 1.76 (was 1.30 / 1.86 / 1.81) | same | open |
| M8 noteTool grow + forgetTurn | 93-112 | 86-92 | 57-61 | 1.01 / 1.25 / 1.30 (was 1.24 / 1.31 / 1.33) | same | open |

M8 rows are `m8_test/run_m8.py perf --rounds 5` with the compiler of this pass (load average 7.7-14.7) and with the
compiler and `bud_map` of main before it (load average 17-22). Only the n = 100 rows moved (by 0.1-0.6); n = 500 / 1000 are within the noise. M8's calls are short
keys (13-unit session ids), where the packed hash and `eq_range` save little. Its remaining gap, from the line-by-line
table in `M8_NOTES.md`, sits in `m8_test/root_m8.pruv` (`load_cv` copies the PLACEHOLDER record into `cv` and
`store_cv` writes it back, about 20-30 instructions; the hand side reads a struct field) and in the `t_call`
boundary `valid()` (about 80 of 736 instructions). Neither is in this pass's files (`m8_test/` belongs to M8, and the
record layout waits for M7). A find-or-slot API in `bud_map` would only save the second `hash_key` of a new
activity's 2-5 word key in toolgrow (estimated at a few tens of the 2206 instructions per toolgrow call in `M8_NOTES.md`; not measured), so it was not added.

The guard result differs between `p_wr` (no cost) and M4 sessions (about 23%): in a tight loop the always-taken branch
is free, while in the M4 state code the guards sit on long dependent chains of reads and block other work. M4 sessions
now avoids most of them in source: one range check per run of state accesses, plain proven indexing inside, and the
total accessor as the fallback (`perf_headroom/HOST.md`, M4 sessions profile).

M11 work cards (from `M11_NOTES.md`): A validate+build 0.79 / 0.96, A error path 0.96 (was 1.41), B successive setCard
0.89-0.90 (was 1.40-1.41), C = B without the stand-in 0.80. All are ok. The fixes: the runner (A reps scale with size,
alternating run order), plain proven V-word reads plus one `obj_get` pass in `bud_card.pruv`, and whole-region copies
in `blob_of` / `emit_all`.

New budget milestones (M7-M14) report the same three columns in their notes and meet the same bar.

M7 rows are `m7_test/run_m7.py perf --rounds 5` with main's compiler and `bud_map` (d28b52c, load average 9.7-11.3)
and with this pass (load average 8.2-9.2); the M7 code is the same in both. `current` and `isHalted` move by about
2 ns (`#[inline]`, `eq_range`); what is left is not in the map code. Per call, hand Rust does one `HashMap::get` and a
field read (10 ns). The generated path pays the harness dispatch (7.6 ns) and the raw-buffer call (4.6 ns), both
measured by the runner's `dispatch` / `rawcall` rows, before any M7 code runs, then `bud_store::valid` and one
`bud_map::find` over a short key, which take the remaining ~16 ns, about 6 ns more than the hand lookup. So about
12 of the 18 ns gap is the root-call boundary, which Pruv cannot remove in source: the hand side has no boundary at all. The
typed-handle design in `perf_headroom/HOST.md` removes `valid`; the boundary belongs to the ABI.

Answers to the M7 requests (`M7_NOTES.md`, "Requests to other owners"):

1. Hash tag bits in the slot word: implemented and measured, then not committed. Live slot words held e+1 in the
   low 33 bits and 30 hash bits above, and `find` / `put_key` compared the tag before `entry_matches`. All Q sets
   passed with it, but M7 current / isHalted / closereopen and M4 sessions / tools / clocks did not move (within
   0.5 ns). Probe chains are short under the index's load bound (3/4), so there are few non-matching candidates
   to filter (expected from linear probing; probe counts were not measured). The index format stays as it was.
2. Cheaper `valid`: it already runs once per root call; the typed-handle design (no trust in unchecked input) is in
   `perf_headroom/HOST.md`.
3. `#[inline]`: done in the compiler for every generated function.
4. Slice equality: done, `eq_range` (one bounds-proved slice `==`, `memcmp` in the binary), used in
   `bud_map::entry_matches`. No `bud_lifecycle.pruv` change is needed for any of these.

