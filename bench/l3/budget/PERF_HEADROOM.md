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
Every variant's checksum folds the full result: the status, the effect count `fx[0]` and every effect word
`fx[1..=used]` (before the early return on a non-zero status), plus the observed view words.
Allocations are 0 per call except `replace` (1, the stored record).
Control: M7 Q1 300 cases (Unicode and numeric), exact 113 / not implemented 187 / fail 0 (`m7_test/run_m7.py`).

Full-result run (`work/m7/boundary_p01b.txt`, 7 rounds, S = 100 / 500; the table shows S = 100, S = 500 instructions
are within 1):

| item | A instr | B instr | Bnv instr | C instr | A ns (100) | B ns | Bnv ns | C ns |
|---|---|---|---|---|---|---|---|---|
| current | 1917 | 1772 | 1714 | 1615 | 105.1 | 101.3 | 100.8 | 98.9 |
| isHalted | 942 | 838 | 779 | 679 | 27.6 | 23.9 | 22.5 | 20.8 |
| attach | 986 | 880 | 812 | 762 | 32.3 | 29.1 | 27.3 | 26.0 |
| close + reopen (pair) | 2099 | 1810 | 1674 | 1568 | 60.2 | 49.9 | 48.2 | 44.0 |
| replace | 3761 | 3598 | 3529 | 3479 | 140.7 | 137.8 | 133.5 | 132.2 |
| noop | 341 | 233 | - | - | 9.7 | 6.7 | - | - |

Consuming every effect word adds 4-43 instructions per item, the same within a few instructions for every variant of that item (the earlier run read only part of the
effects); no difference between variants moved by more than a few instructions, so no variant was being eliminated.
Most spreads are at most 0.2. The exceptions include current A at 100 (0.73), attach Bnv at 100 (0.28), closereopen
C at 500 (0.92), replace C at 100 (0.42) / B at 500 (0.38) and replace prep at 500 (0.75). Those cells are reported
but not used for a decision; the instruction counts are used instead.

Decomposition (per call; close+reopen per pair):

- A - B, wrapper: 104-163 instructions, 2.9-7.4 ns (pair: 289 instructions, 9-10 ns). A no-op call alone costs 108.
- B - C, packed decode and checks including `valid`: 118-160 instructions, 0.8-5.6 ns (pair: 242, ~6 ns).
- B - Bnv, `valid` only: 58-68 instructions, 0.5-4.3 ns (pair: 136). It is part of B - C and is not added to it.
- observe (reading the result view) is 986 of current's 1917 instructions and is in every variant.

Result: removing the whole boundary (A -> C) saves 224-302 instructions and 6.2-8.5 ns per call (pair: 531
instructions, 16.2 ns).
It is a real C3/C4 cost, but it is not the main M7 gap. The historical "~12 of 18 ns is the boundary" reading is
replaced by these matched numbers.

### Equal-semantics M8 hand (`m8_test/hand_m8_u16.rs`) and C1 re-split

`hand_m8_u16` keeps the Pruv semantics: `Box<[u16]>` keys in std `HashMap` (lone surrogates preserved, exact unit
equality), safe-integer checks on operands and results (+-(2^53 - 1), `checked_add`), the `bud_charge.pruv` refusal
order (no budget / closed -> 1; halted -> 16 + reason; per key in order: unsafe operand or sum -> 5, over limit ->
32 + k; then all writes, atomic), noteTool insert-then-check order, and the same ingress rejection of malformed input (below); a stored record with
state -1 (no budget) is kept as opaque words.
The old `hand_m8.rs` (UTF-8 String keys, plain `+=`) stays as an informational row only.

Malformed external input (P01 follow-up). Policy: ABI.md (units 0..65535) and REQUEST_BUDGET_SPEC §12 (malformed host
input -> InvalidInput, status 2, state unchanged). Before, the sides disagreed: a matching record with state 3 charged
`X 2` in Pruv and `R 1` in hand16 (hand16 kept it opaque); unlimited 2 counted a failure in Pruv only; a key unit 65536
was `R -` in Pruv and a fatal exit in hand16. Now both sides validate once at ingress:

