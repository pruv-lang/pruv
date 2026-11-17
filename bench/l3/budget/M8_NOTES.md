# L3-10 M8: activity accounting

Scope: `charge`, `noteFailure`, `noteTool`, `forgetTurn` of `RequestBudgets` (reference B:244–282), spec §4.3, §8, §9.

Files:
- `bud_charge.pruv`: charge decision and counter arithmetic on a charge view frame `cv`; `note_failure` and `add_tool` (the counting half of noteTool).
- `bud_tools.pruv`: the per-turn activity sets in the bud_map world (root 2: turnId → activityIds, both in insertion order). `note_tool`, `forget_turn`.
- `m8_test/root_m8.pruv`: test root (not product). It holds a PLACEHOLDER budget record in the session blob until M7 defines the record, runs CURRENT (session exists, has a budget, budgetId equals ref.budgetId) and dispatches the halt sub-operation frame.
- `m8_test/driver_m8.rs`, `run_m8.py`, `hand_m8.rs`, `perf_ts.mjs`: Q runner, derived set, perf.

All decisions are in Pruv. The host owns the buffers, grows on `-N`, and prints what Pruv returned. It never reads state words and never fills a missing path.

## Sub-operation interface

- `charge(inout cv, what, ws)` statuses: 0 charged `{ok:true}`; 1 `{ok:true}` with nothing charged (no current budget, or closed); 16+r `{ok:false, reason:r}` for a halted budget; 32+k over limit on key k (1..4 = delegations, commands, reviews, checks), nothing charged; 5 ResourceBound; 2 invalid frame.
- On 32+k the root returns status 6 with the halt frame `[20, k, sessionId key, budgetId key]` = `halt(ref, k)`. The halt body is M10, so the root stops there (not-implemented). After M10 runs the halt, the answer is `{ok:false, reason:k}`.
- `note_tool` statuses: 0 already seen (returns before CURRENT), 1 inserted (caller runs CURRENT, then `add_tool`), −N Grow, 2 invalid. The turn entry is created with its first activity, so a turn set is never left empty in the store; TS creates the set before the `has` check, which is the same observable state.
- `forget_turn`: 1 removed, 0 absent.

## Behaviour classification (spec §0, §15)

| behaviour | class | M8 |
|---|---|---|
| over-limit charge charges nothing and halts with the first key in BUDGET_COUNT_KEYS order | intent | kept, oracle-compared |
| limit is inclusive (`used+amount > limit` refuses) | intent | kept |
| unlimited budget is charged but never halts | intent | kept |
| `what[key] ?? 0`; null and undefined both count 0; negative amounts are added and never halt | intent (as written in B) | kept |
| §15 #4–#5 (closed / missing budget answers `{ok:true}`) | keep | kept |
| §15 #6 a missing or stale budget still consumes the noteTool dedup key | keep (chosen accident) | kept, compared via finalToolSeen |
| §15 #7 noteFailure / noteTool count on halted and closed budgets | keep (chosen accident) | kept |
| §15 #27 halted budget without `halt` answers reason `'delegations'` | excluded row | code 17 kept; Q2 never generates it (halted budgets always have `halt.reason`) |
| §15 #26 | excluded row | not reachable from M8 calls |

Known differences (not compared, recorded):
- **KD-M8-1** ResourceBound. TS adds IEEE doubles; once a counter leaves ±(2^53−1) it silently loses precision. Pruv returns status 5 and writes nothing (charge is atomic: all sums are checked before any write). Q2 values never get there.
- **KD-M8-2** Non-integer amounts (e.g. `0.5`, `NaN`, `Infinity`) are not representable in the i64 frame. Q2 generates integers only. A future boundary must reject or map them; nothing in M8 fills them.
- **KD-M8-3** One halt nested in charge: TS calls `deps.now()` inside halt. That belongs to the halt body (M10); the frame M8 emits carries no time.

## Not-implemented paths

- the halt body after an over-limit charge (M10): the frame is checked (see below);
- `reopen`, `halt` (M10), `current`, `isHalted`, `hasCard` (M7 / M11): the case is cut at the first such call.

## Q results

`python3 bench/l3/budget/m8_test/run_m8.py q` (Rust pruvc root, driver_m8):

| set | cases | exact | not-implemented | known-difference | fail |
|---|---|---|---|---|---|
| Q2 seed 20261117 (`work/m8/cases.jsonl`) | 300 | 63 | 237 | 0 | 0 |
| accounting-only derived (`run_m8.py derive`, `work/m8/cases_acct.jsonl`) | 300 | 222 | 78 | 0 | 0 |

Not-implemented reasons, Q2: reopen call 83, halt call 53, halt dispatch (M10) 46, hasCard call 21, isHalted call 18, current call 16.
Derived: halt dispatch (M10) 78; every other call is an M8 method.

Compared calls: Q2 charge 608, noteTool 285, forgetTurn 136, noteFailure 117; derived charge 1038, noteTool 498, noteFailure 213, forgetTurn 207.

Per exact case: every return value, plus every final session's budget presence, state, halt reason, the seven counters, and toolSeen (turn order and activity order).
Per halt dispatch (46 + 78, all checked): the request's oracle value is `{ok:false, reason:k}` with the frame's k; the oracle's halt ran for the frame's ref (`runningDelegationIds(ref)`); the counters at the dispatch equal the oracle's at the halt's `findSession` (nothing charged). The prefix before every cut matched.

