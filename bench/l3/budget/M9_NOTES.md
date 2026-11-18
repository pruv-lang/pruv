# L3-10 M9: run clock (`bud_clock.pruv`)

Spec: `docs/REQUEST_BUDGET_SPEC.md` §5.1–§5.7, §8.1, §9, §15. Reference: `reference/backend-request-budget.ts` (B).

## What is implemented

- `bud_clock.pruv` implements, in Pruv on the M4 arena (`bud_store` / `bud_map`):
  - the run methods: `run_started`, `run_waiting`, `run_ended`;
  - the clock sub-operations: REFRESH, BEGIN, ARM, SETTLE, FIRE, DROP, CLOSE (the clock part of CLOSE only);
  - lookups: `clock_of`, `clock_owner`, `armed`.
- Clock state is explicit in the clock entry. Root 1 of the world holds the clocks, keyed by budgetId. The blob is the owning sessionId.
  - Field 14: since-present.
  - Field 15: since (ms).
  - Field 16: armed. A timer was requested and has not been cleared or delivered. This is the timer callback state.
  - Field 17: active-run count, the number of runs with `waiting = false`. Only the run methods (and DROP, which frees the runs with the clock) change runs, and they keep this count, so REFRESH reads "some run is not waiting" in O(1) instead of scanning. The Q final-state check compares it with the run list on every case that completes (cases that stop on a Grow or bad status are compared on their halt result instead).
  - Runs are child entries keyed by runKey; field 14 holds `waiting`.
  - Field access reads the entry base once (`fa`) and then uses `rf`/`wf` on it, instead of `bud_store::ef`, which re-reads the header on every access.