- `t_put` (and hand16 `put_words`): record n >= 15, state in -1..2, unlimited in 0..1, reason in -1..4,
  word 14 == n - 15, every id unit in 0..65535; otherwise status 2 before any write. Counter words are not validated
  (the safe-number checks of `bud_charge` cover them).
- `key_end` (both sides, every key of `t_put` and `t_call`): length within the op and every unit in 0..65535
  (the `m34_test/root_m4.pruv` loop).
- The q drivers print `P X <status>` for a rejected put and continue; perf setup puts stay fatal. hand16's old
  fail-open path (malformed record -> opaque) is removed.

Proof removal: every stored record has passed the `t_put` check (store invariant, stated in the `root_m8.pruv`
header). Under it the state / reason range checks in `load_cv` and `bud_charge` re-check what ingress already
proved and can be removed by proof once the invariant is carried by the resident-state contracts (C3); they are kept
in this pass (total code, no product change). The key-unit check on every `t_call` key stays: those keys are external
input on each call.

Controls (re-run in this pass, outputs in `work/p01b/`): Q2 300 cases, Pruv and hand16 both exact 63 / not
implemented 237 / fail 0; derived 300 cases (`cases_acct.jsonl`), both exact 222 / not implemented 78 / fail 0;
`m8_test/diff_hand16.py --count 300 --seed 20261117`: 300 random edge programs (empty / NUL / astral / lone high and
low surrogate keys, near-miss budget ids, 0, +-1, +-M, +-(M + 1), 2^53, 1e12), 0 mismatching cases, then 69 fixed
malformed controls (record state 3 / -2, unlimited 2 / -1, reason 5 / -2, n 0 / 14, word 14 too big / too small, id
unit 65536 / -1; key units -1 / 65536 / 70000 in the put key and in every call key position; put framing errors),
each with an expected Pruv log: 0 mismatches between sides, 0 expectation failures. Outcome lines (Pruv, random +
controls): R 984, H 14, X 5 185, X 2 27, P X 42.

Compiler agreement and versions: Python `pruvc/pruvc.py --backend rust` and Rust `pruvc-rs` (release, pruv repo
commit c9a4900) give byte-identical Rust for `root_m8.pruv` (112267 bytes) and `root_m7.pruv` (`run_m7.py same`,
161256 bytes). rustc 1.99.1 (58cf8f9c3 2026-10-13), `-C opt-level=3 --edition 2021`, every driver and hand file. Seeds:
Q2 / derived cases 20261117, `diff_hand16.py` 20261117, `run_keys.py check` 20261117.

`m8_test/run_m8.py perf --rounds 7 --sizes 100,500` after the ingress checks (load average not controlled; 4 sides
rotated; output `work/p01b/m8_perf.txt`). Each timed loop is followed, outside the timing, by a final-state checksum
(`chk`, the `t_observe` words folded); 56 (item, size, round) checksums are equal between Pruv and hand16:

| item | n | Pruv ns | hand16 ns | old hand ns | TS ns | Pruv/hand16 (bar) | Pruv/old hand | Pruv instr | hand16 instr | old hand instr | instr Pruv/hand16 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| charge | 100 | 40.4 | 19.2 | 15.2 | 123.4 | 2.10 | 2.66 | 988 | 496 | 352 | 1.99 |
| charge | 500 | 44.5 | 21.7 | 16.1 | 126.0 | 2.05 | 2.76 | 990 | 495 | 350 | 2.00 |
| failure | 100 | 33.4 | 16.1 | 12.8 | 26.8 | 2.07 | 2.61 | 884 | 364 | 287 | 2.42 |
| failure | 500 | 37.4 | 17.3 | 13.0 | 30.8 | 2.16 | 2.88 | 887 | 363 | 284 | 2.44 |
| tooldup | 100 | 41.5 | 31.9 | 24.1 | 11.6 | 1.30 | 1.72 | 1031 | 528 | 462 | 1.95 |
| tooldup | 500 | 51.4 | 30.2 | 23.0 | 12.1 | 1.70 | 2.23 | 1043 | 528 | 460 | 1.98 |
| toolgrow | 100 | 90.9 | 92.0 | 74.8 | 51.3 | 0.99 | 1.22 | 2092 | 1902 | 1642 | 1.10 |
| toolgrow | 500 | 97.1 | 101.2 | 81.2 | 54.3 | 0.96 | 1.20 | 2105 | 2029 | 1751 | 1.04 |

