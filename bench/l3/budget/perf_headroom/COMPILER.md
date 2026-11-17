# perf_headroom part B: generated-code items (2026-11-17)

Bar: equivalent hand-written idiomatic Rust doing the same work (AGENTS.md perf rules; > 1.5x must be profiled and
fixed or explained). Runner and baselines: `compiler/` (see Method).

## Result

Median of 5 alternating rounds, each run min of 3 reps after 1 warm-up, ns per call. "Pruv" is py / rs compiler
output (sol.rs byte-identical). "hand" is the fastest same-work hand Rust writing (named in brackets); the idiomatic
`String` writing is listed in the cause column when it differs. Pruv/hand uses the slower of py / rs.

| item | n | Pruv ns (py / rs) | hand Rust ns | TS ns | Pruv/hand | cause | status | before->after |
|---|---|---|---|---|---|---|---|---|
| p_int (String(int)) | 100 | 15.4 / 15.8 | 15.4 [stack units] | 107 | 1.03 | none; hand `to_string` 29.6, `Vec` units 33.8 | ok (stack result from KD-M6-3) | unchanged |
| | 500 | 15.1 / 14.4 | 13.9 | 40.5 | 1.09 | | | |
| | 1000 | 14.0 / 13.8 | 13.1 | 23.8 | 1.07 | | | |
| p_iso (ISO date) | 100 | 30.4 / 30.4 | 29.2 [Vec units] | 373 | 1.04 | none; hand `format!` 145-183 | ok | unchanged |
| | 500 | 27.5 / 29.4 | 30.6 | 335 | 0.96 | | | |
| | 1000 | 25.0 / 27.2 | 31.8 | 335 | 0.86 | | | |
| p_ref (refusal text) | 100 | 39.6 / 38.8 | 40.4 [Vec units] | 240 | 0.98 | none; hand `format!` 159-190 | ok | unchanged |
| | 500 | 38.5 / 36.6 | 43.8 | 81 | 0.88 | | | |
| | 1000 | 34.5 / 36.6 | 39.5 | 60 | 0.93 | | | |
| p_amt (amount_text "used/limit") | 100 | 32.9 / 29.6 | 33.8 [fresh Vec units] | 52 | 0.97 | allocation + copy: 4 heap Vecs (`one`, `limit_text`, 2 `cat`) and 2 copy passes | fixed (source: one sized allocation, written in place) | 85.0 / 81.2 -> 32.9 / 29.6 |
| | 500 | 28.5 / 28.2 | 29.3 | 29.0 | 0.97 | hand `format!` 56-62; String reused across calls 19-21 (not same work, reference only) | | 78.8 / 80.5 -> 28.5 / 28.2 |
| | 1000 | 29.8 / 26.2 | 30.4 | 26.8 | 0.98 | | | 75.9 / 79.4 -> 29.8 / 26.2 |
| p_wr (ent_u16 rd/wr guard, 64-unit `put_all` copy) | 100 | 6.7 / 7.1 | 7.1 [index loop, bounds checks] | 154 | 1.00 | guard is a predictable branch; guard-free sol.rs no faster | won't-fix (no cost) | unchanged |
| | 500 | 6.8 / 6.5 | 6.9 | 92 | 0.99 | `copy_from_slice` 7.5 | | |
| | 1000 | 6.6 / 6.6 | 6.1 | 105 | 1.08 | | | |

Before (p_amt, 5 rounds, same runner, old `amount_text`): hand fresh `Vec` units 26-28, `format!` 54-55, TS 26-52 ns,
so Pruv/hand was about 2.9-3.0x. No item is above 1.5x now; the largest ratio is 1.09 (p_int n = 500, within the
round-to-round spread of about ±10%).

## Fixes