- The timer is host state. Pruv emits effect records, and the host scheduler owns the tokens:
  - `[12, requested, effective]` for timerSet, where effective = 1 when requested > 2^31−1 (Node's clamp);
  - `[13]` for timerClear (cancel the budget's timer);
  - `[14]` for timerForget, emitted only by FIRE when field 16 = 1: the host drops the budget→token mapping, but the token stays scheduled. This is B's `clock.timer = undefined` without `clearTimeout`, so an orphaned live timer still fires later.
  - FIRE takes the ref the delivered timer captured when it was armed (B's `setTimeout(() => this.fire(ref))`). The host keeps that ref per token and also delivers stale but captured callbacks. FIRE then behaves exactly as B: it clears the armed flag, checks CURRENT/open/unlimited, settles (a no-op for another owner), then re-arms or returns HALT_TIME. The host never judges liveness.
- Both compilers verify the module graph:
  - Python: `pruvc.py m9_test/root_m9.pruv -o … --backend rust --lib bench/l3/budget --lib bench/l3/lib` → `{"ok": true}`;
  - Rust: `pruvc-rs/target/release/pruvc`, which is the default in `run_m9.py`.
  - Both generated programs give identical Q results.
- Test pieces in `m9_test/`:
  - `root_m9.pruv`: the test root (`t_init`, `t_grow`, `t_put`, `t_call`, `t_observe`).
  - `driver_m9.rs`: the host scheduler and the Q/perf driver.
  - `run_m9.py`: `q`, `derive`, `perf`.
  - `hand_m9.rs`: the hand-written Rust baseline.
  - `perf_ts.mjs`: the TS perf side.

### Placeholder budget record (until M7)

The real budget record and CURRENT belong to M4/M7. `root_m9.pruv` keeps a placeholder session blob in root 0:
`[state, unlimited, timeMinutes, activeMs, activeSince kind, since, bidLen, bid…]`.

`load_bv` does the following:
- it finds the session and checks that the stored bid equals the ref's budgetId (this is CURRENT), reading the blob offset and length once;
- it decodes the blob into the budget view frame `bv`;
- `store_bv` writes it back after the call.

M7 replaces this pair with its record read/write. Nothing in `bud_clock.pruv` depends on the placeholder layout.

## Sub-operation interface (for M7 lifecycle and M10 halt/decide)

Shared frames, described in full in the header of `bud_clock.pruv`:

- `st`: the world arena (inout).
- Keys are `(array, start, length)` triples, already validated by the caller: `bk` budgetId, `sk` sessionId, `rk` runKey.
- `now`: one instant per public call (KD-1).
- `bv` (inout) is the budget view frame:
  - `[0]` current (1/0);
  - `[1]` state (0 open, 1 halted, 2 closed);
  - `[2]` unlimited;
  - `[3]` limits.timeMinutes;
  - `[4]` used.activeMs;
  - `[5]` activeSince kind (0 absent, 1 persisted and untouched, 2 = ISO(`bv[6]`) set by BEGIN);
  - `[6]` since in ms.
  - The caller formats the ISO text with `bud_date` when it writes the record.
- `fx` (inout) is the effect frame:
  - `fx[0]` counts the words used, and records follow;
  - one public sub-operation appends at most 4 words (FIRE: timerForget then timerSet);
  - it returns 3 with nothing written unless `fx[0] + 5 <= len(fx)` (`settle_at` and `drop_at` leave that test to the caller: bud_lifecycle and the key forms make it).
- `limit_ms` clamps timeMinutes to ±150000000000000 (result within ±9e18, exact in i64), so every delay `limit - activeMs` with safe inputs is the exact integer (P22, audit C1: the old ±150119987580 clamp gave 59009 where B gives 119009).
- A limit product or a settled activeMs above 2^53 leaves the safe integral domain (spec §2.3, §8.4): B rounds the double, the clock keeps the exact i64 delay and saturates activeMs (`sc`). Such cases are KD-M6-1 (excluded domain); `run_m9.py` classifies an oracle record holding a `{num}` value as known-difference. The clock does not yet refuse them with `ResourceBound(number)` (queued, Appendix P22.7).
- Every public entry taking `bv` tests `len(bv) >= 7` before any write and returns 0 with nothing changed.
- Status codes:
  - `0` done;
  - `4` done + HALT_TIME: the caller must run `halt(ref, 'time')` (M10), and SETTLE has already run;
  - `2` invalid (corrupt state);
  - `3` fx too small;
  - `−N` Grow(N): nothing was written; grow the buffer to N and retry.

| Name | Frame | Status | Use |
|---|---|---|---|
| `clock_of(st, bk,bs,bn) -> ce` | read | entry or −1 | locate once, then use the `_at` forms |
| `clock_owner(st, bk,bs,bn, inout dst) -> n` | read | length or −1 | observation only (FIRE uses the captured ref) |
| `armed(st, bk,bs,bn) -> 0/1` | read | | observation only |
| `refresh_at(inout st, ce, sk,ss,sn, now, inout bv, inout fx)` | | 0 / 3 / 4 | M10 decide-extend, M5 raise |
| `begin_at(inout st, ce, now, inout bv)` | | – | |
| `arm_at(inout st, ce, sk,ss,sn, now, inout bv, inout fx)` | | 0 / 3 / 4 | 4 = sync FIRE ended in halt |
| `settle_at(inout st, ce, sk,ss,sn, now, inout bv, inout fx) -> added ms` | | caller checks `fx_ok` | M10 halt, M7 close |
| `fire_at(inout st, ce, sk,ss,sn, now, inout bv, inout fx)` | | 0 / 3 / 4 | host timer delivery with the captured ref; `ce` may be −1 (no clock → HALT_TIME, as B) |
| `drop_at(inout st, ce, inout fx)` | | caller checks `fx_ok` | dropClock (M7 reopen/forget/dispose) |
| `refresh / settle / fire / drop / close(... bk,bs,bn ...)` | key forms | 0/2/3/4/−N | same, with the clock lookup and the fx check |
| `run_started / run_waiting / run_ended(...)` | key forms + `rk,rs,rn` | 0/2/3/4/−N | B:286/292/299 |

- `close` covers the clock part of §5.7 only: SETTLE, `bv[1] = 2`, DROP. M7 writes `ended` and saves.
- `run_started` reserves room for both entries (`make_room`) before its first write, so a Grow leaves the state untouched. It looks the run up once: a run that was not found is added without a second lookup (`make_room` keeps entry numbers).

## New behaviour not in §15 (classification)

| # | Behaviour | Class | Decision |
|---|---|---|---|
| M9-1 | A captured `setTimeout` callback delivered after its timer was cleared or already fired: B runs `fire(ref)` again | B behaviour | Modelled, not a KD: the host keeps the captured ref per token and delivers it; FIRE forgets (`[14]`) rather than clears, as B does. (An earlier draft had this as KD-M9-1; it is gone) |
| M9-2 | ARM with `left <= 0` calls FIRE synchronously in B. Right after BEGIN this always ends in SETTLE adding 0 and then `halt(ref,'time')` (activeMs ≥ limit) | intended (B:337) | `arm_at` does SETTLE and returns status 4 instead of recursing; the halt itself is M10 |
| M9-3 | FIRE re-arms on `runs.size > 0` (§15.1 #11). The spec decision says "use the consuming predicate" | equivalent except for a captured callback delivered after every run went waiting (then `runs.size > 0` but the consuming predicate is false, so B re-arms one more timer, which a later REFRESH or FIRE settles) | Kept B's `runs.size` (`bud_map::count`, O(1)) to match B, including that case. Q shows no difference against B |
| M9-4 | REFRESH reads a kept active-run count (field 17) instead of scanning the runs as B does | intended (same predicate, O(1)) | The count is maintained by every path that changes `waiting` or the run set; Q compares it with the runs in every completed case |
| M9-5 | runWaiting flips the run of a stale ref before REFRESH stops at CURRENT | B behaviour (§15.1 text) | kept |

No bugs found in B on the clock paths.

## Q results

The original 300 Q3 cases are `work/m9/cases.jsonl` against `oracle.jsonl`.
- Clock events compared on the prefix of every case:
  - S: set token + requested delay;
  - C: clear;
  - F: fire;
  - H: halt instant and halted sessionId, with the first of runningDelegationIds / cancelRecovery / stopMain as the marker.
- At a time-halt cut with no raise/charge cut before it, the halted session is also checked: the oracle shows state 'halted' with `halt.at` = ISO(the H instant), its activeMs equals Pruv's settled activeMs, and activeSince is absent on both sides.
- Full cases also compare the final state: state, activeMs, activeSince, clocks (owner, runs and waiting, since, timer token), and the active timers with their due times.
- Any `X deliver` line (a delivery the host could not route) is a fail.

| Set | exact | known-difference | not-implemented | fail | halt state checked |
|---|---:|---:|---:|---:|---:|
| Q3 original (300) | 26 | 0 | 274 (raise/charge call 176, time halt 98) | 0 | 39 |
| Q3 clock-only derived (300) | 136 | 0 | 164 (time halt) | 0 | 164 |

- The derived set comes from `run_m9.py derive`. It removes raise/charge calls from the same 300 cases (ids get a `-clock` suffix) and reruns the TS oracle.
- not-implemented cases are cut:
  - at the first raise/charge call, which needs M5/M6 charge and M7;
  - or at the time halt, which needs M10.
- The prefix before the cut must match. Nothing after the cut is counted or filled in by host code.
- Sensitivity check: mutating `60000` → `60001` in the generated code gives 153 fails.
- Both compilers give the same numbers.

Commands, run from `bench/l3/budget`:

```
python m9_test/run_m9.py q
python m9_test/run_m9.py derive
python m9_test/run_m9.py q --cases work/m9/cases_clock.jsonl --oracle work/m9/oracle_clock.jsonl
python m9_test/run_m9.py q --gen work/m9/gen_py.rs     # Python-compiler output
```

## Perf

How it was measured (`run_m9.py perf --rounds 5 --reps 200000`):
- Pruv, hand Rust and TS runs are interleaved round by round, and the table shows medians (the machine is heavily loaded).
- TS is the pinned reference class on node, with real timers.

Items:
- **refresh R**: R runs with R−1 waiting; toggle runWaiting on the last run. Each call runs REFRESH, then SETTLE or BEGIN+ARM.
- **cycle**: runStarted/runEnded on one run.
- **fire**: an early delivery that settles and re-arms.

Both Rust sides run the same algorithm and the same host design:
- an active-run count, so REFRESH is O(1) on both;
- the same lookups (session, clock, run) per call;
- the same scheduler: a timer list plus a budget → token map, with keys in the same bytes. The driver encodes the op's units into reused scratch buffers, and hand Rust uses its `String`s. Keys are copied only when a timer captures its ref or a budget gets its first map slot.

Bar (user decision 2026-11-17): parity with hand Rust. Anything above 1.1x must be fixed, or explained down to a compiler/lib cause outside M9's files.

| item | size | Pruv ns | hand Rust ns | TS ns | Pruv/hand |
|---|---|---|---|---|---|
| cycle | 1 | 210.4 | 162.6 | 2671.0 | 1.29 |
| fire | 1 | 269.6 | 205.9 | 4833.1 | 1.31 |
| refresh | 10 | 200.5 | 142.8 | 2715.4 | 1.40 |
| refresh | 50 | 203.4 | 141.5 | 2784.3 | 1.44 |
| refresh | 100 | 198.5 | 143.1 | 2879.7 | 1.39 |

**Every row is above 1.1x. The remaining cause is in `bud_map.pruv` (M4, outside M9's files) and in Pruv codegen.**

Pruv is 10–18x faster than TS on every item.

**Update (L3-10 perf pass 3, 2026-11-17): every row is now within 1.1x.** The two fixes the table below asks for
are done: `bud_map::hash_key` packs 4 units per mix, the compiler emits `#[inline]`, and `entry_matches` compares
keys with the new `eq_range` builtin (one bounds-proved slice `==`). `run_m9.py perf --rounds 5`, load average 5.7-6.0:

| item | size | Pruv ns | hand Rust ns | TS ns | Pruv/hand |
|---|---|---|---|---|---|
| cycle | 1 | 156.9 | 168.1 | 3025.4 | 0.93 |
| fire | 1 | 224.2 | 209.1 | 5505.9 | 1.07 |
| refresh | 10 | 155.3 | 147.5 | 3096.7 | 1.05 |
| refresh | 50 | 153.8 | 143.3 | 2897.3 | 1.07 |
| refresh | 100 | 152.9 | 142.9 | 3259.9 | 1.07 |

Q after the change: 26 / 0 / 274 / 0 (Q3) and 136 / 0 / 164 / 0 (clock cases). The tables below are the earlier state.

### What was fixed in M9's files (parity pass)

Before the pass the ratios were cycle 1.51, fire 1.48, and refresh 10/50/100 at 1.76/1.97/2.21.

| change | file | effect |
|---|---|---|
| Active-run count in field 17 replaces the REFRESH scan. The deadline field it replaced was observation only | `bud_clock.pruv` | refresh is flat in R: 251/326/443 → ~200 ns |
| Entry base read once per sub-operation (`fa`, `rf`, `wf`) instead of `ef` re-reading the header on each field | `bud_clock.pruv` | fewer header reads on every path |
| `run_started` does one run lookup (was find, then `make_room`, then find again) | `bud_clock.pruv` | cycle |
| `owner_is` and `load_bv` read the blob offset and length once, instead of `blob_len` / `blob_unit` per unit | `bud_clock.pruv`, `root_m9.pruv` | per call |
| Driver scheduler keys are byte keys like hand Rust's `String`, borrowed from scratch buffers. Before, they were `Vec<i64>` and hashed 8 bytes per unit, and freeing 288-byte timer keys hit a slower malloc path. The op for a delivery reuses one buffer | `driver_m9.rs` | DefaultHasher dropped from 18% to 3% of samples |
| Hand baseline given the same active count and the same `by_bid` update (`get_mut`, then insert) | `hand_m9.rs` | equal algorithm |

### Remaining gap: `bud_map::find` (M4) and per-unit loops (codegen)

Profile (`sample`, `-g` build, refresh 10 phase, about 3000 samples):
- `bud_map::find`: 37%;
- the `t_call` body (inlined `load_bv`, `store_bv`, `key_end`, the bid compare): 18%;
- `refresh_at` / `settle_at` / `arm_at`: 6%;
- `bud_store::valid`: 1%;
- the rest is host: malloc and free, the scheduler hash, and the encoding. Hand Rust shows the same malloc and free work.

Hand Rust's top is SipHash `write` plus `memcmp`.

Proof by experiment. The same driver and the same Pruv code were used, and only the generated `bud_map` functions were swapped in a copy of `gen.rs`. These files are in `work/m9/` and are not committed.

| variant | cycle | fire | refresh 10 | refresh 50 | refresh 100 |
|---|---|---|---|---|---|
| as generated | 1.29 | 1.31 | 1.40 | 1.44 | 1.39 |
| `hash_key`: 4 units per `mix` | 1.07 | 1.14 | 1.31 | 1.30 | 1.34 |
| + `entry_matches` as a slice `==` (memcmp) | 0.95 | 1.10 | 1.12 | 1.11 | 1.21 |

Line by line, for a lookup:

| step | hand Rust (`hand_m9.rs`) | generated Rust (`work/m9/gen.rs`) | why Pruv cannot emit the hand code |
|---|---|---|---|
| hash | `self.clocks.get(&r.budget_id)`: SipHash-1-3 over 36 UTF-8 bytes, 8 bytes per compression round | `bud_map_8_hash_key`: `while j < n { h = mix(h, rd(key, s0 + j)); j += 1 }`, where `mix` is `(h ^ v).wrapping_mul(1099511628211)`. That is one dependent multiply per u16 unit: 36 serial multiplies for the budget id and 56 per call | The hash function is part of M4's map format. `add` and `find` must agree, and entries store `h` (field 6), so M9 cannot change it. Fix in `bud_map.pruv`: pack 4 units per `mix` (−30 to −40 ns per call above) |
| key compare | `b.id == r.budget_id` → `memcmp` | `entry_matches`: `while j < n { if rd(st, ko + j) != rd(key, s0 + j) { return false } }` | `ent_u16::rd` is a checked read (it returns 0 out of range), so each unit carries two bound checks and LLVM does not turn the early-exit loop into `memcmp`. Pruv has no slice-equality primitive. Fix: a lib or compiler slice-compare that proves the bounds once (−15 to −25 ns) |
| probe | hashbrown SwissTable group probe | linear probe: `slot_pos` re-reads slot count and base from the header (`hw(st, 4)`, `hw(st, 3)`) on every probe step | M4 layout. With 100 runs the probe chains are longer, which is the residue of refresh 100 (1.21) in the last variant |
| CURRENT bid compare | `memcmp` | `load_bv` loop with `rd` per unit (M9's file) | same codegen cause as the key compare: Pruv has no way to express a bounds-proved block compare |

The residue in the last variant (fire 1.10, refresh 1.11–1.21) comes from:
- the CURRENT compare loop;
- the `bud_store::valid` header check at the `t_call` boundary;
- the `field_addr` checks;
- the linear probe.

All of these are either M4 layout or the same per-unit codegen cause.

## Complexity bound check (§9)

- Costs below are the expected normal path. Storage upkeep is separate: an add in `run_started` can trigger a `bud_map` rehash (amortized), DROP frees the runs (linear in what is freed), and a Grow status makes the host copy the store into a larger one before retrying (`t_grow`, linear in the store, rare).
- The run methods and REFRESH are `O(key lengths)`: three `bud_map::find` calls (O(key length) expected) plus constant field work. The active count makes REFRESH O(1), and measurement confirms it is flat: 200.5 / 203.4 / 198.5 ns at R = 10 / 50 / 100. This is below §9's O(R) bound, and B is O(R).
- BEGIN, ARM, SETTLE, DROP and CLOSE are `O(key lengths)`:
  - field writes are constant;
  - `owner_is` compares sessionId units (key length);
  - DROP, and CLOSE through `drop_at`, call `bud_map::remove`, which frees the clock and its R runs. That is storage reclamation, linear in what is freed.
- FIRE checks are constant: field 16 (forget), `bv`, and `count` (O(1), see M9-3), plus SETTLE/ARM. The halt work (`D`) is M10.
- No call copies the world or replays history. The cycle and fire items stay flat at about 210 and 270 ns over 200k calls.

## Not done in M9 (owned elsewhere)

- The budget record, CURRENT and the ISO activeSince text (M7). This is replaced by the placeholder `load_bv`/`store_bv`.
- `halt(ref,'time')` after status 4 (M10). Q counts these cases as not-implemented.
- raise/charge paths (M5/M6/M7). Q counts these cases as not-implemented.
- Wiring into `bud_api` / `host.rs` / `harness.rs`. Those files are outside M9's scope. The M7/M10 integration calls the interface above.
