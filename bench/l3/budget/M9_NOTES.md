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
  - Field 17: deadline (now + effective delay). It is kept for observation only.
  - Runs are child entries keyed by runKey; field 14 holds `waiting`.
- The timer is host state. Pruv emits effect records, and the host scheduler owns the tokens:
  - `[12, requested, effective]` for timerSet, where effective = 1 when requested > 2^31−1 (Node's clamp);
  - `[13]` for timerClear.
  - FIRE decides on its own whether a delivery is live: with field 16 ≠ 1 it is a no-op. The host never judges.
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
- it finds the session and checks that the stored bid equals the ref's budgetId (this is CURRENT);
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
  - one public sub-operation appends at most 3 words;
  - it returns 3 with nothing written unless `fx[0] + 4 <= len(fx)`.
- Status codes:
  - `0` done;
  - `4` done + HALT_TIME: the caller must run `halt(ref, 'time')` (M10), and SETTLE has already run;
  - `2` invalid (corrupt state);
  - `3` fx too small;
  - `−N` Grow(N): nothing was written; grow the buffer to N and retry.

| Name | Frame | Status | Use |
|---|---|---|---|
| `clock_of(st, bk,bs,bn) -> ce` | read | entry or −1 | locate once, then use the `_at` forms |
| `clock_owner(st, bk,bs,bn, inout dst) -> n` | read | length or −1 | FIRE ref = (owner sessionId, budgetId) |
| `armed(st, bk,bs,bn) -> 0/1` | read | | M10 / observation |
| `refresh_at(inout st, ce, sk,ss,sn, now, inout bv, inout fx)` | | 0 / 4 | M10 decide-extend, M5 raise |
| `begin_at(inout st, ce, now, inout bv)` | | – | |
| `arm_at(inout st, ce, sk,ss,sn, now, inout bv, inout fx)` | | 0 / 4 | 4 = sync FIRE ended in halt |
| `settle_at(inout st, ce, sk,ss,sn, now, inout bv, inout fx) -> added ms` | | | M10 halt, M7 close |
| `fire_at(inout st, ce, sk,ss,sn, now, inout bv, inout fx)` | | 0 / 4 | host timer delivery |
| `drop_at(inout st, ce, inout fx)` | | – | dropClock (M7 reopen/forget/dispose) |
| `refresh / settle / fire / drop / close(... bk,bs,bn ...)` | key forms | 0/2/3/4/−N | same, with the clock lookup and the fx check |
| `run_started / run_waiting / run_ended(...)` | key forms + `rk,rs,rn` | 0/2/3/4/−N | B:286/292/299 |

- `close` covers the clock part of §5.7 only: SETTLE, `bv[1] = 2`, DROP. M7 writes `ended` and saves.
- `run_started` reserves room for both entries (`make_room`) before its first write, so a Grow leaves the state untouched.

## New behaviour not in §15 (classification)

| # | Behaviour | Class | Decision |
|---|---|---|---|
| KD-M9-1 | A captured `setTimeout` callback delivered after its timer was cleared or already fired: B runs `fire(ref)` again (clearing whatever timer is current, then settle/re-arm or halt); the Pruv host has no stale-callback object, and FIRE with field 16 = 0 is a no-op | accidental in B (relies on clearTimeout never racing) | KD. Q classifies a dropped token whose next oracle event is that `F tok` as KD-M9-1; the prefix before it must match. 7 cases in the original set, 10 in the derived set |
| M9-2 | ARM with `left <= 0` calls FIRE synchronously in B. Right after BEGIN this always ends in SETTLE adding 0 and then `halt(ref,'time')` (activeMs ≥ limit) | intended (B:337) | `arm_at` does SETTLE and returns status 4 instead of recursing; the halt itself is M10 |
| M9-3 | FIRE re-arms on `runs.size > 0` (§15.1 #11). The spec decision says "use the consuming predicate" | equivalent in reachable states (#11) | Kept B's `runs.size` (`bud_map::count`, O(1)). The consuming predicate is an O(R) scan and would break §9's "FIRE: constant clock checks otherwise". Q shows no difference |
| M9-4 | Deadline (field 17) stored with the clock | intended (host/observation aid) | Not read by any judgment |
| M9-5 | runWaiting flips the run of a stale ref before REFRESH stops at CURRENT | B behaviour (§15.1 text) | kept |

No bugs found in B on the clock paths.

## Q results

The original 300 Q3 cases are `work/m9/cases.jsonl` against `oracle.jsonl`.
- Clock events compared on the prefix of every case:
  - S: set token + requested delay;
  - C: clear;
  - F: fire;
  - H: halt, with the first of runningDelegationIds / cancelRecovery / stopMain as the marker.
- Full cases also compare the final state: state, activeMs, activeSince, clocks (owner, runs and waiting, since, timer token), and the active timers with their due times.

| Set | exact | known-difference | not-implemented | fail | compared events |
|---|---:|---:|---:|---:|---|
| Q3 original (300) | 25 | 7 (KD-M9-1) | 268 (raise/charge call 174, time halt 94) | 0 | S 162, H 94, F 88, C 4 |
| Q3 clock-only derived (300) | 132 | 10 (KD-M9-1) | 158 (time halt) | 0 | S 260, H 158, F 166, C 8 |

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
- **refresh R**: R runs with R−1 waiting; toggle runWaiting on the last run. Each call runs REFRESH with a full scan, then SETTLE or BEGIN+ARM.
- **cycle**: runStarted/runEnded on one run.
- **fire**: an early delivery that settles and re-arms.

Every item includes the host scheduler work on both Rust sides.

| item | size | Pruv ns | hand Rust ns | TS ns | Pruv/hand |
|---|---|---|---|---|---|
| cycle | 1 | 225.1 | 159.4 | 2564.6 | 1.41 |
| fire | 1 | 468.0 | 233.8 | 4724.5 | 2.00 |
| refresh | 10 | 239.6 | 144.0 | 2768.7 | 1.66 |
| refresh | 50 | 321.7 | 163.3 | 2701.2 | 1.97 |
| refresh | 100 | 429.3 | 187.4 | 2890.3 | 2.29 |

Pruv is 6–12x faster than TS everywhere. Against hand Rust it is above 1.5x except for cycle.

- Fixes made after the first profile:
  - `any_active` reads the entry base from the header once instead of on every field access;
  - the driver scheduler keys are built lazily as `Vec<u16>`.
  - Together they cut 15–19%: refresh 100 went from 479.8 to 429.3, fire from 566.5 to 468.0.
- Profile of the remaining gap (`sample` on a `-g` build, R=100 refresh phase, about 2300 samples):
  - `refresh_at`, i.e. the run scan, 31%: about 2.1 ns per run in Pruv against 0.5 ns per run in hand Rust. Each run costs two checked arena reads (`ent_u16::rd`: index bound plus value) and follows a linked list of entries. Hand Rust iterates a dense hashbrown value array.
  - `bud_map::find`, 29%: three keyed lookups per call (session, clock, run). Each one hashes the UTF-16 key word by word and compares it against the entry's key in the arena. Hand Rust's SipHash over a short `String` costs about the same per lookup, but its compare is a `memcmp`.
  - `load_bv` / `store_bv`, about 11%: the placeholder session blob is decoded and written back. M7 owns the real record.
  - The driver's `HashMap<Vec<u16>>` and malloc, about 15%. The hand side does equivalent `String` work.
- Explanation, not fixed in M9:
  - The two largest parts are the M4 arena design (checked u16-indexed reads, a linked child list) and the M4 `find`. `bud_store.pruv` and `bud_map.pruv` are out of M9's file scope.
  - The fixed cost per call shows at R=10 (+96 ns). That is lookup plus blob decode, and it carries the cycle and fire ratios.
  - The slope (2.1 ns/run against 0.5) carries refresh at large R.
- Options for M4/M7, not done here:
  1. An O(1) active-run counter on the clock entry. It needs a free scalar; all four are in use, and only field 17, the observation-only deadline, could be repurposed. It would make REFRESH O(1) and remove the slope entirely. B is O(R), and so is the §9 bound, so this is an optimisation, not a contract fix.
  2. A `find` that compares keys with a single bounds proof for the whole key instead of one checked read per unit.
  3. A dense run array instead of a linked list, which belongs to M4.

### Complexity bound check (§9)

- REFRESH and the run methods are `O(R + key lengths)`:
  - `any_active` is a single pass bounded by `walk_limit`;
  - the lookups are `bud_map::find`, which is O(key length) expected.
  - Measured: Pruv rises by 190 ns from R=10 to R=100, about 2.1 ns/run. That is linear, with one REFRESH per call, measured at each prepared R as §9 requires.
- BEGIN, ARM, SETTLE, DROP and CLOSE are `O(key lengths)`:
  - field writes are constant;
  - `owner_is` compares sessionId units (key length);
  - DROP calls `bud_map::remove`, which frees the clock and its R runs. That is storage reclamation, linear in what is freed.
- FIRE checks are constant: field 16, `bv`, and `count` (O(1), see M9-3), plus SETTLE/ARM. The halt work (`D`) is M10.
- No call copies the world or replays history. The cycle and fire items stay flat at about 225 and 470 ns over 200k calls.

## Not done in M9 (owned elsewhere)

- The budget record, CURRENT and the ISO activeSince text (M7). This is replaced by the placeholder `load_bv`/`store_bv`.
- `halt(ref,'time')` after status 4 (M10). Q counts these cases as not-implemented.
- raise/charge paths (M5/M6/M7). Q counts these cases as not-implemented.
- Wiring into `bud_api` / `host.rs` / `harness.rs`. Those files are outside M9's scope. The M7/M10 integration calls the interface above.
