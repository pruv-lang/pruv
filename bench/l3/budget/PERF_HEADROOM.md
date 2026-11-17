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

## P02: stronger hand baselines, typed M7 hand, entry fast paths (2026-11-18)

Bar: hand Rust 1.0x is the floor every op must reach; below 1.0x is the goal (proof removes checks the hand must run).
1.1x is only the alarm line. An op between 1.0x and 1.1x stays open with its cause written down. Ratios are
Pruv / lane, so > 1.0x means Pruv is slower. Kept outputs: `work/m8/p02_perf3.txt`, `work/p01_keys/p02_perf_after.txt`,
`work/m7/p02_perf.txt` and `work/m7/p02_perf2.txt`.

New lanes:

- `fxmix`: hashbrown-style map with Fx plus an fmix64 final mix. This removes the uuid collision of the Fx 1.x row:
  Fx hashes only the low bits of uuid words that differ in the high bits.
- `oa`: the same data structure as Pruv, which is open addressing over packed records with u32 slots.
- `w`: the matched wire boundary. The hand decodes the same packed words Pruv gets, using bulk unit conversion.
- M7 `hand16`: a typed hand with a q mode. `diff_hand_m7.py` checks it against the Pruv driver: 0 mismatches, plus the
  edge and malformed controls.

### M8 (`run_m8.py perf`, 7 rounds, n = 100 / 500; checksums equal on 56 / 56 runs)

Before the audit fixes (`fd0aff9`, hit-path key check removed):

| op | Pruv ns | Pruv/hand16 | Pruv/hand16w | Pruv/fxmixw | Pruv/oaw | Pruv instr | Pruv/oaw instr |
|---|---|---|---|---|---|---|---|
| charge | 31.4 / 36.1 | 1.50 / 1.45 | 0.93 / 0.91 | 1.17 / 1.13 | 1.18 / 1.10 | 737 / 740 | 0.94 / 0.94 |
| failure | 23.4 / 28.8 | 1.44 / 1.57 | 0.81 / 0.90 | 1.15 / 1.29 | 1.08 / 1.14 | 627 / 630 | 1.00 / 1.00 |
| tooldup | 29.8 / 41.5 | 1.16 / 1.64 | 0.63 / 0.82 | 0.83 / 1.13 | 1.00 / 1.13 | 783 / 796 | 0.87 / 0.88 |
| toolgrow | 80.6 / 91.9 | 0.89 / 0.93 | 0.72 / 0.75 | 0.99 / 1.07 | 1.47 / 1.72 | 1774 / 1789 | 1.32 / 1.39 |

After the audit fixes (hit-path key and record check restored, see "P02 audit findings" below):

| op | Pruv ns | Pruv/hand16 | Pruv/hand16w | Pruv/fxmixw | Pruv/oaw | Pruv instr | Pruv/oaw instr |
|---|---|---|---|---|---|---|---|
| charge | 35.1 / 37.7 | 1.55 / 1.52 | 0.99 / 0.94 | 1.25 / 1.12 | 1.26 / 1.01 | 787 / 790 | 1.01 / 1.01 |
| failure | 26.3 / 31.0 | 1.54 / 1.64 | 0.87 / 0.94 | 1.18 / 1.34 | 1.16 / 1.18 | 671 / 674 | 1.07 / 1.07 |
| tooldup | 29.6 / 41.2 | 1.05 / 1.58 | 0.60 / 0.79 | 0.81 / 1.10 | 0.98 / 1.08 | 711 / 727 | 0.79 / 0.80 |
| toolgrow | 76.0 / 86.9 | 0.79 / 0.86 | 0.65 / 0.70 | 0.91 / 0.99 | 1.33 / 1.56 | 1591 / 1607 | 1.18 / 1.25 |

Measured cost of the restored check (same machine, same session, `fd0aff9` swapped in for the before run): charge
+50 instructions (+6.8%) and +3.7 / +1.6 ns, failure +44 instructions (+7%) and +2.9 / +1.7 ns. tooldup (-72) and
toolgrow (-186) went down, because the fix also removed the `note_tool_v` / `add_absent_ck` double-check paths.

The un-suffixed hand lanes (hand16, fxmix, oa) take typed arguments, so their boundaries are not matched. Against
`oa`, Pruv is 2.0-3.3x in ns. The difference between the matched and unmatched lanes is the host-side decode of the
wire words. That decode is the C4 boundary item and is a host cost, not a Pruv code cost.

Changes in P02:

- Step 1 (`c4d3a63`): the key unit check moved off the lookup-hit path. A hit equals a stored key, and `t_put` already
  checked that key. Charge went from 988 to 737 instructions and tooldup from 1031 to 783. **Reverted by the audit
  fix**: this relied on I1 (every stored key is in range), which nothing checks. The check is back on the hit path as
  one OR-fold `units_ok` band test per key with an exact per-key fallback, and the record state / reason / word-14
  check folded into one branch, all in one pass before any write (`keys_ok_1/2/4`, `load_cv`).
