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
| M8 charge | 33.7-40.9 | 16.0-17.3 | 128-138 | 2.11 / 2.34 / 2.43 (instr 1.97-1.99 with the inline policy, was 2.33-2.36; pass 2: 1.84 / 2.36 / 2.17) | i64-per-unit key and id width (hash, compare, bid), `valid()` boundary; call frames removed by the compiler inline policy; line by line in `M8_NOTES.md` "Perf pass 3" and "Inline policy" | open (> 1.1x), written reason |
| M8 noteFailure | 25.4-31.7 | 13.5-15.4 | 28-33 | 1.88 / 2.15 / 2.06 (instr 2.03-2.05 with the inline policy, was 2.40-2.45) | same | open (> 1.1x), written reason |
| M8 noteTool dedup hit | 27.4-39.7 | 24.3-26.0 | 12-14 | 1.05 / 1.63 / 1.61 (instr 1.29-1.33 with the inline policy, was 1.46-1.53, ~1.65) | same; TS wins with V8's cached string hashes | open (> 1.1x), written reason |
| M8 noteTool grow + forgetTurn | 84-102 | 77-83 | 54-58 | 1.09 / 1.20 / 1.23; with the inline policy 0.99 / 1.04 / 1.08 (instr 1.01 / 0.96 / 0.97, was 1.13 / 1.07 / 1.08, ~1.19) | hash once per new activity (`bud_map::add_absent`), inline policy; rest is key width | ok (within 1.1x on ns and instr) |

M8 rows are pass 3: `m8_test/run_m8.py perf` with per-item processes, rotated order, 7 rounds and an instruction
column (load average ~4.3). `note_tool` now hashes the activity key once (`bud_map::add_absent`), which took
toolgrow from ~2075 to ~1870 instructions and tooldup from ~760 to ~700. Charge and failure stay at ~2.4x
instructions. The line-by-line table in `M8_NOTES.md` puts the gap in the key and id width (one i64 per UTF-16 unit:
hash, key compare and the 36-word budget-id compare, about 300 of 694 instructions), the `valid()` boundary (about
70) and call frames. The PLACEHOLDER record copy measured at about 0.

Compiler inline policy (after pass 3; SPEC section 7, "Inline policy"; candidates and numbers in `M8_NOTES.md`,
"Inline policy"): non-recursive fns get `#[inline(always)]` when their inclusive size is at most 8, when they have one
call site, or when incl * (call sites - 1) <= 1500. Instructions per op: M8 charge 825 -> 698, failure 697 -> 585,
tooldup 710 -> 611, toolgrow 1876 -> 1680; M7 8340 -> 7629; M9 20728 -> 20431; M4 sessions 6712 / 6830 -> 6323 /
6547, tools 10097 / 11064 -> 9784 / 10724; M11 B 1000 16.29M -> 15.29M, C 1000 11.43M -> 11.00M. No measured row got
worse. rustc CPU goes up 1.2-2.4x (M7 0.97 -> 2.28 s, M11 1.56 -> 2.71 s) and binaries grow 2.6-10.7%;
`#[inline(always)]` on every fn was 3-4x and up to +38%. The ns columns above are from before the policy except
where marked.

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
3. `#[inline]`: done in the compiler for every generated function; the inline policy (SPEC section 7) raises it to
   `#[inline(always)]` where it measured as a gain.
4. Slice equality: done, `eq_range` (one bounds-proved slice `==`, `memcmp` in the binary), used in
   `bud_map::entry_matches`. No `bud_lifecycle.pruv` change is needed for any of these.

