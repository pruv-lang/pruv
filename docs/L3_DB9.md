# L3 DB9: supervisor perf debts (D-R10-1..4, D-DB1-7, D-DB1-8)

Target: every supervisor cell at or below hand Rust 1.0x on time and RSS.

Result:
- Core rs/rust time is closed, at 0.931-0.987x.
- Core rs/llvm time went from 1.10-1.25x down to 0.999-1.025x.
- Core RSS is 1.001-1.012x and the whole lane is still over 1.0x. These are listed as D-DB9-1..4.
- What is left is in the compiler, the LLVM backend and the `ev_` runtime, which are outside this lane. Those are G-DB9-1 and G-DB9-2.

## Status

Status: DB9: D-R10-1 partial. `put_cp` is gone. ASCII is written inline in `push_dec` and longer code points go through `enc`. Core rs/llvm time is now 0.999-1.025x (was 1.20-1.51x in R10 and 0.94-1.25x in DB1). The rest is D-DB9-1.
Status: DB9: D-R10-2 partial. Core rs/rust chunk=65536 time is 0.962-0.987x (was 1.011-1.024x), so the time part is closed. RSS is 1.001-1.011x (D-DB9-2).
Status: DB9: D-R10-3 partial. Whole lane time is rs/rust 0.941x (M) and 1.116x (L), and rs/llvm 1.076x (M) and 1.154x (L). It was rs/rust 1.07-1.10x and rs/llvm 1.22-1.30x (D-DB9-3).
Status: DB9: D-R10-4 partial. Whole lane RSS is 1.015-1.053x (was 1.018-1.067x) (D-DB9-4).
Status: DB9: D-DB1-7 partial. Core rs/llvm L chunk=4/T=65536 is now 1.025x (was 1.253x). The state is typed struct fields, kept in locals across the byte loop (D-DB9-1).
Status: DB9: D-DB1-8 closed. Core rs/rust chunk=65536 time is 0.962-0.987x. The trim check now runs only after a byte is written, never on bytes that produce no output.

## Changes

- `bench/l3/supervisor/btail.pruv`
  - Decoder state is now `Tail { acc: Acc { t, n, u }, d1: Dec, d2: Dec }` with `Dec { need, seen, cp, lo, hi }`, replacing the flat 13-slot `[i64]`.
  - `push_dec` keeps the counters and the decoder in locals for the whole range and writes them back once. A field write is a whole-struct store on LLVM (G-DB9-1).
  - Trimming:
    - The trim check runs after each write, as `u > 2t || n > 6t`. The buffer is 6t+4 bytes, so one more code point always fits.
    - `trim` takes `t, inout n, inout u` as scalars. Scalar inout works on LLVM since LG5.
  - Removed:
    - The `cp < 0` runtime check, because `cp` is built from `band(...)` and is proved `>= 0`.
    - `put_cp`.
- `bench/l3/supervisor/core_root.pruv`, `a1_root.pruv`, `supervise.pruv`
  - These use `crate::btail::Tail`, with `ts.acc.t` / `ts.acc.n` / `ts.acc.u` in place of `ts[1]` / `ts[0]` / `ts[12]`.
  - The `len(ts) == 13` contracts are gone.
- `bench/l3/supervisor/capfile.pruv`
  - `cap_take` writes `data` directly when the whole chunk fits. Before, it always did `truncate(copy(data), n)`, which is two 64 KiB copies per read.
  - The chunk bound is now 1 MiB. `take` in `supervise.pruv` has the same bound.
  - Reads stay at 64 KiB.
- `bench/l3/supervisor/measure_r10.py`
  - New `--rounds N` and `--lane all|core|whole` options.
- `bench/l3/pkgs/transport` is unchanged. A new function there cannot remove the copies, because they happen inside the `ev_` runtime (G-DB9-2).

Tried and reverted:
- An ASCII fast path in front of the struct decoder made things worse: rs/rust 1.10-1.14x and rs/llvm 1.11-1.21x. The test data is mostly non-ASCII.
- 1 MiB process reads made whole-lane RSS 1.18-1.19x, with no gain in time.

## Measurement

- Load average at the start was 3.11 / 4.39 / 11.46, and at the end 3.36 / 4.11 / 10.64. That is below 16, so 7 rounds were used.
- Command: `python3 bench/l3/supervisor/measure_r10.py --rounds 7`, with rounds alternating with hand Rust.
- Cells: `bench/l3/supervisor/work/r10/perf_cells.json` (40 cells).
- `tools/perf_gate.py` verdict: FAIL, 30 of 40 cells over 1.0x. Most of those are RSS cells at 1.001-1.012x.