- Step 3d (`5b9a9d8`): `eb()` proves `16 <= base && base + 20 <= len` once per run of entry accesses. After that,
  `fill_entry`, `entry_alloc`, `entry_free`, `put_key_h`, `drop_entry` and the `remove` child loop index `st[]`
  directly, which compiles to unchecked loads. The total `ea` / `wr` path stays as the fallback, so short or corrupt
  buffers give the same result. Toolgrow went from 1953 to 1774 instructions.

Checks erased by proof (hand16 and oa run them, Pruv does not):

- Per-field bounds checks on entry records: one `eb` range proof per run of accesses replaces about 20 checked
  accesses (step 3d).
- Probe-slot bounds checks: the table header is checked once per call (key bench step 3b), while the hand's
  `slots[p]` / `ents[s - 1]` stay checked. A header that fails the test falls back to the per-probe checked loop.

Not erased (checked at run time): the hit-path key-unit range check and the stored record state / reason / word-14
check. Real erasure needs I1 as a checked resident invariant: the state must carry "every stored key is in range and
every record is well formed" between calls (resident / host invariants, P06 / P07 / P11) in the typed-state
representation of P03-P05. That is the root fix; a cheaper skip without it changes results on corrupt buffers
(`corrupt_resident.py`: 77 mismatches on `fd0aff9`, 0 after the fix, 828 controls).

Where Pruv is below 1.0x and why:

- toolgrow against hand16 (0.79-0.86): the second and later probes need no per-slot bounds check, and the entry fast
  paths write without checks.
- tooldup against fxmixw at n = 100 (0.81) and against oaw (0.98, 0.79-0.80 instructions): one pass over the keys,
  without the old separate `note_tool_v` validation walk.

Open, with line-level causes:

- **charge, failure (oaw 1.01-1.26x ns, 1.01-1.07x instructions).**
  - The restored hit-path check (`root_m8.pruv` `keys_ok_*` and the folded record test in `load_cv`): about 45-50
    instructions per call. It stays until I1 is a checked resident invariant (see above).
  - `valid(st)` runs on every call (`root_m8.pruv` entry, about 60-70 instructions). The hand's `struct` holds its
    invariant by construction.
  - `load_cv` copies 14 words, and `store_cv` writes them back. The hand mutates the struct field in place.
  - The `c32` / `c40` clamps on counters read from the state.
  - Why Pruv cannot emit the hand code: the resident state is a `[i64]` handed in on each call. The language has no way
    to carry "this buffer is still valid" between calls (resident or host invariants, P06 / P07 / P11), so the check
    and the clamps must stay to keep the total semantics.
  - Instructions are 1.01-1.07x of oaw after the restored check; the rest of the ns gap is the dependent load chain
    of the 14-word copy.
- **tooldup at n = 500 (oaw 1.08x, fxmixw 1.10x).** Same causes as above, plus a cache effect: Pruv entries are 20
  words (160 B), oa entries are 12 B plus the arena.
- **toolgrow (oaw 1.33-1.56x, 1.18-1.25x instructions).**
  - `forget` is O(N): `drop_entry` re-probes with `slot_of` and calls `entry_free` for each child. The oa hand bumps a
    generation in O(1). This is a layout choice that the P03-P05 port can change; proof does not remove it.
  - `put_key_h` runs a second probe after `find_h`, where the hand reuses the probe position. Returning the probe
    position from `find_h` needs a two-value result, which is the C4 item.
  - The `ef` / `hd` accessors outside the fast-pathed functions still go through checked `rd`.
  - `valid()` runs on every call, as for charge.

### Key bench (`run_keys.py`, 7 rounds, P/w packed records with the matched wire boundary)

Before the audit fix (`fd0aff9`, no fallback; `work/p01_keys/p02_perf_after.txt`):

| set | ret ns (100 / 500) | Pruv/sip | Pruv/fxmix | Pruv/oa | instr Pruv / oa (100) |
|---|---|---|---|---|---|
| sid | 6.96 / 7.21 | 0.49 / 0.54 | 1.14 / 1.31 | 1.08 / 1.14 | 159 / 149 |
| nonascii | 7.09 / 7.49 | 0.51 / 0.56 | 1.27 / 1.36 | 1.11 / 1.08 | 161 / 152 |
| surr | 7.07 / 7.65 | 0.47 / 0.53 | 1.16 / 1.31 | 1.11 / 1.10 | 161 / 151 |
| uuid | 12.70 / 14.41 | 0.62 / 0.70 | 1.32 / 1.44 | 1.12 / 1.18 | 220 / 210 |