Before the ingress checks (same command, same machine) Pruv was charge 695 / 698, failure 581 / 584, tooldup 596 /
610, toolgrow 1666 / 1680 instructions, hand16 509 / 508, 358 / 357, 557 / 556, 1957 / 2083; Pruv/hand16 was 1.36,
1.62, 1.07-1.10 and 0.81-0.85 in instructions. **The checks are a measured Pruv regression:** +293 (charge), +303
(failure), +435 / +433 (tooldup), +426 (toolgrow) instructions per call, about 6 per key unit (charge / failure read
one ref of 13 + 36 units, noteTool four keys of 69 units). The cause is the new unit loop in `root_m8.pruv` `key_end`
(scalar, early exit, one compare pair and branch per unit), which runs on every key of every call. hand16 did not get
slower: its keys were already converted with `u16::try_from` per unit, so the range check was part of the work it
already did. The 1.36x tooldup ns at n = 500 of the earlier run is now 1.70x with 1.98x instructions, so it is no
longer an unexplained timing outlier but this regression.

Root fix (P02, not done here): the check is needed only for keys that do not hit a stored key. A lookup hit means the
key equals a stored key, and every stored key passed `t_put` (store invariant), so its units are already in range.
Checking units on the miss / insert path only (and once at `t_put`) keeps the rejection semantics (malformed -> 2)
and removes the per-unit loop from the hit path of charge, failure and tooldup; this needs a proof that a hit implies
the range, not a new check. Until then the M8 ratios above are the honest numbers.

Re-split of the old gap (instructions at n = 100, Pruv - old hand = semantics part + remaining part; at n = 500 the
remaining part is charge 495, failure 524, tooldup 515):

| item | old gap | equal semantics (hand16 - old hand) | remaining (Pruv - hand16) |
|---|---|---|---|
| charge | 636 | 144 (23%) | 492 |
| failure | 597 | 77 (13%) | 520 |
| tooldup | 569 | 66 (12%) | 503 |
| toolgrow | 450 / 354 | 260 / 278 | 190 / 76 |

Without the key-unit loop (the pre-check run) the remaining part was charge 186, failure 223, tooldup 39; about 300 /
300 / 460 of today's remaining part is the loop. Against the equal-semantics bar all four items are now open
(charge 2.0x, failure 2.4x, tooldup 2.0x instructions; toolgrow 1.04-1.10x).

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
- Retained vs raw: on retained keys every packed layout beats the SipHash hand (P/w 0.52-0.61x; not the Fx hand,
  see below). Raw keys pay the host encoder,
  and no layout reaches hand: P/w raw is 1.12 / 1.38 / 1.22 / 1.13x at N = 100 (sid / uuid / nonascii / surr), up to 1.43x at N = 500 (uuid).
- Host encode vs Pruv check: chk + ret below is a sum of separately measured medians, not a measured combined lane
  (enc + chk + ret does not reproduce raw exactly, e.g. nonascii P/w 14.8 vs 17.2 ns). The Pruv part (chk + ret) for P/w is 9.1 / 13.2 / 9.0 /
  8.6 ns, which is 0.58-0.67x of the hand find. The rest is the host encoder (5.7-13.8 ns for P). That encoder is a
  plain per-unit shift loop over `chunks(4)` and is slower than the U copy (6.0 ns on uuid). No faster encoder was
  measured.

#### Full-result re-run and the fast-hash hand row (P01 follow-up, `work/p01_keys/perf_p01b.txt`)

