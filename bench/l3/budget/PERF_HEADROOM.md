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


## P01: matched boundary, key ablations, equal-semantics hand (2026-11-18)

Spec: `docs/L3_RESTRUCTURE_SPEC.md` §1.2, §4.1, §6.2 P01. All numbers below are measured in this pass; no saving is
assumed. Instructions are `instructions retired` from `/usr/bin/time -l`, (run at 5 x reps - run at reps) / extra calls;
M7 and M8 take the median of 3 samples, the key bench takes 1 sample. ns are medians over alternating rounds; "spread" is (max - min) / median over the rounds.

### M7 matched boundary (`m7_test/run_m7.py boundary`, 7 rounds alternating variant order, S = 100 / 500)

Variants run the same M7 code on the same state and keys, per opcode:

- A: legacy `t_call` packed op through the World wrapper (take_op / put_op / call_with).
- B: `t_call` called directly on a prepared op (no wrapper).
- Bnv: B with `bud_store::valid(st)` removed (`t_call_nv`), validation only.
- C: typed `c_*` entries (keys as unit ranges, no packed-op decode, no `valid`).
- prep / observe: building the op, and consuming the view words; present in every variant.

A driver self-check exits 2 if any variant differs in result, state or effects; all runs passed (checksums equal).
Allocations are 0 per call except `replace` (1, the stored record).
Control: M7 Q1 300 cases (Unicode and numeric), exact 113 / not implemented 187 / fail 0 (`m7_test/run_m7.py`).

| item | A instr | B instr | Bnv instr | C instr | A ns (100) | B ns | Bnv ns | C ns |
|---|---|---|---|---|---|---|---|---|
| current | 1905 | 1760 | 1702 | 1604 | 105.7 | 102.6 | 101.3 | 99.3 |
| isHalted | 930 | 826 | 767 | 666 | 26.8 | 23.8 | 22.2 | 20.3 |
| attach | 982 | 875 | 807 | 757 | 32.3 | 28.6 | 26.8 | 25.9 |
| close + reopen (pair) | 2056 | 1767 | 1631 | 1526 | 59.6 | 48.7 | 46.8 | 42.5 |
| replace | 3739 | 3576 | 3508 | 3458 | 142.3 | 136.8 | 136.0 | 133.6 |
| noop | 329 | 221 | - | - | 9.1 | 6.3 | - | - |

Most spreads are at most 0.12. The exceptions in `work/m7/boundary.md` include current A at 100 (1.00) and 500 (0.47),
current B at 100 (0.25), attach Bnv at 500 (0.25), noop B at 500 (1.27) and close+reopen prep (1.07). Those cells are
reported but not used for a decision; the instruction counts are used instead.

Decomposition (per call; close+reopen per pair):

- A - B, wrapper: 104-163 instructions, 2-6 ns (pair: 289 instructions, ~11 ns). A no-op call alone costs 108.
- B - C, packed decode and checks including `valid`: 118-160 instructions, 0.4-3.5 ns (pair: 242).
- B - Bnv, `valid` only: 58-68 instructions, 0.5-1.8 ns (pair: 136). It is part of B - C and is not added to it.
- observe (reading the result view) is 974 of current's 1905 instructions and is in every variant.

Result: removing the whole boundary (A -> C) saves 225-301 instructions and 6.4-8.7 ns per call (pair: 530
instructions, 17.1 ns).
It is a real C3/C4 cost, but it is not the main M7 gap. The historical "~12 of 18 ns is the boundary" reading is
replaced by these matched numbers.

### Equal-semantics M8 hand (`m8_test/hand_m8_u16.rs`) and C1 re-split

`hand_m8_u16` keeps the Pruv semantics: `Box<[u16]>` keys in std `HashMap` (lone surrogates preserved, exact unit
equality), safe-integer checks on operands and results (+-(2^53 - 1), `checked_add`), the `bud_charge.pruv` refusal
order (no budget / closed -> 1; halted -> 16 + reason; per key in order: unsafe operand or sum -> 5, over limit ->
32 + k; then all writes, atomic), noteTool insert-then-check order, and malformed records stored as opaque words.
The old `hand_m8.rs` (UTF-8 String keys, plain `+=`) stays as an informational row only.

