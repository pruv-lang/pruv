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

Python `pruvc/pruvc.py … --backend rust` and the Rust `pruvc` emit byte-identical Rust for `m8_test/root_m8.pruv` (`cmp`; sha1 `13202593…`, 107183 bytes, after the perf pass).

## Perf

`run_m8.py perf --rounds 5` (median ns per call, interleaved rounds). Items:
- `charge`: `charge(ref_i, {commands:1, retries:1})` round-robin over N sessions;
- `failure`: `noteFailure(ref_i)` round-robin;
- `tooldup`: `noteTool` on a turn holding N activities, dedup hit;
- `toolgrow`: N new activities on one turn, then `forgetTurn`; ns per noteTool, forget included.

Perf pass (L3-10 M8 pass 2). Before = the M8 commit, load average 17.98→17.59; after = this pass, load average
5.40→6.45. Host load differs between the two runs, so compare the Pruv/hand ratio and the instruction counts below,
not raw ns across runs.

| item | size | before Pruv / hand / TS ns | before ratio | after Pruv / hand / TS ns | after ratio |
|---|---|---|---|---|---|
| charge | 100 | 91.7 / 30.8 / 221.8 | 2.98 | 37.2 / 15.8 / 133.5 | 2.35 |
| charge | 500 | 92.4 / 42.7 / 302.5 | 2.16 | 44.0 / 18.5 / 135.6 | 2.38 |
| charge | 1000 | 97.4 / 41.6 / 259.2 | 2.34 | 43.6 / 17.0 / 141.8 | 2.56 |
| failure | 100 | 87.4 / 31.2 / 51.9 | 2.80 | 26.4 / 13.9 / 29.0 | 1.90 |
| failure | 500 | 77.2 / 18.5 / 46.0 | 4.17 | 34.3 / 15.0 / 33.7 | 2.29 |
| failure | 1000 | 69.1 / 19.8 / 43.5 | 3.49 | 33.4 / 15.3 / 33.3 | 2.18 |
| tooldup | 100 | 64.1 / 36.2 / 22.1 | 1.77 | 34.8 / 26.7 / 12.3 | 1.30 |
| tooldup | 500 | 73.6 / 35.5 / 35.5 | 2.07 | 47.9 / 25.0 / 13.5 | 1.92 |
| tooldup | 1000 | 45.6 / 27.7 / 39.4 | 1.65 | 46.6 / 25.9 / 14.4 | 1.80 |
| toolgrow | 100 | 173.2 / 125.7 / 113.1 | 1.38 | 95.1 / 79.3 / 58.9 | 1.20 |
| toolgrow | 500 | 178.8 / 116.8 / 102.8 | 1.53 | 113.1 / 86.2 / 58.2 | 1.31 |
| toolgrow | 1000 | 188.6 / 124.5 / 123.0 | 1.51 | 116.3 / 86.0 / 59.5 | 1.35 |

Load-robust measure: instructions retired per call (`/usr/bin/time -l`, difference of two rep counts, item-gated
driver builds, N = 1000):

| item | Pruv before | Pruv after | hand Rust | after/hand |
|---|---|---|---|---|
| charge | 1358 | 863 | 351 | 2.46 |
| failure | 1181 | 736 | 287 | 2.56 |
| tooldup | 836 | 835 | 462 | 1.81 |
| toolgrow | 2650 | 2206 | 1739 | 1.27 |

**No row reaches 1.1x.** Charge and failure lost ~37% of their instructions; tooldup is unchanged (its path is
almost all `bud_map`); toolgrow lost the `load_cv` share. Pruv beats TS on charge (~3x) and ties it on failure;
TS (V8 caches string hashes) still beats Pruv on tooldup and toolgrow.

### Fairness

- Lookup count matches hand: instrumented `find` = 1 per charge / failure, 2 per tooldup (turn, activity), with
  1.07–1.2 probes per find. Hand does 1 `HashMap` lookup per charge / failure and 2 per tooldup.
- Both sides use `black_box` on inputs, same warm-up (1000 charge calls), same sizes and round-robin order.
- The Pruv side also pays the test-root boundary rule (`bud_store::valid` on every `t_call`) and the PLACEHOLDER
  blob record (copy into `cv`, write counters back); hand has neither (a struct field). Both are kept: the first is
  the store's safety contract, the second is M8's frame until M7 defines the record.