Changes: the `enc` row now folds the whole encoded buffer (`fold_words`: s = len, then s = s * 31 + w per word;
before, it observed only the length and the last word). `ret` / `miss` / `raw` / `chk` / `cmp` already read the full
result. `hand_keys.rs` takes a hasher argument (`sip` = std default, `fx` = an inline FxHasher, the rustc-hash 1.x
`(rotl(h, 5) ^ w) * 0x517cc1b727220a95` over 8 / 4 / 2 / 1-byte chunks); `run_keys.py` runs hand sip and hand fx as
two sides in the rotation and puts fx in the checksum groups. 7 rounds, N = 100 / 500, all sets; checksum groups
disagreeing: none.

enc, full fold (ns, N = 100 / 500): sid P 6.93 / 6.73, U 7.45 / 7.47; uuid P 16.27 / 16.12, U 14.52 / 14.10;
nonascii P 6.94 / 6.76, U 7.47 / 7.28; surr P 8.04 / 7.95, U 8.42 / 8.40. enc now measures encoding plus a fold of the whole
buffer inside the timing, so its cost grows with the word count of each layout (U 14 words and P 5 words for a
13-unit sid) and the order can change: sid was U 3.64 < P 5.73 before the fold and is P 6.93 < U 7.45 with it.

ret / miss / raw, P/w against both hand hashers (median ns; instructions for ret):

| set | N | P/w ret | hand fx ret | hand sip ret | P/w / sip | P/w / fx | P/w miss | fx miss | P/w raw | fx raw |
|---|---|---|---|---|---|---|---|---|---|---|
| sid | 100 | 7.68, 186 i | 5.68, 122 i | 13.97, 291 i | 0.55 | 1.35 | 4.72 | 4.38 | 15.24 | 5.92 |
| sid | 500 | 7.76, 191 i | 5.73, 130 i | 13.30, 291 i | 0.58 | 1.35 | 5.71 | 4.19 | 17.42 | 5.95 |
| nonascii | 100 | 7.85, 189 i | 5.01, 122 i | 13.98, 291 i | 0.56 | 1.57 | 4.76 | 3.20 | 16.77 | 5.25 |
| nonascii | 500 | 8.48, 191 i | 4.66, 120 i | 13.28, 290 i | 0.64 | 1.82 | 5.83 | 3.53 | 17.69 | 4.89 |
| surr | 100 | 7.77, 189 i | 5.10, 122 i | 14.57, 302 i | 0.53 | 1.52 | 5.24 | 3.33 | 17.39 | 5.40 |
| surr | 500 | 8.08, 192 i | 4.83, 121 i | 14.02, 301 i | 0.58 | 1.67 | 5.59 | 3.55 | 17.88 | 5.16 |
| uuid | 100 | 12.45, 237 i | 14.36, 277 i | 20.76, 406 i | 0.60 | 0.87 | 7.09 | 14.08 | 27.97 | 13.67 |
| uuid | 500 | 13.52, 238 i | 41.02, 717 i | 20.87, 405 i | 0.65 | 0.33 | 7.27 | 54.61 | 30.23 | 32.32 |

Result (§5 D12): **the packed-u16 lead over hand comes from SipHash.** Against a fast-hash hand map with the same
`Box<[u16]>` keys, P/w ret is 1.35-1.82x slower on sid / nonascii / surr (61-71 more instructions per lookup), miss is
1.08-1.65x and raw is 2.6-3.6x. P/w is ahead only on uuid, where Fx degrades: the 36-unit keys differ only in the last
digits, and at N = 500 hand fx ret costs 717 instructions and miss 54.6 ns, which points to collisions of the 1.x Fx
fold on such keys (the mechanism was not isolated). That is a weakness of this hasher, not a P/w win; a fast hash with a final
mix (or FxHash 2.x) was not measured. The 0.52-0.61x "beats hand" figures above are therefore against SipHash only.

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
| 1 | C1 key / id width | Key bench: U/u -> P/w ret saves 3.8-7.0 ns and 84-178 instructions per lookup. Against hand16, M8 charge has 492 and failure 520 instructions left (n = 100), of which about 300 each is the new key-unit loop | The M8 effect of P is expected but not measured; it needs the P03-P05 port |
| 2 | C4 packed argument / result boundary | M7 B - C 118-160 instructions, 0.4-3.5 ns (pair 242); wrapper A - B 104-163 | The A - B wrapper is in the driver and host, not in Pruv code |
| 3 | C3 `valid` re-check | M7 B - Bnv 58-68 instructions, 0.5-1.8 ns (pair 136) | Inside B - C; not counted twice |
| 4 | C2 accessor re-check / record | Not isolated in P01. M7 observe (view consumption) is 974 of current's 1905 instructions in every variant | Needs its own ablation before any claim |
| 5 | C7 residual frames | Key bench inline factor 0.96-1.05; the M8 inline policy is already in the baseline | No new P01 gain found |
| - | C6 safe-number work | Now inside the hand16 bar (part of the 158 / 72 instruction semantic share) | No longer a gap against the bar; its exact cost in Pruv vs `checked_add` is not split |