Controls: Q2 300 cases, Pruv and hand16 both exact 63 / not implemented 237 / fail 0; derived 300 cases, both exact
222 / fail 0; `m8_test/diff_hand16.py` 300 edge programs (empty / NUL / astral / lone high and low surrogate keys,
near-miss budget ids, 0, +-1, +-M, +-(M + 1), 2^53, 1e12), byte-identical output, 0 mismatching cases (outcome lines:
R 816, H 14, X 5 185).

`m8_test/run_m8.py perf --rounds 7 --sizes 100,500` (load average not controlled; 4 sides rotated):

| item | n | Pruv ns | hand16 ns | old hand ns | TS ns | Pruv/hand16 (bar) | Pruv/old hand | Pruv instr | hand16 instr | old hand instr | instr Pruv/hand16 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| charge | 100 | 30.7 | 20.6 | 14.8 | 123.5 | 1.49 | 2.07 | 695 | 509 | 351 | 1.36 |
| charge | 500 | 35.4 | 23.5 | 16.0 | 127.4 | 1.51 | 2.21 | 698 | 508 | 350 | 1.37 |
| failure | 100 | 22.8 | 15.9 | 12.9 | 26.7 | 1.43 | 1.77 | 581 | 358 | 286 | 1.62 |
| failure | 500 | 26.6 | 17.6 | 13.4 | 31.3 | 1.51 | 1.99 | 584 | 357 | 285 | 1.63 |
| tooldup | 100 | 24.4 | 25.1 | 24.4 | 11.4 | 0.97 | 1.00 | 596 | 557 | 462 | 1.07 |
| tooldup | 500 | 33.1 | 24.3 | 23.1 | 12.1 | 1.36 | 1.43 | 610 | 556 | 460 | 1.10 |
| toolgrow | 100 | 77.9 | 89.3 | 74.4 | 50.8 | 0.87 | 1.05 | 1666 | 1957 | 1642 | 0.85 |
| toolgrow | 500 | 88.9 | 100.7 | 81.4 | 54.6 | 0.88 | 1.09 | 1680 | 2083 | 1751 | 0.81 |

Re-split of the old gap (instructions at n = 100, Pruv - old hand = semantics part + remaining part; at n = 500 the
remaining part is charge 190, failure 227, tooldup 54):

| item | old gap | equal semantics (hand16 - old hand) | remaining (Pruv - hand16) |
|---|---|---|---|
| charge | 344 | 158 (46%) | 186 |
| failure | 295 | 72 (24%) | 223 |
| tooldup | 134 | 95 (71%) | 39 |
| toolgrow | 24 / -71 | 315 / 332 | -291 / -403 (Pruv ahead) |

So C6 (safe-number work) and the UTF-16 key representation together explain about half of the old charge gap and a
quarter of failure. Against the equal-semantics bar charge (1.36x instr, ~1.5x ns) and failure (1.62x instr, ~1.5x ns)
stay open; tooldup is 1.07-1.10x instr (the 1.36x ns at n = 500 is not matched by instructions and needs a re-run
before it counts); toolgrow is ahead of hand16.

### Key representation (`p01_keys/`, 7 rounds rotated, N = 100 / 500)

`p01_keys/keys.pruv` holds four record layouts behind the same open-addressing table, the same probe loop and the same
call structure (`k_find_L_H`, `k_insert_L_H`, `k_check_L`, `k_cmp_L`), so only the layout and the hash change:

