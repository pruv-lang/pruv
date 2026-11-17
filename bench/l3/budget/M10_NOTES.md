# L3-10 M10 — halt and decisions (status: M10a halt done, M10b raise / decide not started)

## Scope (REQUEST_BUDGET_SPEC.md §14 M10)
- M10a (this commit): `bud_halt.pruv` (halt section codec in the record's `rest`, halt with ordered effects),
  `m10_test/` (root_m10.pruv, driver_m10.rs, run_m10.py with q / derive / same / perf, --pruvc, --py).
- M10b (next): `bud_decide.pruv` raise (limits / unlimited) and decide (extend / end), hand_m10.rs, perf_ts.mjs.
- Not implemented paths report not-implemented; the host never fills them in: decide / raise (M10b),
  hasCard / setCard (M11), M12 auto / cleanup.

## Design (fixed)
1. Halt section in `rest` (M7 keeps rest opaque): binary prefix
   `[-2, reason (0 time, 1..4 delegations/commands/reviews/checks), atKind (1 text, 2 ms), atMs, atLen, atUnits..,
   nIds, (len, units)*]`, followed by the remaining JSON units (units are 0..65535, so -2 cannot start JSON). halt
   writes atKind 2 (at = now); the runner renders it with toISOString semantics. The runner codec moves a
   well-formed `halt` object (exactly reason / at / stoppedDelegationIds, valid reason, string ids) between JSON and
   the prefix; any other `halt` value stays JSON. `halt_reason(st, off)` reads it for M8's halted-charge reason.
2. halt effect order in fx (B:381-397): SETTLE (bud_clock, may emit timerClear 13) → 30 runningDelegationIds
   consumed → state halted + halt section → 31 cancelRecovery → 32 stopMain → [33, i] stopDelegation of id i (list
   order, duplicates kept) → 34 stopAttachedTurns → 35 holdQueued → 36 save → 20 changed → 37 announce.
   Status: 0 (also when nothing halts: no current / not open), 2 invalid (bad reason / ids, or the record frame fails
   `rec_ok_in`: six raw string lengths must end exactly at the blob end), 3 fx too small (fx[0] + 12 + 2·D > len),
   -N Grow (nothing written; `halt_words` lets a caller that already wrote reserve first). All checks run before the
   first write; root_m10 run_op also checks the nested halt's fx room and `rec_ok_in` before its clock writes.
   `prefix_len` accepts a halt prefix only with reason 0..4, atKind 1/2, raw 0 <= atLen <= l-6 and well-formed ids;
   otherwise rest is opaque JSON (the runner's `prefix_ok` mirrors it).
3. Helpers: the 17 record helpers of bud_lifecycle.pruv are `pub` on main since the M7 perf pass (d28b52c, see
   M7_NOTES 'API for M10'; M10a's identical visibility edit merged into it) and are used directly. M10a is built on
   the M7 host protocol from that pass: isQuiet is prefetched in op slot 2 and consumed by effect [23]; current
   returns the view `[4, n, off]`.
4. Driver: M7 World + M9 timer scheduler (token counter, due = now + effective, by_bid; advance fires due timers in
   (due, token) order through FIRE; on HALT_TIME the root runs halt reason time itself).
5. Comparison per request: value (charge {ok, reason}, current's budget as JV, booleans, undefined) and the effect
   sequence: runningDelegationIds, cancelRecovery, stopMain, stopDelegation (sessionId and id units),
   stopAttachedTurns, holdQueued, save, changed, announce, isQuiet (answer), uuid, carryHeld, timer.set (token,
   delay), timer.clear (token). now / unit / timer.unref / setUnit are inputs or host detail. Whole case: final
   sessions as JV objects (key order not compared), clocks the M9 way, active timers (token, due). save snapshots
   are not compared.

## Review (Codex, fixes required, 6 P2, all fixed)
1. corrupt string length could make halt write past its reservation → `rec_ok_in` before any write (status 2).
2. run_op wrote clock state before checking the nested halt's fx room → fx room and record checked first.
3. `prefix_len` clamped a negative atLen → raw range checks on reason / atKind / atLen / ids.
4. effects between requests were dropped by the comparison → compared (`between`).
5. a missing request still gave exact → requests ran must equal the expected call list.
6. decode_budget took negative reasons via Python negative indexing → `prefix_ok` gate and ValueError on bad fields.

## Known differences / spec notes
- **KD-M10-1 (spec note, §8.1)**: the Need/resume pattern (return a Need status with nothing written, host answers,
  call again) does not fit a query that comes after a write. In halt, SETTLE (which may already clear the timer)
  precedes `runningDelegationIds(ref)`, and FIRE → HALT_TIME has already written state before halt runs; decide's
  `releaseHeld` is in the same position. Adopted: the host prefetches the next answer into the op (deps queue head,
  default []) and Pruv emits a "consumed" marker effect (fx 30) at the exact query position; the host advances its
  queue and logs the query there. M7's perf pass moved isQuiet to the same form (op slot 2, consumed by [23]; status 5
  only when the host passes -1). Spec §8.1 should name this prefetched-answer query form.

## Results (M10a)
Q (`python3 bench/l3/budget/m10_test/run_m10.py q`, work/m10, 300 cases Q4 150 / Q5 150). Rust rows use the pruvc-rs
release built in this worktree after merging main (db2474a, aedd5ab), passed with --pruvc. Besides per-request value
and effects, a case is exact only if the requests that ran equal the script's calls (up to the cut) and the effects
outside requests (timer fires between requests) equal the oracle's (none fire in either set).

| set | compiler | exact | not-implemented | fail | halts compared |
|---|---|---|---|---|---|
| cases.jsonl | Rust pruvc | 78 | 222 (decide 145, raise 61, hasCard 16) | 0 | 59 |
| cases.jsonl | Python pruvc (--py) | 78 | 222 | 0 | 59 |
| cases_halt.jsonl (derived, 87 reach halt) | Rust pruvc | 300 | 0 | 0 | 97 |
| cases_halt.jsonl (derived) | Python pruvc | 300 | 0 | 0 | 97 |

In not-implemented cases every request before the cut is still compared (value and effects) and matches.
Compared requests (derived set): halt 452, charge 292, maybeClose 266, runStarted 254, runWaiting 175, runEnded 159,
reopen 51, current 22, isHalted 16. A mutation check (wrong reason table, altered halt.at) produced failures, so the
comparison is live.

`same`: Rust and Python pruvc output byte-identical (182995 bytes, after the review fixes).

Perf (Pruv only, `perf --rounds 3 --reps 20000`, after the review fixes, one session, halt with D running
delegations; the loop also puts the record back open, `putback` times that put alone): halt D=10 3.3 µs, D=50
14.6 µs, D=100 29.5 µs (~0.3 µs per id, linear); putback ~32 ns. Not compared with hand Rust / TS yet (M10b).

## M10b still needs
- `bud_decide.pruv`: raise (limits / unlimited; unlimited clears an armed timer, then changed) and decide (extend:
  remove the halt section, save, changed, REFRESH with a possible re-halt, queueContinue / releaseHeld with the
  captured reason; end: CLOSE('user-end'), dropHeld, save, changed).
- releaseHeld prefetched answer in the op + consumed marker (KD-M10-1), effect codes for queueContinue / releaseHeld
  / dropHeld, root codes for raise / decide, runner value / effect mapping for them.
- hand_m10.rs and perf_ts.mjs, perf against them; then q on cases.jsonl should leave only hasCard (M11) as
  not-implemented.

## Requests to other owners
- bud_lifecycle.pruv `store`: Grow returned from inside session_put is a defect (already noted in M7 review).
  halt reserves `halt_words` before writing, so it does not hit it, but a nested caller that wrote first could.

## Data
`work/m10/cases.jsonl`, `oracle.jsonl` (generated with q_gen.py), `cases_halt.jsonl`, `oracle_halt.jsonl`
(`run_m10.py derive`) are under work/ and not committed.