The derived set is the same world and script without reopen / halt / current / isHalted / hasCard, re-run through the TS oracle (ids end in `-acct`).

## Compiler parity

Python `pruvc/pruvc.py … --backend rust` and the Rust `pruvc` emit byte-identical Rust for `m8_test/root_m8.pruv` (`cmp`; sha1 `9ac4c161…`, 96343 bytes).

## Perf

`run_m8.py perf --rounds 3` (median ns per call, interleaved rounds). Items:
- `charge`: `charge(ref_i, {commands:1, retries:1})` round-robin over N sessions;
- `failure`: `noteFailure(ref_i)` round-robin;
- `tooldup`: `noteTool` on a turn holding N activities, dedup hit;
- `toolgrow`: N new activities on one turn, then `forgetTurn`; ns per noteTool, forget included.

| item | size | Pruv ns | hand Rust ns | TS ns | Pruv/hand |
|---|---|---|---|---|---|
| charge | 100 | 63.6 | 17.6 | 212.6 | 3.61 |
| charge | 500 | 76.1 | 18.3 | 168.6 | 4.16 |
| charge | 1000 | 72.0 | 18.8 | 230.7 | 3.83 |
| failure | 100 | 53.4 | 15.9 | 35.5 | 3.36 |
| failure | 500 | 65.1 | 15.8 | 41.4 | 4.12 |
| failure | 1000 | 65.9 | 15.1 | 56.6 | 4.36 |
| tooldup | 100 | 50.9 | 30.5 | 16.9 | 1.67 |
| tooldup | 500 | 65.8 | 26.8 | 15.7 | 2.46 |
| tooldup | 1000 | 55.4 | 26.2 | 21.2 | 2.11 |
| toolgrow | 100 | 135.9 | 89.0 | 84.7 | 1.53 |
| toolgrow | 500 | 155.6 | 91.7 | 68.7 | 1.70 |
| toolgrow | 1000 | 155.9 | 98.0 | 83.0 | 1.59 |

**Every row is above the 1.1x bar. Not fixed in M8: the dominant cost is `bud_map::find` (M4, not M8's file).**
Pruv beats TS on charge (2.3–3.2x); on failure, tooldup and toolgrow TS's V8 Map/Set is faster than Pruv.

Profile (`sample`, `-g` build of the M8 driver, `perf 1000`, ~1470 top-of-stack samples over all items):

| function | samples | owner |
|---|---|---|
| `bud_map::find` | 959 (65%) | M4 |
| `load_cv` (bid compare + 14-word copy) | 134 | M8 root |
| `bud_map::add` | 87 | M4 |
| `t_call` | 82 | M8 root |
| `bud_store::valid` | 73 | M4 store |
| drop_entry / fill_entry / put_units / make_room / entry_free | ~96 | M4 |
| `note_tool` | 27 | M8 |
| `store_cv` | 13 | M8 root |

Line by line, charge (one call):

| step | hand Rust (`hand_m8.rs`) | generated Rust (`work/m8/gen.rs`) | cause |
|---|---|---|---|
| session lookup | `sessions.get_mut(&r.session_id)`: SipHash over 13 bytes, SwissTable probe, `memcmp` | `bud_map::find`: one dependent multiply per u16 unit (`mix`), linear probe re-reading header words, per-unit checked compare | M4 map format; same cause measured in M9_NOTES (hash 4 units per mix + slice compare brought M9 near 1.0x). Not editable in M8 |
| CURRENT bid compare | `b.id == r.budget_id` → `memcmp` (36 bytes) | `load_cv` loop, `ent_u16::rd` per unit (two bound checks each) | codegen: Pruv has no bounds-proved slice compare |
| boundary check | none | `bud_store::valid` header check (15 reads) per `t_call` | test-root boundary rule; M4 store |
| record access | field access on a struct | `field_addr` + blob offset, copy 14 words into `cv`, write 7 back | PLACEHOLDER record until M7; frame copy is M8's choice (≈5–8 ns) |
| limit checks | 4 compares, `Option::unwrap_or` | 4 × `amount` (checked reads) + `sum` safe-integer checks; three loops (check, validate, write) | ResourceBound atomicity (KD-M8-1); few ns |

failure has the same lookup and CURRENT path with no limit work, so its ratio is the same lookup gap. tooldup is two `find`s (turn, activity) vs two hash lookups; toolgrow adds `bud_map::add` + `make_room` (+ free on forget) vs `String` alloc + set insert.

What could be done next (outside M8's files): the M9 fixes to `bud_map::hash_key` / `entry_matches` (4 units per mix, slice compare); after M7, read the counters in place instead of copying `cv`.

## Complexity bound check (§9)

Expected normal path; Grow (host copy, rare) and rehash (amortized) are storage upkeep.
- charge: O(sessionId + budgetId length): one `find`, the bid compare, constant field work. Measured flat in N: 63.6 / 76.1 / 72.0 ns at N = 100 / 500 / 1000.
- noteFailure: O(key length); flat: 53.4 / 65.1 / 65.9 ns.
- noteTool: O(turnId + activityId + ref key lengths), no scan of the turn's activities: dedup hit on a turn of N activities is flat: 50.9 / 65.8 / 55.4 ns.
- forgetTurn: O(turnId + A): `bud_map::remove` frees the A children. Measured inside toolgrow as a per-noteTool cost that stays flat (135.9 / 155.6 / 155.9 ns), so forget is linear in A, not worse.