| Row | Cell | Before time | After time | Before RSS | After RSS |
|---|---|---:|---:|---:|---:|
| D-R10-1 / D-DB1-7 | core rs/llvm (M, L) | 0.94-1.25 (DB1), 1.20-1.51 (R10) | 0.999-1.025 | 1.001-1.012 | 1.001-1.012 |
| D-R10-2 / D-DB1-8 | core rs/rust chunk=65536 | 1.011-1.024 | 0.962-0.987 | 1.003-1.011 | 1.001-1.011 |
| - | core rs/rust chunk=4 | 0.95-1.05 | 0.931-0.963 | 1.004-1.011 | 1.001-1.012 |
| D-R10-3 | whole rs/rust | 1.07-1.10 | 0.941 (M), 1.116 (L) | - | - |
| D-R10-3 | whole rs/llvm | 1.22-1.30 | 1.076 (M), 1.154 (L) | - | - |
| D-R10-4 | whole RSS | - | - | 1.018-1.067 | 1.015-1.053 |

## Debt

| Row | Cell | Cause |
|---|---|---|
| D-DB9-1 | core rs/llvm time 1.006-1.025x (M c65536/T65536 0.999x) | The hot loop already runs on locals. Not profiled further. The likely cause is the LLVM lowering of the remaining struct reads and writes (G-DB9-1) and `enc` calls that are not inlined. |
| D-DB9-2 | core RSS 1.001-1.012x, both pairs (+20-230 KB) | It is no longer the `ts` array. It grows with T, so the likely cause is the `tail_bytes` copy on output plus binary and runtime size. Not profiled. |
| D-DB9-3 | whole time rs/rust L 1.116x, rs/llvm M 1.076x / L 1.154x | Per-read cost in the `ev_` runtime (G-DB9-2). M rs/rust has 1196 voluntary context switches against 270 for hand, and 1.43 G instructions against 1.20 G (`/usr/bin/time -l`). |
| D-DB9-4 | whole RSS 1.015-1.053x | The `ev_` process reader reads ahead up to 1 MiB per stream before it blocks (generated `supervise_gen.rs:656`, `while q[i].bytes[0].len()>=1048576`). Hand uses a sync channel of 2 x 64 KiB. |

## Gaps

| Row | Gap | Workaround |
|---|---|---|
| G-DB9-1 | The LLVM backend lowers each struct field read as `load %Struct` + `extractvalue`, and each write as `load` + `insertvalue` + `store` of the whole struct, through allocas (`core_root_gen.ll`, `dec_slow` / `push_dec`). With per-field access in the byte loop, rs/llvm was 1.10-1.20x. | `push_dec` copies the fields into locals once and writes them back once. |
| G-DB9-2 | The `ev_` runtime has a fixed per-read cost, and there is no API to read into a caller buffer or to bound read-ahead. Each read does a new zeroed 64 KiB vec, an extend into the queue, a drain into `l.buf`, then an `ev_bytes` clone. There is also one condvar wake per read, and the read-ahead is fixed at 1 MiB per stream. | None in the lane. The two copies in `cap_take` were removed. |

## Tests

All tests were run on the final code, on each pair listed.

| Test | Result | Pairs |
|---|---|---|
| test_a0 | 522/522 | py/rust, py/llvm, rs/rust, rs/llvm |
| test_a1 | 410/410 | py/rust, py/llvm, rs/rust, rs/llvm |
| test_a2 | 78/78 | the pairs it runs |
| test_a3 | 35/35 | the pairs it runs |
| q.py | same 22, allowed 7, DIFF 0 | py/rust, py/llvm, rs/rust, rs/llvm |
| mutants.py | 12/12 rejected | - |

The codex tests were not run, because transport did not change.

## Delegation

| Work | Who |
|---|---|
| Decoder structs, proofs, `cap_take`, `measure_r10.py` (proof and encoding work) | main, Claude Opus 5.5, directly |
| Final perf measurement | main, directly |
| Bundle audit (a proof replaced the `cp < 0` runtime check) | Codex gpt-6.2-astra ultra, review, delegation 7e549b74 |

Audit result: no must-fix.
- Final tail bytes are the same as before. The middle value of `u` can differ, because trimming now happens after the write instead of before it. A suffix of T units always fits in 3T bytes, so the final suffix is kept either way.
- The completed code point `c` cannot be negative, even if a negative `d.cp` comes in from outside. It is built as `band(cp, 32767) * 64 + band(b, 63)`, so it stays in `0..2097151`.
- `cap_take` keeps its counters, its truncation flag and its error flags.
- All four reads are 64 KiB, so the running total cannot reach the `2^62 - 1048576` guard.