### What this pass changed (M8 files only)

- `load_cv`: one range check (`off + n <= len(st)`, `bs + bn <= len(bk)`, `len(cv) >= 15`), then plain proven
  indexing; the budget-id compare is an OR-accumulate (`d |= st[..] ^ bk[..]`, no early exit) that LLVM vectorizes;
  the 14-word copy is plain indexing. The old `rd`/`wr` path stays as the fallback (same tests, same words).
- `load_cv` returns the blob offset; `store_cv` writes counters at that offset directly instead of re-deriving it
  from the session entry (`field_addr` + header reads again; the state does not change between the two). Fast path
  for the 7 writes, total fallback.
- `charge`: amounts read once (`a0..a4`) with direct indexing (`amount` gets a `requires` for the 10-word range);
  the three loops (check / validate / write) are unrolled; still atomic (ResourceBound: nothing written unless
  every sum passes, KD-M8-1).
- `bump`, `t_call`, `key_end`: direct indexing where the range is already checked.

### Remaining gap, line by line (failure, N = 1000: 736 vs 287 instructions)

| step | hand Rust (`hand_m8.rs`) | generated Rust | approx. cost | owner |
|---|---|---|---|---|
| boundary check | none | `bud_store::valid` header check per `t_call` | ~80 instr (failure 736 → 651 with `valid` stubbed to `true`) | M4 store / boundary rule |
| session lookup | SipHash 13 bytes + SwissTable probe + `memcmp` | `bud_map::find`: `hash_key` one dependent multiply per u16 unit, probe re-reading header words, `entry_matches` per unit | the bulk of the rest (~40% of failure samples are `load_cv` with `find` inlined, ~20% `valid`, then `hash_key`) | M4 `bud_map` (not editable here) |
| CURRENT id compare | `memcmp` 36 bytes | vectorized OR-accumulate over 36 words | small now | M8 (done) |
| record access | struct field | `field_addr` + copy 14 words, write 7 back | ~20–30 instr | PLACEHOLDER record until M7 |
| call overhead | everything inlined | separate `fn`s for `load_cv` / `store_cv` / `charge` / `key_end` / `find` | 25–60 instr (all generated fns `#[inline]` in a scratch copy) | compiler (no inline hints) |

charge = failure + ~130 instructions of limit work (5 amount reads, 4 limit compares, 5 safe sums, 5 writes) vs ~65
for hand's `Option` loop; the extra is the safe-integer checks of `sum` (ResourceBound), which hand skips.
tooldup: 2 × `find` (+ `hash_key` each) + `valid` vs 2 SipHash lookups; nothing M8-owned is left on that path
(`note_tool` is ~30 instructions). toolgrow: the dedup `find` misses, then `bud_map::add` hashes the key again
(no "insert at the slot `find` stopped at" API in `bud_map`), plus `make_room` / free on forget, vs `String` alloc
+ set insert.

Next steps (outside M8's files): the M9 `bud_map::hash_key` / `entry_matches` change (4 units per mix, slice
compare); a `find`-returns-slot / `add_at` API so `note_tool` hashes once; a cheaper per-call `valid` (or one check per
batch); inline hints from the compiler; after M7, read the counters in place instead of copying `cv`.

## Complexity bound check (§9)

Expected normal path; Grow (host copy, rare) and rehash (amortized) are storage upkeep.
- charge: O(sessionId + budgetId length): one `find`, the bid compare, constant field work. Measured flat in N: 63.6 / 76.1 / 72.0 ns at N = 100 / 500 / 1000.
- noteFailure: O(key length); flat: 53.4 / 65.1 / 65.9 ns.
- noteTool: O(turnId + activityId + ref key lengths), no scan of the turn's activities: dedup hit on a turn of N activities is flat: 50.9 / 65.8 / 55.4 ns.
- forgetTurn: O(turnId + A): `bud_map::remove` frees the A children. Measured inside toolgrow as a per-noteTool cost that stays flat (135.9 / 155.6 / 155.9 ns), so forget is linear in A, not worse.
