# L3 DB1: perf debt bundle part 1 (claude D-R08-1..5, supervisor core D-R10-1..2)

Target: every listed cell at or below hand Rust 1.0x on time and RSS.
Result: no row is fully closed. D-R08-1..3 and D-R10-1 improved a lot, and their remaining cells are listed below. D-R08-4, D-R08-5 and D-R10-2 are still open. One new regression is D-DB1-5.

## Status

- Status: DB1: D-R08-1 partial. depth_limit window time 1.73-1.76x → 0.105-0.118x on all four pairs. RSS is 0.983-1.017x, and 1.017x is left on py/rust and rs/rust (D-DB1-1).
- Status: DB1: D-R08-2 partial. ring_wrap window time 1.49-1.65x → 1.39-1.54x and RSS 1.12-1.18x → 1.02-1.06x. turns200 window time 1.42-1.57x → 1.33-1.44x (D-DB1-2).
- Status: DB1: D-R08-3 partial. window_8MiB window time 1.56-1.65x → 1.37-1.56x and RSS 1.65x → 0.995-1.014x (D-DB1-3).
- Status: DB1: D-R08-4 open. block_arrays window time on llvm was 1.04x and is now 1.03-1.08x across two runs (D-DB1-4). This bundle also caused a new py/llvm RSS regression (D-DB1-5).
- Status: DB1: D-R08-5 open. long_text window RSS was 1.12x and is still 1.12x on the rerun. The first 7-round run gave 1.03-1.39x because hand RSS itself moved between 10.9 and 12.9 MiB. turns16/17 window RSS is 1.02-1.10x and turns200 head/tail is 1.03-1.08x (D-DB1-6).
- Status: DB1: D-R10-1 partial. Core rs/llvm time was 1.20-1.51x and is now 0.94-1.06x. The one exception is L chunk=4/T=65536 at 1.253x (D-DB1-7). RSS is 1.001-1.012x.
- Status: DB1: D-R10-2 open. No change landed, because the try made rs/rust worse. Core rs/rust chunk=65536 time is 1.011-1.024x (was 1.023-1.069x) and RSS is 1.003-1.011x (D-DB1-8).

## Measurement

- Measured once at the end with the existing scripts. Each run had 7 rounds alternating with hand Rust, on the same cells as R08/R10.
  - claude: `python3 bench/l3/claude/q_measure.py --out bench/l3/claude/work/db1/perf_cells.json` (288 cells).
    - block_arrays and long_text were measured again with `--only`, to tell a real regression from noise.
  - supervisor: `python3 bench/l3/supervisor/measure_r10.py`, writing `bench/l3/supervisor/work/r10/perf_cells.json` (40 cells).
- The `tools/perf_gate.py` verdict is FAIL on both files, as the rows above predict.

| Row | Cell | Before time | After time | Before RSS | After RSS |
|---|---|---:|---:|---:|---:|
| D-R08-1 | depth_limit window | 1.73-1.76 | 0.105-0.118 | 1.00-1.05 | 0.983-1.017 |
| D-R08-2 | ring_wrap window | 1.49-1.65 | 1.39-1.54 | 1.12-1.18 | 1.02-1.06 |
| D-R08-2 | turns200 window | 1.42-1.57 | 1.33-1.44 | ~1.0 | 0.977-1.039 |
| D-R08-3 | window_8MiB window | 1.56-1.65 | 1.37-1.56 | 1.65 | 0.995-1.014 |
| D-R08-4 | block_arrays window llvm | 1.04 | 1.03-1.08 | 0.98-0.99 | rs/llvm 0.98, py/llvm 1.31-1.37 |
| D-R08-5 | long_text window | 0.27-0.51 | 0.27-0.50 | 1.12 | 1.12 |
| D-R08-5 | turns16/17 window | — | 1.14-1.44 | 1.00-1.09 | 1.02-1.10 |
| D-R10-1 | core rs/llvm (M, L) | 1.20-1.51 | 0.94-1.06; L c4/T65536 1.253 | 1.000-1.011 | 1.001-1.012 |
| D-R10-2 | core rs/rust chunk=65536 | 1.023-1.069 | 1.011-1.024 | 1.000-1.011 | 1.003-1.011 |

## What landed

- `bench/l3/supervisor/btail.pruv` (D-R10-1): `dec_byte` now writes UTF-8 bytes directly instead of calling `put_cp` four times. Decoder state stays in `ts`.
- `bench/l3/claude/entry.pruv` (D-R08-1): `window_line` projects a terminal chain of singleton arrays around one verified number in the last root member to `[]`.
  - Only the array kind is kept, so key shadowing still works.
  - Its helpers are `terminal_array` and `scan_shortened`. `user_text` and `title_for_type` split out lookups that are already done.
- `bench/l3/claude/turns.pruv` (D-R08-2/3): `reclaim` rebases the arena past evicted spans once they take half the capacity. It runs after a wrap only.
- `bench/l3/claude/import.pruv` (D-R08-2/3):
  - `read_window` looks up `type` once and computes the title candidate in the loop.
  - `Effect::Done` now carries a model token id instead of copied title and model strings, so a repeated identical model is not decoded again.
