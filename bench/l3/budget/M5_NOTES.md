# L3-10 M5 notes: Rust host mechanics

Spec: `docs/REQUEST_BUDGET_SPEC.md` §14 row M5. Ground truth: `oracle-world.mjs`, `reference.mjs`.
M5 is an apparatus check. It does not count as semantic implementation (§14).

## Files

| File | Contents |
|---|---|
| `leaf.rs` | Order- and number-text-preserving JSON reader/writer `J`, JS value `V`, JV decode/encode (u16/undef/num/arr/obj/sess markers, `{sref}`/`{bref}` labels), `Heap` of identity objects with budget-only write-through, `set_budget` and `host_view` |
| `host.rs` | `World`: clock, UUID stream, fake timers (`(due, seq)` order, delay clamp to 1..2147483647), session store (put replaces in place or appends, remove), dep reply queues and scripted throws, trace, observation; program consumer; `run_q`; `Core` trait and `ResidentState<C>` lane (Grow to `max(N, 2·len)`, at most 3 retries, out-too-small doubling) |
| `harness.rs` | `Core` over the generated M4 entries (`m4_init`/`m4_grow`/`m4_op`), `harness q --prog <programs.jsonl>` (scenarios on stdin, observations on stdout), `harness perf <n> <reps>` |
| `host/` | crate `budget_host` (`include!` of `host.rs`, `leaf.rs`), `--offline --locked` |
| `dev_build.sh` | pruvc `m34_test/root_m4.pruv` -> `work/m5build/sol.rs`, cargo build of the host crate, rustc of the harness -> `work/m5build/harness`. `PRUVC` overrides the compiler (default: this checkout's `pruvc-rs/target/release/pruvc`) |
| `m5_test/gen_prog.py` | Oracle observations -> effect programs |
| `m5_test/run_m5.py` | Q cases -> oracle -> programs -> `q.py --impl` compare -> host and TS perf |
| `m5_test/perf_ts.mjs` | TS side of the perf: the same mechanics on the oracle world, tracing off |

The host makes no budget decision. It owns only I/O, storage, time, IDs, timers and buffer allocation on Grow.

## Effect programs

The semantic core (M7-M12) does not exist yet, so the core's side of each case is a program generated from the oracle
trace. One line per case: `{"id", "segs", "final"}`.

- `segs` in trace order: `{"kind": "call", "r", "items", "terminal", "value"}` for each script call,
  `{"kind": "fire", "key", "items"}` for each timer callback, `{"kind": "snap", "clocks", "toolSeen"}` for each snapshot.
- Items (what the core asks for, in order): `now`, `unit`, `uuid`, `find {id}`, `dep {c, a}` (void effects),
  `q {c, a}` (queries), `tset {key, delay, unref}`, `tclear {key}`, `tthrow {o}`, `bset {s, b?}`.
- `final`: `clocks`, `toolSeen` (per instance) and `bsets`.

The program carries only core decisions. The host recomputes everything else and the observation is compared whole by
`q.py`:
- `call.start`/`call.end` and script-event records
- `now`/`unit`/`uuid` values, findSession results, query replies
- timer tokens, dues and firing order (`key` is an opaque name the host maps to its own token)
- `timer.unref`/`timer.fire` records, scripted dep throws
- final sessions and active timers

Two conventions model TS's in-place mutation of shared session objects:
1. **Write-through (budget only).** For every `{sref: "sK", v}` in program JV (dep arguments, return values) whose
   label the host already knows, the host takes only the `budget` pair of `v` (absent = key deleted). Every other
   field stays host-owned. An unknown `sK` is created, unstored, from `v`. Dep/query trace arguments and call return
   values are re-encoded from the host objects, not echoed from the program. Label counters rise to the highest
   `sK`/`bK` seen.
2. **`bset`.** TS writes `session.budget` in place with no trace record. The budget attachment is core state, so the
   program carries `{"k": "bset", "s": "sK", "b": <budget JV>}` (no `b` = key deleted). It does so before each host read of
   a session (call.start arguments, findSession result, snapshot, final sessions) whose attachment differs from what
   the host last saw. Everything else in the session (id, other keys, key order, store position, presence) stays
   host-computed. 262 in-segment and 67 final `bset` items over the 300 cases.

Final and snapshot clocks/toolSeen are carried verbatim: they are core state (M9 and later).

## Commands and results

Run from `bench/l3/budget`:

```
cargo build --release --locked --manifest-path ../../../pruvc-rs/Cargo.toml --target-dir work/m5pruvc   # this checkout's compiler
PRUVC=$PWD/work/m5pruvc/release/pruvc sh dev_build.sh                                                   # -> built
python3 m5_test/run_m5.py                      # cases, oracle, programs, compare, perf (add --build to run dev_build.sh)
```

Q compare, seed 20261117, 300 cases (Q1-Q8 37-38 each):

```
implementation compare: exact=300 P6=0 excluded-domain=0 unexplained=0  (300 compared)
driver violations: 0   determinism: identical
```

Controls:
- Without `bset` (write-through only): exact=140, unexplained=160. All 160 differences were inside a session's `budget`
  (138 in final sessions, 20 in findSession results, 2 in call.start arguments).
- An extra host field `auditHostField: "KEEP"` in an input session of `Q6-20261117-1` (program generated without it):
  the host observation equals the oracle observation (the field survives in finds, dep arguments and final sessions).
- Every `tset` delay +1 (`work/m5/prog_bad.jsonl`): exact=260, unexplained=40. These are the 40 cases that arm timers,
  so the host computes dues and firing order itself.

## Perf

Per-op ns, min of 5 reps after warm-up. Apple silicon (this Mac). Host: `harness perf n 5` per n. Both sides time only the measured operation (setup outside). TS: `node m5_test/perf_ts.mjs 100,500,1000 5`.

| Measure | Host n=100 | n=500 | n=1000 | TS n=100 | n=500 | n=1000 | Host n=1000 / n=100 |
|---|---:|---:|---:|---:|---:|---:|---:|
| timer_cycle (set + clear) | 128 | 120 | 125 | 353 | 125 | 118 | 0.98 |
| timer_fire (n timers registered in setup, one timed advance) | 95 | 89 | 94 | 471 | 528 | 1094 | 0.98 |
| store_find (n sessions, n finds) | 730 | 1768 | 3159 | 632 | 1589 | 3213 | 4.3 |
| store_save (n sref saves, each setting a new budget) | 942 | 948 | 1007 | n/a | n/a | n/a | 1.07 |
| lane: M4 session_put via ResidentState | 90 | 86 | 85 | n/a | n/a | n/a | 0.95 |

Lane counters: grows 5 / 7 / 8 and final state length 8192 / 32768 / 65536 words at n = 100 / 500 / 1000. That is
logarithmic in n, as `max(N, 2·len)` implies.

- Timers, save and the lane are flat (n=1000 ≤ 1.5× n=100), and the timers are not slower than TS.
- `store_find` is not flat. Both sides scan the session list linearly, because oracle `findSession` is
  `sessions.find` (first match in store order). The host also encodes each result, which TS does not.
- At n=1000 the host is not slower than TS (3159 vs 3213). At n=100 it is (730 vs 632): the per-find result encoding.
- TS has no equivalent of store_save (a TS save is a dep record on a shared object) or of the lane.

## Unimplemented dependent paths

The host does not implement any of these. A program or a carried value stands in for each one.

| Path | Stand-in in M5 | Owner |
|---|---|---|
| All semantic ops: lifecycle, charge/tools, clocks, halt/decide, cards, auto/cleanup | generated effect program (`segs`) | M7-M12 |
| Dispatch of each call through a Pruv core with the M3 ABI packet (`Call/Facts/Outcome/Need/Resume`) | program items consumed in order | M3 ABI + M13 generated root |
| Budget attachment of sessions as core state (`save(sessionId, record words)` instead of shared objects) | write-through and `bset` | M7/M8 + M13 |
| Instance clocks and toolSeen (final, snapshot) | carried verbatim | M8/M9 |
| Return values and terminals | carried verbatim (`terminal`, `value`) | M7-M12 |
| `unit` / `findSession` answered through the core's facts (KD-1) | host world answers directly | M13 |
| ResidentState lane carrying real budget state | exercised only on M4 `session_put` in perf | M13 |

## Audit

One audit, three required findings, all fixed:
1. Write-through replaced whole host sessions from the program snapshot (host fields, ids and key order were not
   host-computed). Now budget-only, with traces from host objects; dep arguments are encoded only when tracing.
2. `run_m5.py` passed the size list to `harness perf`, which takes one n. Now one call per n.
3. TS `timer_fire` timed registration plus advance, the host only the advance. TS now registers in setup.

One confirmation review: the three fixes hold. One new required finding: `store_save` saved values without a
`budget`, so after the fix it measured no write. It now saves a new budget per session and checks, outside the timed
region, that all n budgets were written. Its numbers are not comparable with the earlier 472 ns figure, which
replaced whole sessions that had no budget.

## Remaining problems

- `store_find` is linear in the number of stored sessions, like the oracle world. An id index would make it flat,
  but it must keep first-match-in-store-order semantics. Not done in M5 (build budget).
- The main checkout's `pruvc` rejects `inout` parameters on host entries (`entry_signature` on `m4_init`).
  `dev_build.sh` therefore defaults to this checkout's `pruvc-rs`, which must be built first.
- `store_save` costs about 950 ns per save: `set_budget` wraps, strips and decodes the budget value through
  intermediate copies. It is flat in n. Decoding the budget directly would cut it; not done (build budget).
- Build budget was 3; the compiler build plus rebuilds after the audit used 6 build commands.
- Write-through and `bset` belong to the test program, not to the final boundary. The final shape is the core-owned
  attachment noted in the table above.

## M5 perf follow-up

Two host-only changes. No budget decision moved into the host, no compiler or `.pruv` change.

1. **Id index for the session store** (`host.rs`). `World.idx` maps a string id to the position in `stored` of the
   first object slot with that id. `init_world` builds it (first wins, so duplicate initial ids keep first-match
   order). `put_session` replaces in place, which keeps the position, or appends and indexes the new position.
   `removeSession` rebuilds the index after its `retain` only when something was removed (removal was already
   linear). `find_h` uses the index for
   string queries. Other query types keep the linear `strict_eq` scan: a stored string id never strict-equals them.
   Heap object ids never change after allocation, because write-through and `bset` write only `budget`. The index
   therefore returns the same handle as the old scan.
2. **Direct budget decode in `set_budget`** (`leaf.rs`). A `{"bref": L, "v": W}` value attaches label `L` and decodes
   `W`. Any other value decodes as is. The old path wrapped the value into a one-pair object, ran `strip_budget` and
   decoded that, which gives the same result: the key `budget` is not index-like, so `build_obj` keeps it, and decode
   errors are the same.

Correctness after the change (seed 20261117, 300 cases): exact=300, unexplained=0, driver violations 0,
determinism identical. The negative control (every `tset` delay +1) still gives exact=260, unexplained=40.

Perf: ns per op, Apple silicon, high machine load. Base (b59fd23), new host and TS ran alternately for 5 rounds,
each run min of 5 reps. The table shows the median over the rounds.

| Measure | Base n=100 | n=500 | n=1000 | New n=100 | n=500 | n=1000 | TS n=100 | n=500 | n=1000 | New n=1000 / n=100 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| store_find | 738 | 1892 | 3248 | 423 | 450 | 453 | 630 | 1643 | 3337 | 1.07 |
| store_save | 1022 | 1028 | 1042 | 324 | 330 | 325 | n/a | n/a | n/a | 1.00 |
| timer_cycle | 128 | 121 | 121 | 124 | 129 | 130 | 365 | 118 | 123 | 1.05 |
| timer_fire | 95 | 99 | 96 | 86 | 96 | 99 | 492 | 542 | 1069 | 1.15 |
| lane | 94 | 86 | 86 | 92 | 87 | 86 | n/a | n/a | n/a | 0.93 |

- `store_find` is flat and faster than TS at every n. The rest of its cost is the encoding of each result. TS
  returns an object reference and encodes nothing.
- `store_save` is 3.1x faster and still flat.
- Timer and lane changes are within run-to-run noise. Their code is unchanged.
- After the audit fix, a second 3-round alternation gave the same picture: find 409 / 435 / 442 (TS 635 / 1559 / 3231),
  save 307 / 316 / 319.

Audit (one, Codex gpt-6-astra ultra): no required finding. One advisory, applied: a `removeSession` that removed
nothing rebuilt the index. It now rebuilds only when the store shrank.
Builds: 3 (base, change, audit fix). The compiler was not rebuilt.

## Perf headroom against hand-written Rust

See `perf_headroom/HOST.md` for the hand Rust baselines, the store_find breakdown and the method. Host-side
allocation cuts brought the numbers to the following (medians of 5 alternating rounds, ns per op, n = 100/500/1000):

- store_save: 175 / 172 / 171, 1.30–1.35× hand Rust. Before: 307–330.
- store_find_lookup (a new harness measure of the string-id lookup helper only): 13.8 / 15.0 / 16.2, 1.18–1.29× hand
  Rust with UTF-16 keys. Before: 33 at n=1000.
- store_find with the JV encode: 365 / 434 / 394.
  - Against a hand frame built as an owned tree, it is 1.27–1.48×, and 1.11–1.21× for the encode alone.
  - Against hand code that writes the frame as text, it is 5.1–5.8×.
  - This is won't-fix: the encode is trace-only apparatus.

Correctness after the change is still exact=300, unexplained=0, determinism identical.