**Can the chosen layout reach 1.0x?** For the retained-key lookup, measured yes: P/w ret is 0.52-0.61x of the SipHash
hand map. With the external check added, the sum of separate medians is 0.58-0.67x; a combined check + find lane was
not measured, so this part is an estimate. Against the Fx hand row the same lookup is 1.35-1.82x (sid / nonascii /
surr): a hand map with a fast hash is not reached by the layout alone. Whole M8 ops are not shown at 1.0x
yet. Charge and failure need the key-unit loop off the hit path (about 300 instructions each, see the M8 section), then
C1 (P port) plus C4 / C3 for the remaining 186 / 223 instructions (n = 100); only the C1 part has a measured
per-lookup saving. Raw keys re-encoded on every call stay at 1.12-1.43x unless the host keeps packed records.

**Which checks proofs can remove:**

- `valid` on trusted resident state: 58-68 instructions per call, through the resident contracts (C3).
- `k_check_p` on keys already inside the state, through the state invariant. Its separate cost is 0.9-1.45 ns; the
  saving inside a combined lane was not measured.
- The external-key `k_check_p` stays, as required.
- No bounds-check share inside find was isolated, so no figure is claimed for it.
- Caveat: the 1.0x statements above are against std SipHash. Against the Fx hand row P/w ret is 1.35-1.82x
  (sid / nonascii / surr), see the full-result re-run above.

### Open items after the P01 follow-up (handoff to P02)

Closed in the follow-up: the malformed-input blocker (both sides reject with status 2, 69 fixed controls), full-result
consumption (key `enc` folds the whole buffer, M8 perf compares a final-state checksum between Pruv and hand16, M7
folds every effect word), the re-runs with kept outputs (`work/p01b/`, `work/m7/boundary_p01b.txt`,
`work/p01_keys/perf_p01b.txt`), the compiler agreement and version record, and the fast-hash hand row.

Still open:

- **First P02 item: the M8 key-unit regression.** The `key_end` unit loop costs about 6 instructions per key unit on
  every call (charge 695 -> 988, tooldup 596 -> 1031 instructions; Pruv/hand16 now 2.0-2.4x). Move the range check off
  the lookup-hit path (a hit equals a stored key, which `t_put` already checked) and prove it, then re-run
  `run_m8.py perf` and the 69 controls.
- M7 typed / same-packet hand comparator: deferred to P02. `hand_m7.rs` has no q mode and no differential check against
  the Pruv driver, so its numbers would not show equal semantics; P02 needs a hand16-style M7 hand with q mode and a
  `diff_hand16.py`-style edge + malformed diff before an M7 ratio counts.
- Native / hand lane apparatus for P02: a hand map with a fast hash and a final mix (the Fx 1.x row collides on uuid
  keys), and an optimized hand with the same data structure as Pruv (open addressing over packed records), so the bar
  is not set by the hasher choice.
- hand_m8.rs (old UTF-8 row) prints no checksum; it stays informational only.
- Proof removal of the `load_cv` / `bud_charge` state and reason checks under the store invariant, and of `valid` on
  trusted resident state (C3): not done, no saving measured inside M8.
- Explicit "unavailable" marks for allocation / probe counters in the key bench and M8 tables.