- U: `[n, u...]`, one i64 per UTF-16 unit (today's budget keys).
- P (packed-u16): `[n, w...]`, 4 units per word, big-endian 16-bit lanes, zero tail. Check is O(1): the tail lanes.
- A8 (ASCII-8): `[2n + tag, w...]`; tag 0 packs 8 ASCII bytes per word, tag 1 falls back to P (needs a unit >= 128).
- F1: `[2 * bytelen + tag, n, w...]`, 8 bytes per word; tag 1 is `FF, lo, hi...` per unit.

The hashes are u (the `bud_map::hash_key` unit hash, one mix per 4 units, the same value for every layout) and w (one mix per data word,
then `fin(bxor(h, k[0]))`). The hand lane (`hand_keys.rs`) is std `HashMap<Box<[u16]>, u32>` (SipHash), which is the
idiomatic baseline. Its raw input is already `&[u16]`, so its raw row equals its ret row.

Rows:

- ret: find of a retained, pre-encoded record (hits).
- miss: find of an absent key.
- raw: host encode from `&[u16]` + `k_check` + find.
- enc / chk / cmp: host encode, Pruv check, and neighbour compare, each on its own.

Sets: sid `session-%05d` (13 ASCII), uuid (36 ASCII), nonascii `세션-%05d-ключ` (mixed BMP), and surr (ASCII + lone
D800 / DC00 + astral pair).

`run_keys.py check --count 300 --seed 20261117`: 300 pair cases (empty, NUL, every tail length 0-7, ASCII, BMP,
128-255, astral, lone high / low surrogates; equal, prefix and last-unit-differs pairs), 39 noncanonical records and
10 canonical controls. Encodings, check, unit-hash equality across layouts, find / insert hit and miss for both
hashes, and compare order are checked against a Python oracle (UTF-16 code-unit order): 0 mismatches. All perf
checksum groups agree with the hand lane.

ns per op, median (min-max over 7 rounds), N = 100. Instructions are for ret / raw. At N = 500 the cells stay within 10%, except U/u ret
(+10-21%, the larger U table), A8/w and F1/w raw on sid / nonascii (+13-14%), F1/w surr ret (+11%) and uuid U/u raw
(bimodal, see below).

| set | row | hand | U/u | P/u | P/w | A8/w | F1/w |
|---|---|---|---|---|---|---|---|
| sid | ret | 13.54 (13.38-14.07), 292 i | 11.49 (11.38-22.25), 273 i | 9.13 (8.97-9.23), 232 i | 7.65 (7.49-7.71), 186 i | 6.18 (6.06-6.33), 162 i | 6.13 (6.00-6.30), 165 i |
| sid | raw | 13.68, 291 i | 19.57, 414 i | 16.85, 405 i | 15.35, 360 i | 15.37, 400 i | 16.02, 414 i |
| uuid | ret | 20.16 (19.20-20.74), 407 i | 19.25 (18.82-21.51), 415 i | 14.53 (13.36-14.79), 326 i | 12.27 (11.20-12.69), 237 i | 8.88 (8.61-8.99), 200 i | 8.88 (8.55-9.00), 203 i |
| uuid | raw | 20.17, 404 i | 26.22, 600 i | 29.81, 676 i | 27.76, 588 i | 28.07, 682 i | 29.80, 696 i |
| nonascii | ret | 14.10 (13.89-15.09), 293 i | 11.97 (11.69-13.01), 273 i | 9.03 (8.90-9.41), 232 i | 7.85 (7.74-8.44), 189 i | 8.47 (8.04-8.96), 187 i | 7.52 (7.37-8.09), 191 i |
| nonascii | raw | 14.12, 291 i | 19.51, 414 i | 17.13, 405 i | 17.22, 363 i | 17.11, 414 i | 19.03, 551 i |
| surr | ret | 15.01 (14.18-15.36), 301 i | 12.64 (12.36-13.54), 280 i | 9.12 (9.03-10.10), 237 i | 7.75 (7.63-8.44), 189 i | 7.52 (7.12-7.80), 187 i | 8.67 (8.56-9.21), 207 i |
| surr | raw | 15.12, 302 i | 20.36, 408 i | 19.78, 417 i | 17.06, 370 i | 19.38, 465 i | 31.59, 930 i |

| set (N = 100) | bytes / key: hand (u16 payload) / U / P / A8 / F1 | enc ns U / P / A8 / F1 | chk ns U / P / A8 / F1 | cmp ns hand / U / P / A8 / F1 |
|---|---|---|---|---|
| sid | 26 / 112 / 40 / 24 / 32 | 3.64 / 5.73 / 7.10 / 7.91 | 2.78 / 1.45 / 2.47 / 2.40 | 5.68 / 6.15 / 2.53 / 2.38 / 2.49 |
| uuid | 72 / 296 / 80 / 48 / 56 | 5.99 / 13.78 / 16.64 / 18.38 | 3.40 / 0.90 / 3.15 / 2.96 | 13.06 / 11.76 / 3.97 / 2.97 / 3.19 |
| nonascii | 26 / 112 / 40 / 40 / 48 | 3.56 / 5.76 / 6.02 / 8.81 | 2.73 / 1.18 / 3.06 / 2.79 | 3.43 / 3.64 / 2.08 / 2.45 / 12.60 |
| surr | - / 136 / 40 / 40 / 56 | 3.95 / 7.52 / 8.45 / 10.62 | 1.79 / 0.89 / 2.94 / 11.30 | 4.22 / 5.04 / 2.22 / 2.68 / 15.04 |

Hand bytes count only the u16 payload, not the `Box` header and allocation.

The factors, each measured with the others fixed:

- Layout (hash u fixed): U -> P lowers ret by 2.4-4.7 ns and 41-89 instructions; the gain grows with key length.
- Hash (layout fixed): u -> w lowers P ret by 1.2-2.3 ns and 43-89 instructions. On U, w is slower (one mix per
  word is one mix per unit there, vs one per 4 units for u), so the hash and the layout must change together.
- Inline: the `+inl` lanes (every generated `#[inline(always)]` turned into `#[inline]`) are within 0.96-1.05 of the
  base in 251 of 288 cells, so the layout and hash gains are not inlining effects. The 37 other cells include sid P chk
  (0.74), F1 cmp and F1/u ret (0.80-0.92, the F1 unit decode), and uuid N = 500 U raw / enc (0.42-0.67). The base runs of those
  uuid cells are bimodal (min 29.4 vs median 68.5 ns), so they are noise and not used.
- Retained vs raw: on retained keys every packed layout beats hand (P/w 0.52-0.61x). Raw keys pay the host encoder,
  and no layout reaches hand: P/w raw is 1.12 / 1.38 / 1.22 / 1.13x at N = 100 (sid / uuid / nonascii / surr), up to 1.43x at N = 500 (uuid).
- Host encode vs Pruv check: chk + ret below is a sum of separately measured medians, not a measured combined lane
  (enc + chk + ret does not reproduce raw exactly, e.g. nonascii P/w 14.8 vs 17.2 ns). The Pruv part (chk + ret) for P/w is 9.1 / 13.2 / 9.0 /
  8.6 ns, which is 0.58-0.67x of the hand find. The rest is the host encoder (5.7-13.8 ns for P). That encoder is a
  plain per-unit shift loop over `chunks(4)` and is slower than the U copy (6.0 ns on uuid). No faster encoder was
  measured.

**Chosen representation: packed-u16 (P) with the word hash.** Reasons, from the tables:

1. It has no content cliff. F1 decodes non-ASCII units: cmp 12.6 / 15.0 ns vs P 2.1 / 2.2 ns (nonascii / surr),
   surr chk 11.3 vs 0.9 ns, surr raw 930 vs 370 instructions. A8 is ahead of P on ret for ASCII (uuid 8.9 vs 12.3 ns) and slightly on surr
   (7.5 vs 7.75 ns). On summed chk + ret it is about equal
   on sid / uuid (8.7 / 12.0 vs 9.1 / 13.2 ns) and behind on nonascii / surr (11.5 / 10.5 vs 9.0 / 8.6 ns), and it
   needs a two-form tag check (2.5-3.2 ns vs P's 0.9-1.45 ns).
2. Its check is O(1) (tail lanes and word count), which keeps the per-call external-input validation cheap.
3. It keeps exact UTF-16 units, lone surrogates included, with no decode, and its word order equals code-unit order
   (checked).
4. It is 2.8-3.7x narrower than U (sid 40 vs 112, uuid 80 vs 296 bytes).

A8 stays a possible later ASCII fast path; it was not chosen because of the second form and its check cost.

**Handoff to P03-P05 (product source; P01 did not change product source):**

- Budget keys and ids in the P record `[n, w...]` (format and `k_check_p` / `k_cmp_p` in `p01_keys/keys.pruv`).
  Stored hashes change to the word hash, so the hashes in existing index entries must be recomputed at migration.
- Equality as `eq_range` over `1 + ceil(n / 4)` words. Ordering, where it is used, by word compare (equal to unit
  order).
- External keys are validated with `k_check_p` on every call (required). Retained keys inside the resident state
  rely on the state invariant instead (C3 work).
- The host encodes a key once at ingress and keeps the packed record. Per-call re-encoding is the cost that keeps raw
  keys above 1.0x. A faster (vectorizable) encoder is a host item and is not measured here.

### Remaining causes, re-ranked from P01 measurements

| rank | cause | P01 evidence | note |
|---|---|---|---|
| 1 | C1 key / id width | Key bench: U/u -> P/w ret saves 3.8-7.0 ns and 84-178 instructions per lookup. Against hand16, M8 charge has 186 and failure 223 instructions left (n = 100) | The M8 effect of P is expected but not measured; it needs the P03-P05 port |
| 2 | C4 packed argument / result boundary | M7 B - C 118-160 instructions, 0.4-3.5 ns (pair 242); wrapper A - B 104-163 | The A - B wrapper is in the driver and host, not in Pruv code |
| 3 | C3 `valid` re-check | M7 B - Bnv 58-68 instructions, 0.5-1.8 ns (pair 136) | Inside B - C; not counted twice |
| 4 | C2 accessor re-check / record | Not isolated in P01. M7 observe (view consumption) is 974 of current's 1905 instructions in every variant | Needs its own ablation before any claim |
| 5 | C7 residual frames | Key bench inline factor 0.96-1.05; the M8 inline policy is already in the baseline | No new P01 gain found |
| - | C6 safe-number work | Now inside the hand16 bar (part of the 158 / 72 instruction semantic share) | No longer a gap against the bar; its exact cost in Pruv vs `checked_add` is not split |

**Can the chosen layout reach 1.0x?** For the retained-key lookup, measured yes: P/w ret is 0.52-0.61x of the SipHash
hand map. With the external check added, the sum of separate medians is 0.58-0.67x; a combined check + find lane was
not measured, so this part is an estimate. Whole M8 ops are not shown at 1.0x
yet. Charge and failure need C1 (P port) plus C4 / C3 to close 186 / 223 instructions (n = 100), and only the C1 part has a
measured per-lookup saving. Raw keys re-encoded on every call stay at 1.12-1.43x unless the host keeps packed records.

**Which checks proofs can remove:**

- `valid` on trusted resident state: 58-68 instructions per call, through the resident contracts (C3).
- `k_check_p` on keys already inside the state, through the state invariant. Its separate cost is 0.9-1.45 ns; the
  saving inside a combined lane was not measured.
- The external-key `k_check_p` stays, as required.
- No bounds-check share inside find was isolated, so no figure is claimed for it.
- Caveat: hand rows use std SipHash. A hand row with a faster non-default hasher was not measured.

### Open items after the P01 audit

- Blocker for "equal semantics": on malformed external input, hand16 and the Pruv M8 side disagree. hand16 turns a
  record with state, unlimited or reason out of range into an opaque record and rejects key units outside 0..65535;
  `m8_test/root_m8.pruv` (`load_cv`, `t_call`) checks neither. Examples: charge on a matching record with state 3 is
  `X 2` in Pruv and `R 1` in hand16; `call 2 1 65536 0` on an empty state is `R -` in Pruv and `X 2` in hand16. The
  controls above cover well-formed input only. Fix: add the missing range checks on the Pruv side (external-input
  validation is required), then add malformed fixed controls to `diff_hand16.py`.
- Result consumption: the key bench `enc` row observes only the length and last word of the buffer, M8 does not
  compare a full final-state checksum between sides, and the M7 driver does not consume every effect word. No DCE was
  observed, but the rows should consume the full result.
- Not produced in P01: a typed / same-packet hand comparator for M7, an optimized hand with the same data structure,
  full-result S / M runs, the compiler agreement and exact version record, explicit "unavailable" marks for
  allocation / probe counters, and a hand row with a non-default hasher.
- Kept outputs: Q2 63 / 222 and the edge outcome counts (816 / 14 / 185) come from runs whose outputs are not kept in
  the repo; re-run before relying on them.