- `bench/l3/claude/test_a4.py`: a fixture for the projection (shadowed key, accepted key, malformed `[[01]]`).
- `lib/parse.pruv` is unchanged, so its consumer tests were not run.

## Debt rows

| Id | Cell | Line-level cause (hand vs generated) |
|---|---|---|
| D-DB1-1 | depth_limit window RSS 1.017x (py/rust, rs/rust) | Hand parses the borrowed input slice (handrust/src/lib.rs:498). Generated Rust allocates the shortened copy `owned` plus its tape (entry.pruv:68 `scan_shortened`). |
| D-DB1-2 | ring_wrap window 1.39-1.54x, turns200 window 1.33-1.44x | Hand reuses an evicted `Draft` at once (handrust/src/lib.rs:630). Generated keeps evicted bytes until they reach half the arena (turns.pruv:68 `reclaim`), and copies the arena on growth (turns.pruv:114 `reserve`). |
| D-DB1-3 | window_8MiB window 1.37-1.56x | Generated needs the collection arena and a separate parse tape per line (boundary.pruv:182, turns.pruv:114). Hand builds turns while it parses (handrust/src/lib.rs:498,945). |
| D-DB1-4 | block_arrays window llvm 1.03-1.08x | Hand checks with a recursive parser in one pass (handrust/src/lib.rs:472). Generated builds the whole tape, then checks it (lib/parse.pruv:123, lib/json.pruv:168). The fix needs lib/json.pruv, which is outside this lane. A try to cut source copies regressed and was reverted. |
| D-DB1-5 | block_arrays window py/llvm RSS 1.31-1.37x (R08: 0.98x), new in DB1 | Reproduced in two runs. The other three pairs are 0.98-0.99x, so only the py compiler's LLVM output is affected. The cause was not isolated within the cap. Suspect: the per-entry `Title` and `Effect` values now built in `read_window` (import.pruv:521-552) are not released on the py/llvm path. |
| D-DB1-6 | long_text window RSS 1.12x, turns16/17 window 1.02-1.10x, turns200 head/tail 1.03-1.08x | Generated keeps the fixed ring state (`sl[1800]` plus `st`) and arena headroom (turns.pruv:43,114; import.pruv:176,245,498). Hand uses a `VecDeque` sized to the input (handrust/src/lib.rs:626). A try at the head/tail copies was reverted. |
| D-DB1-7 | supervisor core rs/llvm L chunk=4/T=65536 1.253x | The byte loop in `dec_byte` still loads and stores decoder state through the `ts` heap array, and runs the trim check per byte (btail.pruv). Hand keeps the decoder in struct fields (hand/core_hand.rs:59). |
| D-DB1-8 | supervisor core rs/rust chunk=65536 1.011-1.024x (D-R10-2) | Same `ts` heap state as D-DB1-7. A try moved the state into locals, but it evaluated the trim condition even on bytes with no output (core_root_gen.rs:231). rs/rust got worse (1.06-1.20x), so it was reverted. |

## Gap rows

| Id | Gap | Workaround |
|---|---|---|
| G-DB1-1 | clang -O2 does not inline `put_cp` into `dec_byte` on the LLVM backend. | The UTF-8 writes are inlined by hand (btail.pruv `dec_byte`). |
| G-DB1-2 | The LLVM backend has no scalar `inout`. The D-R10-2 try had to pack two trim results into one int. | The try was reverted. |
| G-DB1-3 | `copy_range` rejects overlapping ranges (same as G-R10-1). | A forward copy loop (btail.pruv:87). |
| G-DB1-4 | The LLVM backend limits on `inout` structs and `take [u8]` block a struct interface for the claude ring. | The existing `[i64]`/`[u8]` array interface is kept. |
| G-DB1-5 | Worktree tests fall back to the main checkout's `pruvc-rs` binary when the worktree has no build (bench/l3/json/run_a2.py:113-115). That binary is stale and lacks `utf8_valid`, so rs pairs fail with `unknown_fn`. | Build `pruvc-rs` in the worktree (`cargo build --release --offline`). |

## Tests (worktree, rebuilt pruvc-rs, all four pairs)

- claude:
  - test_a0 272, test_a1 2,884, test_a2 2,443, test_a3 134 and test_a4 270: all pass.
  - q: 20,270/20,270 agree with the oracle.
  - q_mutants: 8 detected and 10 proof-rejected, 0 missed.
- supervisor:
  - test_a0 522, test_a1 410, test_a2 78 and test_a3 35: all pass.
  - q: 22 same, 7 allowed, 0 DIFF.

## Delegation

| Work | Who |
|---|---|
| Supervisor D-R10-1 + D-R10-2 try (write, reverted) | Codex gpt-6.2-sol, delegation f0731f11 |
| Supervisor D-R10-1 only (write, landed) | Codex gpt-6.2-sol, delegation 25d1a35d |
| Claude D-R08-1..5 (write, landed) | Codex gpt-6.2-sol, delegation 51a333b8 |
| Fragment tests, final 7-round measurement, D-DB1-5 recheck, this doc | main (Claude Opus 5.5) |
| Bundle audit (Codex gpt-6.2-astra ultra review) | not run: 2-hour cap reached |