After the audit fixes (header fallback to `tab_find_checked`, plus the sb / eb <= 2^40 test from the confirm audit;
checksums equal, no hand rows missing; `p02fix2_keys_perf.txt` in the run's output folder):

| set | ret ns (100 / 500) | Pruv/sip | Pruv/fxmix | Pruv/oa | instr Pruv / oa (100) |
|---|---|---|---|---|---|
| sid | 8.11 / 7.69 | 0.58 / 0.57 | 1.32 / 1.33 | 1.25 / 1.18 | 168 / 149 |
| nonascii | 8.30 / 8.40 | 0.58 / 0.61 | 1.44 / 1.48 | 1.26 / 1.19 | 171 / 152 |
| surr | 7.67 / 8.50 | 0.51 / 0.60 | 1.24 / 1.41 | 1.19 / 1.25 | 171 / 151 |
| uuid | 13.82 / 15.17 | 0.66 / 0.72 | 1.39 / 1.45 | 1.18 / 1.21 | 230 / 210 |

`raw` (re-encode on every call) is 1.08-1.16x of oa raw. `miss` is 1.08-1.24x of oa.

Open (regression, blocking for P03): the retained-key find went from 1.08-1.18x to 1.18-1.26x of oa, +9-10
instructions per lookup (sid 159 -> 168, uuid 220 -> 230). The cause is the fallback itself: the fast loop must keep
`t`, `k`, `h`, `hw` live for the cold `tab_find_checked` call, there are three call sites (two header tests, one
`w > cap` probe test), and the header needs the extra or-fold `bor(sb, eb) >= 2^40` test (+3 instructions; without it
a table longer than 2^40 words gave 0 where the checked loop gives -1). A single cold exit (flag + one call after the
loop) is the next thing to try; it was not built in this budget. The root fix is the same as for M8: a checked
table invariant carried between calls (P06 / P07 / P11) drops the header test and the fallback.

Line-level diff, `keys.pruv` `tab_find` against `hand_keys.rs` `find_h`:

- `keys.pruv:466-477`: the header is loaded from `t[0]`, `t[3]`, `t[4]` and `t[5]`, followed by 8 range compares on
  every call; a failing cap / sb / eb test runs `tab_find_checked` (`keys.pruv:426`). The hand reads
  `self.slots.len()` and `self.mask` from a struct whose fields are consistent by construction.
- `keys.pruv:489-491`: each probe tests `w >= 1`, then `w > cap` exits to the checked loop. The hand indexes
  `self.ents[s - 1]` with one bounds check, which it also pays.
- `ent_match` compares the stored hash and length, then the words. This is the same work as the hand's
  `e.hash == h && e.len == k.len()` plus the zip compare.

Pruv cannot emit the hand code because `t` is a plain `[i64]` argument, and the table header invariant cannot be
carried between calls (P06 / P07 / P11). Once it can be, `keys.pruv:476-477` and the fallback drop, and the find should go below oa,
because the per-probe `ents` bounds check in the hand has no counterpart in Pruv.

### M7 (`run_m7.py perf`, typed hand16, S = 100)

Before step 3d (`work/m7/p02_perf.txt`):

| item | Pruv ns | hand16 ns | Pruv/hand16 | Pruv instr | hand16 instr |
|---|---|---|---|---|---|
| attach | 21.4 | 12.8 | 1.67 | 741 | 303 |
| closereopen | 28.8 | 13.5 | 2.13 | 998 | 341 |
| current | 25.8 | 12.7 | 2.03 | 930 | 342 |
| ishalted | 26.2 | 12.4 | 2.11 | 895 | 335 |
| replace | 119.9 | 125.9 | 0.95 | 3286 | 2999 |

The hand16 perf lane is typed. It takes typed arguments and returns references, with no out words, so its boundary is
not matched. The table below splits the Pruv driver variants in instructions per call at S = 100:

| item | A (World::call) | B (t_call) | Bnv (no valid) | C (typed root) | observe | hand16 |
|---|---|---|---|---|---|---|
| attach | 987 | 882 | 811 | 762 | 245 | 280 |
| current | 1916 | 1771 | 1713 | 1615 | 986 | 330 |
| ishalted | 942 | 837 | 779 | 677 | 44 | 348 |
| closereopen | 1049 | 905 | 837 | 784 | 51 | 356 |
| replace | 3760 | 3597 | 3529 | 3478 | 474 | 2993 |

After step 3d, `id_is` uses a branchless or-fold that LLVM vectorizes, which stands in for the hand's slice `bcmp`. The
ishalted C lane dropped from 677 to 540 instructions, and current from 1615 to 1474.

After step 3d (`work/m7/p02_perf2.txt`, S = 100 / 500):

| item | Pruv ns | hand16 ns | Pruv/hand16 | Pruv instr | hand16 instr |
|---|---|---|---|---|---|
| attach | 21.1 / 21.0 | 11.6 / 11.6 | 1.82 / 1.81 | 741 / 741 | 319 / 303 |
| closereopen | 24.6 / 24.1 | 13.4 / 13.1 | 1.84 / 1.84 | 894 / 894 | 342 / 342 |
| current | 19.5 / 19.4 | 12.2 / 12.4 | 1.60 / 1.56 | 728 / 728 | 331 / 331 |
| ishalted | 19.9 / 19.8 | 12.4 / 12.5 | 1.60 / 1.58 | 730 / 730 | 364 / 335 |
| replace | 120.0 / 121.0 | 117.9 / 120.2 | 1.02 / 1.01 | 3286 / 3286 | 2999 / 2999 |

Before and after 3d, Pruv went from 895 to 730 instructions on ishalted, 930 to 728 on current and 998 to 894 on
closereopen. Pruv ns for replace is unchanged (119.9 -> 120.0, 3286 instructions both times). The hand16 replace
median moved from 125.9 to 117.9 ns between runs, so replace is now at 1.01-1.02x.

After the audit fixes (per-op result checksum `rchk` per cell, Pruv equals hand16 on all 10 cells; S = 100 / 500):

| item | Pruv ns | hand16 ns | Pruv/hand16 | Pruv instr | hand16 instr |
|---|---|---|---|---|---|
| attach | 22.5 / 21.7 | 13.1 / 11.8 | 1.72 / 1.84 | 740 | 300 |
| closereopen | 25.7 / 24.7 | 13.3 / 13.0 | 1.93 / 1.90 | 893 | 342 |
| current | 19.5 / 19.3 | 12.9 / 12.2 | 1.51 / 1.58 | 727 | 331 |
| ishalted | 20.1 / 20.2 | 12.3 / 12.2 | 1.63 / 1.66 | 729 | 336 |
| replace | 123.3 / 121.7 | 123.5 / 127.8 | 1.00 / 0.95 | 3283 | 3010 |

M7 Pruv code is unchanged by the audit fixes; instruction counts match the step-3d run. `rawcall` is 4.6 / 4.5 ns.

- Open: replace is 0.95-1.02x in ns across runs and 1.09-1.10x in instructions. This is in the 1.0-1.1 band. The hand allocates
  `Box<[u16]>` keys and hashes them with SipHash; Pruv copies words into the arena it already owns. Pruv still runs
  more instructions: about 290 per call more than the hand, made up of the same boundary costs listed below (op build,
  `t_call` parsing and `valid`, about 230-310 together).
- Open: attach, closereopen, current and ishalted are 1.51-1.93x. The causes, by variant split:
  - The op build (A - B) costs 100-145 instructions. It is in the driver.
  - `t_call` key parsing (Bnv - C) costs 50-100 instructions. This is C4: the packed boundary has to decode `key_end`.
  - `valid()` (B - Bnv) costs 60-70 instructions. This is C3; the resident invariant cannot be carried between calls,
    as for M8.
  - A C-lane residual remains. For ishalted it is about 494 instructions after the observe fold, against 348 for the
    hand. It is not attributed line by line yet. The hand compares a 36-unit id over u16 (72 B) with one `bcmp`,
    while Pruv stores one unit per i64 word, so it touches 288 B. Packing units 4 per word (the C1 P layout,
    P03-P05) is the expected fix.

### P02 audit findings: status after the fix round

Fixed in P02: `drop_entry` read the key and blob lengths through the cached base after a slot write that, on a corrupt
buffer, can land on header word 5 or 6. It now falls back to `gf` in that case. The PERF_HEADROOM key-bench
Pruv/sip column and the raw range were also corrected against `work/p01_keys/p02_perf_after.txt`.

The six blocking findings of the first audit:

- **Fixed: M8 hit-path key check (`c4d3a63`).** The check is restored on the hit path (see the M8 section): OR-fold
  `units_ok` with one band test, the record state / reason / word-14 test in one branch, one pass before any write.
  `corrupt_resident.py` compares the new code with `c4d3a63^` on 828 corrupt-resident controls (key unit 65536,
  state 3, word-14 mismatch and others): 0 mismatches; on `fd0aff9` the same controls give 77. `diff_hand16`: 300
  random cases and 69 malformed controls, 0 mismatches. Cost: charge +50, failure +44 instructions. This is not
  erasure by proof; erasure needs I1 as a checked resident invariant (P06 / P07 / P11 with the P03-P05 typed state).
- **Fixed: key bench `tab_find` header check (`keys.pruv`).** A failing header, or a stored id above `cap` met in
  the probe loop, runs `tab_find_checked`, the old per-probe checked loop, for the whole call. `run_keys.py check`
  adds 104 corrupt-header controls (`t[3]` = 0, 1, cap - 1, -1, 2^31; `t[4]` / `t[5]` = len or -1; bad slot
  count; a stored id of cap + 1) compared against
  `12c78a4^`: 0 mismatches (32 / 104 on `fd0aff9`). A missing Pruv row is now an error. Confirm audit: a table
  with `eb` > 2^40 (len(t) > 2^40) passed the fast header and returned 0 where the checked loop returns -1; the
  fast path now also sends `bor(sb, eb) >= 2^40` to the checked loop (static code path; such a table cannot be
  allocated in the control). Perf cost: open, see the key bench section (+9-10 instructions per find).
- **Fixed: `perf_budget.py`.** Every result line is validated against the oracle (results and checksum); `{}` and
  lines with missing fields are rejected. Axes follow the spec profile: 100 / 500 for sessions and calls, 10 / 50 for
  runs, delegations and clocks. Implementations alternate per rep; min, median and spread are kept.
  Confirm audit: the P6 early-terminal class was still accepted (a first request ended on `ResourceBound(input)`
  skipped every comparison, final state included), and a run without `--impl` stopped after one TS rep; now only
  `exact` is accepted and a no-impl run does `repeat` TS reps. `perf_budget_controls.py`: 14 / 14 pass (one per rule,
  including P6, final state only, a wrong result on a later rep only, and no-impl repeat).
- **Fixed: M7 perf result equality.** Driver and hand16 print a per-op result checksum (`rchk`) from an untimed pass;
  `run_m7.py perf` compares it per cell and marks a mismatch or a missing line `FAIL` with no ratio and exit 1.
  Confirm audit: values were pooled into a set, so a line missing from one round only passed; each lane must now
  print exactly one value in every round. `rchk_control.py` (tampered, dropped, dropped in round 1 only): 3 / 3 pass.
- **Fixed: `q.py` / `run_meta.py`.** Backend and flags come from the build command (`--backend`, `--build-flags`) or a
  `<impl>.build.json` sidecar next to the binary; otherwise they are recorded as `unknown`, never a default. Confirm
  audit: with an `impl=` override (`run_m5.py`) the sidecar was read from `--impl`, not the binary run; `q.py` and
  `perf_budget.py` now pass the argv actually run (control 10).
- **Fixed (noise): M7 `rawcall` at n = 500.** Five re-runs gave 5.0, 4.7, 8.2, 4.6 and 4.7 ns (median 4.7) at an
  unchanged instruction count; the fix-round run gives 4.5. The 9.1 ns cell was a noisy round, not a regression.

Still open after the confirm audit:

- Performance: M8 charge / failure +50 / +44 instructions (restored check) and the key find +9-10 instructions
  (fallback). Both are recorded with causes above; the root fix is the checked resident / table invariant (P06 / P07 /
  P11 with the P03-P05 typed state).
- `corrupt_resident.py` changes stored key units without updating the stored hash, so its "hit" variants are real
  misses (`bud_map.pruv` hash compare); the restored hit-path check is shown by code reading and the state / reason /
  word-14 controls, not by a corrupted-key hit. A control that rewrites the hash with the key is the next step.

Non-blocking:

- (fixed) `run_keys.py` skipped the comparison when a Pruv row was missing.
- The key checksum is a sum, so it cannot see a permutation of the returned ids.
- The diff harnesses can reuse a stale Pruv binary.

## P03: packed-u16 key codec `bud_keys.pruv` (2026-11-18)

New source-only module, no language extension. A key of n UTF-16 code units (0..65535; NUL and lone surrogates are
ordinary units, 0 <= n <= 2^30) is m = ceil(n / 4) i64 words, big-endian lanes (unit 4i in bits 48..63 of word i),
zero tail. Record form `[n, w0 .. w(m-1)]`. Public functions: `max_units`, `words`, `slice_ok`, `rec_end`, `unit`,
`mix`, `fin`, `hash(seed, k, ks, n)`, `eq`, `cmp`, `pack_units`, `unpack_units`. Every public function is total
(clamps and 0-reads outside the buffer, as in `ent_u16`), so callers need no preconditions. `words(n)` clamps n to
0..2^30 first (negative -> 0, above the cap -> words(2^30)); its contract states the exact value in range and both
clamp results. `eq(a, sa, b, sb, n)` is an equal-length helper: it compares the words(n) words only, and the caller
compares both unit counts first.

What the layout removes, by representation rather than by a check:

- **Per-unit range check.** Every 16-bit lane is a valid code unit, so `units_ok` (one band test per unit) has no
  counterpart. Canonicality of an external key is O(1): n range, slice range and the lanes past n in the last word
  (`slice_ok` / `rec_end`). The old `key_end` in `root_m4.pruv` loops over every unit.
- **Per-unit hash / compare loops.** Hash mixes once per word, then `fin(bxor(h, n))`; equality is the caller's n compare
  plus `eq` over m words; unsigned word compare equals code-unit order, ties broken by n. `"a"` and `"a\0"` have equal words, so
  n is always compared.

Gate (`m34_test/run.py keys --seed 20261117 --count 300`, Pruv binary vs Python oracle): **4893 / 4893 pass**
(4848 at 0f3df19; the audit-fix commit added the `words` op and the cmp / eq fallback and clamp rows).
Fixed negatives: NUL at start / middle / end and `"a"` vs `"a\0"`, lone high / low surrogates and reversed pairs,
astral pairs, 0x8000 vs 0x41 (unit order against signed word order), all-0xFFFF, tail lengths n mod 4 = 0..3,
nonzero lanes past n (canonicality negatives for r = 1, 2, 3), out-of-range units in `pack_units` (status 1, dst
untouched), bad slice ranges (status 2), n clamps, `words` on negative / in-range / above-cap n, and cmp / eq with
short buffers (fallback reads), negative lengths and negative or > 2^40 starts. Random rows: one seed, 300 random rows over hash / eq / cmp / ok / rec / unit /
pack / unpack, with word compare asserted equal to unit-list compare on canonical keys.

Regression gates (product source unchanged in P03, so these confirm nothing else moved): Q2 (`run_m8.py q`, 300):
exact 63, not-implemented 237, fail 0, same as P02. Q6 (`run_m11.py --pairs rs`, 300): exact 213, prefix-ok 87, same
as P02. Q8 (`run_m6.py --pairs rs`, 300): 300 / 300 exact.

Per-op S/M: unchanged by construction (no root links `bud_keys` yet). The P02-end numbers (`work/base/`, commit
`793fff6`) are the "before" column for P04 and P05:

| M8 item | n | Pruv ns | /hand16 | /fxmix | /oa | Pruv instr | /hand16 | /fxmix | /oa |
|---|---|---|---|---|---|---|---|---|---|
| charge | 100 | 32.9 | 1.55 | 2.35 | 2.19 | 787 | 1.55 | 2.09 | 1.98 |
| charge | 500 | 37.8 | 1.57 | 2.29 | 2.10 | 790 | 1.55 | 2.10 | 1.97 |
| failure | 100 | 25.5 | 1.56 | 2.50 | 2.28 | 671 | 1.87 | 2.73 | 2.54 |
| failure | 500 | 30.3 | 1.67 | 2.68 | 2.42 | 674 | 1.88 | 2.75 | 2.52 |
| tooldup | 100 | 29.1 | 1.08 | 2.05 | 2.51 | 711 | 1.27 | 1.93 | 2.40 |
| tooldup | 500 | 40.1 | 1.62 | 3.08 | 3.23 | 727 | 1.31 | 2.00 | 2.43 |
| toolgrow | 100 | 73.2 | 0.81 | 1.15 | 2.07 | 1592 | 0.81 | 1.15 | 2.17 |
| toolgrow | 500 | 85.2 | 0.85 | 1.35 | 2.66 | 1608 | 0.77 | 1.13 | 2.39 |

M7 (S = 100 / 500, Pruv/hand16 ns; instructions Pruv vs hand16): attach 1.73 / 1.93 (740 vs 300), closereopen
1.87 / 1.98 (893 vs 350), current 1.56 / 1.55 (727 vs 332), ishalted 1.58 / 1.60 (729 vs 336), replace 0.93 / 0.97
(3283 vs 2974). All cells above 1.0x stay open with the causes listed in the P02 section.

Erased checks in P03: none in the product yet. Inside the codec, the verifier proves every array access of the
fast-path loops in range, so those loops carry no runtime bounds checks; each call does one entry range test to pick
the fast path. The fallback path (slice partly outside the buffer) reads through `ent_u16::rd`, which keeps its range
branch per read.

### I1 finding (applies to P03-P05; input for P06 / P07 / P11)

I1 (per-entry spans and key encodings valid for every live entry) cannot be carried by the current language, so the
per-call `valid()` scan (~60-70 instructions) and the M8 record state / reason / word-14 checks stay. Precise gaps:

1. User-defined predicates cannot be called in contracts: `pruvc/check.py:1785-1788` rejects a user function call
   in `requires` / `ensures` / `invariant` ("not supported in v0.1", hint "inline the property"), and
   `pruvc-rs/crates/pruv-core/src/check.rs:3613` raises the same `call_in_contract`. Builtins (`len`, `ushr`, ...)
   and `forall` are allowed, so I1 could only be written by inlining `slice_ok` as a forall over live entries with a
   variable-offset tail mask per entry; that is the missing piece (a named, reusable predicate), not quantifiers.
2. No type invariants or field refinements: nothing carries "this buffer is a valid store" from one entry call to
   the next, and there is no `[u16]`. Canonicality (O(1) per key with packing) is a separate fact from the per-unit
   range, which packing already removed by representation.
3. Observed ABI, not a language limit by itself: each entry is a public safe fn that takes an arbitrary `[i64]` from
   the host, so the callee must assume nothing about it. Restricting who may call with an unchecked buffer
   (visibility / private adapter) is the P06 / P09 work.
4. After an `inout` call the verifier knows fresh same-length contents plus only the callee's `ensures`
   (`pruvc/verify.py:1958`, `inout_finals`), so an I1 fact would need a frame clause in every mutator's `ensures`.
5. `eq_range` returns a fresh unconstrained bool in the proof (`pruvc/verify.py:1822-1829`), so "a hit means the
   stored key equals the external key" is not provable; it holds by the code's construction.

The core blocker: every entry call receives an unproven host buffer, so `valid()` has to re-check it on each call.

Not patched around: no host-side flag or cached "valid" bit was added. What P03-P05 can erase without I1 is the
per-unit work (representation), not the per-call validity scan.

## P04: packed keys inside `bud_map.pruv` (2026-11-18)

Scope (spec §6.2 row P04): map storage, hash and equality move to the P03 codec; the wire, id blobs, roots' wire
decoding, drivers and hand baselines are unchanged (P05 owns them). No new interner, no language extension.

What changed:
- Arena keys are packed words (`bud_keys` big-endian lanes, zero tail). Entry field 5 keeps the unit count n; field 18
  (key region capacity), header word 14 (live words) and `drop_entry` count `words(n)`. A freed entry's key region is
  reused when `words(n)` fits.
- `hash_key` = `bud_keys::hash_units(seed', units)`: the units are hashed exactly as their packed form (one mix per
  word, then `fin(bxor(h, n))`), so the stored hash (field 6) is the word hash of the stored key.
- `entry_matches` compares the stored words against `pack4` of the caller units (in-range fast path, `tail_word` for
  the last 1-3 units) and falls back to `units_word` / `rd` when either range is short. n > 2^30 never matches.
- `copy_node` (grow / compaction) copies the packed words directly and reuses the stored hash when owner and owner
  generation are unchanged; otherwise it rehashes the stored words with `bud_keys::hash`. The old path re-hashed the
  key units of every copied entry.
- `key_unit` extracts a lane (`bud_keys::unit`). Unit values outside 0..65535 keep their low 16 bits on write; the wire
  decoders already reject them, so no gate input reaches that path.
- Roots m4 / m7 / m8 / m9 / m10 / m11 now declare `mod bud_keys;` (and `run_m11.py` copies it); the codec has one
  copy. `bud_keys` gained `pack4`, `units_word`, `tail_word`, `hash_units` and a `words(n) == (n + 3) / 4` ensures
  (the loop bound proof in `entry_matches` uses division).

Gates (seed 20261117, 300 cases): keys 5823 / 5823 (adds 630 `hash_units` rows equal to hash-of-pack and 300 random);
M4 300 / 300 (grow formula 1887 / 1887); Q2 `run_m8.py q` exact 63 / not-impl 237 / fail 0; Q1 `run_m7.py q` 113 /
187 / 0; Q3 `run_m9.py q` 26 / 274 / 0; `run_m10.py q` 78 / 222 / 0; Q6 `run_m11.py --pairs rs` exact 213 /
prefix-ok 87. All equal to P03. M8 perf: 56 / 56 checksums equal across hand16 / fxmix / oa and their w lanes.

### M8 after P04 (`run_m8.py perf`, 7 rounds, n = 100 / 500)

| M8 item | n | Pruv ns | /hand16 | /fxmix | /oa | Pruv instr | /hand16 | /fxmix | /oa | instr before (P03) |
|---|---|---|---|---|---|---|---|---|---|---|
| charge | 100 | 32.0 | 1.51 | 2.29 | 2.13 | 844 | 1.66 | 2.24 | 2.12 | 787 |
| charge | 500 | 35.8 | 1.46 | 2.20 | 2.00 | 846 | 1.67 | 2.24 | 2.11 | 790 |
| failure | 100 | 25.0 | 1.53 | 2.38 | 2.19 | 729 | 2.03 | 2.96 | 2.75 | 671 |
| failure | 500 | 28.5 | 1.52 | 2.50 | 2.30 | 731 | 2.04 | 2.98 | 2.73 | 674 |
| tooldup | 100 | 31.2 | 1.19 | 2.20 | 2.71 | 804 | 1.44 | 2.19 | 2.72 | 711 |
| tooldup | 500 | 39.3 | 1.55 | 3.02 | 3.14 | 807 | 1.45 | 2.23 | 2.70 | 727 |
| toolgrow | 100 | 76.8 | 0.84 | 1.19 | 2.21 | 1773 | 0.91 | 1.28 | 2.42 | 1592 |
| toolgrow | 500 | 86.8 | 0.86 | 1.36 | 2.76 | 1758 | 0.84 | 1.24 | 2.61 | 1608 |

M7 (S = 100 / 500, Pruv/hand16 ns; Pruv instr): attach 1.75 / 1.79 (744 / 740), closereopen 1.81 / 1.83 (895 / 894),
current 1.53 / 1.49 (727), ishalted 1.55 / 1.55 (729), replace 0.95 / 0.95 (3283). Instructions equal to P03 within
4; ns within run noise. M4 core (n = 100 / 500 / 1000): sessions 13.3 / 13.6 / 14.3 (P03 13.7 / 13.6 / 13.6), tools
29.5 / 34.7 / 33.6 (P03 28.6 / 31.8 / 30.8), clocks 26.4 / 24.5 / 25.9 (P03 27.7 / 30.2 / 27.5); verdict FLAT, NOT
SLOWER than TS.

### Open: P04 regression on the unit wire (blocking until P05)

M8 instructions rose 57-63 per call on charge / failure, 80-93 on tooldup and 150-181 on toolgrow; ns stayed within
noise except tooldup 100 (+7%) and toolgrow (+2-5%); M4 tools core +3-9%. Line-level cause (generated Rust in
`work/m8/gen.rs`, `bud_keys_10_hash_units`, `bud_map_13_entry_matches`):
- The wire still carries one i64 per unit, so every lookup packs the caller key twice: once in `hash_units` and again
  in `entry_matches`. Each packed word is 4 loads + 4 `& 0xFFFF` + 3 shifts + 3 ors (`bud_keys.pruv` `pack4`). P02
  hashed with an unmasked mul/add pack and compared with one `eq_range` slice compare (memcmp on units).
- The four masks are needed for total semantics (units outside 0..65535 must not corrupt neighbouring lanes); the
  units were validated by the wire decoder in the same call, but no contract can carry "every unit of this slice is
  in 0..65535" into `bud_map` (I1 gaps 1, 2 in the P03 section: no named predicate in contracts, no refined slice type).
- toolgrow also writes keys through `put_key_units` (pack per word) instead of a unit copy.
- `tail_word` (in-range tail, no `rd`) removed 5-17 of the extra instructions; the rest is the representation mismatch.
- The compare-side packing runs only on candidates whose stored hash, owner and length match, not on every probe.
- P04 audit: `put_key_units` already checked the whole source range, yet `units_word` re-checks range and tail per
  word; a full-word direct-read loop plus one `tail_word` removes that with existing features. Not done in P04
  because P05 deletes the unit insert path entirely; if P05 slips, this is the first partial fix.
- P04 audit: `bud_tools.pruv` / `bud_world.pruv` reserve key room in units, so Grow can be asked for more than the
  packed words need (over-reservation only, accounting is exact). P05 switches the key reservation to `words(n)`.

Root fix: P05 moves the wire and id blobs to packed keys. Lookups then hash the caller words directly (`bud_keys::hash`,
one mix per word, no packing, no masks) and compare with `bud_keys::eq` (one word-slice compare over ceil(n/4)
words), which is below both the P03 baseline and the per-unit hand16 key compare. These cells stay open until P05
lands and is measured; P04 alone does not meet the "no regression" bar on instructions.

### Threat model and probe budget

- Hash: fixed seed 1469598103934665603 mixed with owner and owner generation, one 64-bit multiply mix per packed word,
  splitmix64 / murmur3 finalizer. It is not keyed: a caller who controls key text can precompute colliding keys and
  push one owner's lookups to O(entries in the index). Keys come from the app's own session / tool / delegation ids,
  not from remote input, so this is accepted for the budget store; a keyed seed (per-store random word in the header)
  is the fix if keys ever become remote-controlled. Concrete construction (P04 audit, reproduced with 128 distinct
  keys): for an owner start hash H, any two packed words `x, mix(H, x)` of length 8 reach the same state, so every
  such key hashes to `fin(8)`. The exposure is CPU time only (no memory growth beyond the normal entry cost, no
  wrong answers): the trust boundary is "whoever can choose key text for one owner can slow that owner's map".
- Probe budget: every single probe loop in `find_h` / `put_key_h` / `slot_of` is bounded by the slot count S
  (termination does not depend on contents). This bounds one search, not one operation: `add_absent` is a lookup
  plus an insert search plus a possible rebuild, and a rebuild reinserts every live entry, so under a colliding key
  set one operation is O(S + E x S), not O(S). Rebuilding never resolves collisions (same seed, same hashes); only a
  keyed seed would.
- Load: header[13] counts live slots plus tombstones; `make_room` rebuilds when used + new entries > 3S/4 (and also
  when entry or arena room runs out), so load including tombstones stays <= 0.75. For random keys the expected
  linear-probe lengths at alpha = 0.75 are about 2.5 (hit) and 8.5 (miss) slots. Each probe compares the stored hash
  first; the key words are read only on a full 64-bit hash match.
- Packing does not change collision behaviour: equal packed words imply equal unit lists for valid units, and n is
  compared separately (`"a"` vs `"a\0"`).

### Checks erased and added in P04

Erased by proof: the per-unit `rd` range branch in the hash and compare fast paths (both now one entry range test,
then unchecked loads), and the per-unit `rd` in the in-range tail (`tail_word`). Grow copies no longer re-read and
re-hash every key unit (stored hash reused). Added: `n > 2^30` reject in `entry_matches` / `put_key_units`, the
packed region range test (`ko + words(n) <= len(st)`), and the arena-end test for a new key region. The per-call
`valid()` scan and record checks are unchanged (I1).