- **p_amt (fixed, source change in `bench/l3/budget/bud_text.pruv`).** Profile by construction of the generated
  Rust: the old body was `cat(cat(int_text(u), one(47)), limit_text(..))`: `int_text` went to the stack buffer
  (KD-M6-3), but `one(47)`, `limit_text` (heap `int_text`), and two `cat`s each allocated, and each `cat` copied
  its inputs. Cause class: allocation (4 heap allocations and frees per call) and copy (2 passes). The new body
  computes `int_len(u) + 1 + (int_len(limit) | len_nl(lang))`, allocates once and writes with `put_int`, `ent_u16::wr`
  and `put_nl`, the same single-allocation shape `refusal` and the generated templates already use. Generated Rust
  is identical from both compilers. Checked by `m6_test/run_m6.py`: py/rust and rs/rust 300/300 with sol.rs
  identical, and `--count 3000` 3000/3000 each (text.amount, text.limit and text.refusal 110-111 cases each,
  including unlimited in both languages and negative values).
- **ent_u16 rd/wr guards (won't-fix).** `rd`/`wr` are total: `if i < 0 || i >= len(a)` then `get_unchecked`. The
  guard executes in every iteration (an instruction count shows 18-24% of the loop's instructions), but it is
  a never-taken, perfectly predicted branch next to the load/store, so it costs no time: a hand-edited copy of the
  generated sol.rs with all rd/wr guards removed measured the same as the guarded one on p_wr, p_int and p_iso
  within noise (p_ref noisy, no gain), and the guarded loop matches hand Rust's bounds-checked index loop
  (0.99-1.08) and `copy_from_slice` (Pruv is slightly faster). Cause class would be "bounds/total guard", but there
  is no measurable cost, so the guard stays (it is what makes `rd`/`wr` total).

## Open (plans, not done)

1. **Compiler-general `cat`-chain fusion.** The p_amt fix is a source rewrite. The compiler could do it in general:
   a `return cat(cat(a, b), c)` (or a chain of `let t = cat(..)` used once) whose leaves are fresh results of
   functions with a single returned `alloc` could be lowered to one `Vec::with_capacity(len(a)+len(b)+len(c))` plus
   `extend_from_slice`, removing the intermediate vectors. It needs the leaf lengths before the copies (call the
   leaves first, which is already the evaluation order) and must stay identical in pruvc and pruvc-rs, with runtime
   tests pinning the fused and not-fused shapes. Not started: no hot item needs it after the source fix, and the
   other budget text helpers already use the sized template shape (`gen_text.py`).
2. **amount_text on the stack.** With `ensures len(result) <= 35` on `amount_text` (17 + 1 + 17), the KD-M6-3
   small-result path would put the result on the stack at call sites that only read it, matching the hand stack
   writing (p_int shows 13-15 ns for that shape). `int_len` already ensures `result <= 17`; `len_nl` (generated by
   `m6_test/gen_text.py`) only ensures `result <= 1000` and would need `<= 17` (it returns 5 or 8). Small, but it
   changes the template generator's contracts, and no item is above the bar, so it is left as a plan.

## Method

- `compiler/run_compiler_perf.py [--rounds 5] [--keys py,rs] [--no-build] [--json out]` copies
  `m6_test/root.pruv` + `compiler/perf_extra.pruv` (adds `p_amt` and `p_wr`) and the bud modules to
  `bench/l3/budget/work/perf_compiler/`, compiles with the Python compiler and the Rust compiler (`PRUVC_RS`, here
  the worktree `pruvc-rs/target/release/pruvc`), checks the two sol.rs are byte-identical, and builds
  `compiler/pruv_harness.rs` and `compiler/hand.rs` with `rustc -O --edition 2021`.
- Each round runs every item and n for Pruv py, Pruv rs, each hand writing and Node one after another, so load
  changes hit all of them; each run is 1 warm-up + 3 reps, min. The table is the median over rounds. The machine
  was under heavy load (other builds); round-to-round spread was about ±10%.
- Same work: identical inputs, n, loop, input alternation (+0/+1) and consumption (len + parity of first, middle and
  last unit). The hand `_u` writings produce the same `Vec<i64>` of UTF-16 units with Rust's normal bounds checks and
  their checksums equal Pruv's and Node's. The idiomatic `String` writings (`to_string`, `format!`) count UTF-8
  bytes, so p_ref's checksum differs there. `p_amt_reuse` keeps one `String` across calls and is excluded from the
  bar (a fresh result per call is the work Pruv does).
- TS: `m6_test/node_ref.mjs` perf ops for p_int / p_iso / p_ref, `compiler/ts_extra.mjs` for p_amt (template
  literal) and p_wr (array copy loop).
