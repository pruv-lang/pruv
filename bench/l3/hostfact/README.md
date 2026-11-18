# P10H host facts (budget + req)

Concrete synchronous G6 host-fact signatures and adapters. Decision logic lives in Pruv (`root.pruv`); the host
(`host.rs`, crate `hostfact_host`) only answers facts or performs side effects, and appends each call to a trace.

Run: `python bench/l3/hostfact/run.py [--rounds N] [--only gates|perf]` (work files in `work/`, gitignored).

## What is measured
- Gates (`harness gates`, per py/rs x rust/llvm build): halt order (query sees the settled value 8, effects in order
  incl. duplicate ids), a query-before-settle mutant that the trace check catches, invalid-answer and small-`out`
  refusals (no write to `out` or the halted flag), close/release answers, re-entry refusal (code 9) with the guard
  cleared on unwind, req first match / invalid answer / no match with exact call sequences, and Pruv == hand lanes.
- Perf (`harness perf R`, R >= 7, median ns): `rq_find` 20000 x 10 no-match (420000 host calls) Pruv vs hand Rust vs a
  non-call control (same loops, host calls replaced by inline arithmetic); `bh_halt` sync path x 200000 (3 ids).
  Trace is off in perf, so a host fact is a plain call. Ratio = Pruv / hand.

## Host signatures
| fact | signature | answer |
|---|---|---|
| `running_ids` | `(session, settled: i64) -> Vec<i64> ! alloc, io` | `[status, n, ids...]` |
| `is_quiet` | `(session) -> i64 ! io` | 0 / 1 |
| `release_held` | `(session) -> i64 ! io` | 0 / 1 |
| `next_item` | `(s, m) -> i64 ! io` | fresh item id (`s*16+m+1` in the test host) |
| `is_temporary` | `(item) -> i64 ! io` | 0 / 1 |
| `title_obs` | `(s) -> i64 ! io` | one per session |

## Pruv entries and refusal semantics
| entry | returns |
|---|---|
| `bh_halt(inout st, session, inout out)` | 0 done; 2 invalid `running_ids` answer (len < 2, status != 0, len != n+2, id <= 0); 3 `out` smaller than `2*(n+2)` words |
| `bh_close(st, session)` | 10 + answer; 2 if the answer is not 0/1 |
| `bh_release(session)` | 0 (true), 1 (false: caller queues continue), 2 invalid answer |
| `rq_find(s_n, m_n)` | first temporary item id; -1 none; -2 invalid answer |

`bh_halt`: SETTLE (`st[0] += st[1]; st[1] = 0`) comes first and is not undone by a refusal; refusals write nothing to
`out` and leave `st[2]` (halted) at 0. Output `out = [count, 1, 2, 3, (4, id)...]`: halted, cancelRecovery,
stopMain, then stopDelegation per running id in order, duplicates included. The harness adapter refuses re-entry
(code 9) while an entry is active.

## Status
The full request engine is not built. Grid result (P10H, docs/L3_RESTRUCTURE_SPEC.md appendix P10H): `rq_find`
passes on the Rust backend (Pruv / hand 0.90x to 1.07x at the median once trusted shims are inlined and carry no unused
ctx guard); the LLVM backend does not (1.22x to 2.0x, the per-host-call C boundary, P17). req is unblocked on the Rust
backend only. `bh_halt` passes on the Rust backend (0.94x to 0.98x once the non-I/O G6 builtins such as `vec_get` are
inlined and carry no unused ctx guard) and is 2.1x to 2.5x on the LLVM backend (11 boundary crossings per halt, P17).
