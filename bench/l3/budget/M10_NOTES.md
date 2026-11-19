# L3-10 M10 — halt and decisions (status: M10a halt done, synchronous halt query P24, M10b raise / decide not started)

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
2. halt effect order in fx (B:381-397): SETTLE (bud_clock, may emit timerClear 13) → runningDelegationIds(ref),
   asked synchronously by Pruv at this position (`bud_halt::running_ids`, a trusted host fact; P24) → state halted +
   halt section → 31 cancelRecovery → 32 stopMain → [33, i] stopDelegation of id i of the answer (list order,
   duplicates kept) → 34 stopAttachedTurns → 35 holdQueued → 36 save → 20 changed → 37 announce.
   Status (P24): 0 (also when nothing halts: no current / not open), 2 invalid (bad reason, the record frame fails
   `rec_ok_in`: six raw string lengths must end exactly at the blob end, or an invalid answer after SETTLE), 3 fx too
   small for the fixed part (fx[0] + 12 > len, nothing written), 6 the answer does not fit (fx: fx[0] + 8 + 2·D >
   len; store: more id words than QUERY_ROOM = 256 plus the free words a rehash finds; terminal, SETTLE applied; or
   a record above 2^30 words, refused before any write),
   -N Grow (nothing written). Room for the record without its id list plus QUERY_ROOM is reserved before SETTLE
   (`halt_words`), so Grow comes before any write; the second reserve after the query can only rehash in place.
   On 2 / 6 after SETTLE, fx holds SETTLE's timerClear and the host applies it as on status 0. The record is read
   in place from the store (no copy); its frame is checked again after each reserve.
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

Confirmation audit (Codex, fixes required, all fixed): `rec_ok_in` now also requires off + n <= len(st) (out-of-range
`rd` reads 0, so an empty array passed); between-request effects are also compared in cut cases, up to the cut
request's first trace entry; `prefix_ok` adds prefix_len's l <= 2^32 bound so the mirror is exact.

## Known differences / spec notes
- **KD-M10-1 (retired in P24)**: M10a's Need/resume workaround. The host prefetched the next runningDelegationIds
  answer into the op and Pruv emitted a "consumed" marker (fx 30) at the query position. The marker did not prove
  when the host computed the answer, so P24 replaced it with a synchronous query (`running_ids`) that Pruv calls
  after SETTLE. The test host logs the query with the effect count at call time; q checks it comes after SETTLE's
  timerClear, and the query-before-SETTLE mutant is killed (P24 results below). M7's isQuiet already used the
  synchronous form (`is_quiet`); decide's releaseHeld (M10b, P31) must use it too.

## Results (M10a)
Q (`python3 bench/l3/budget/m10_test/run_m10.py q`, work/m10, 300 cases Q4 150 / Q5 150). Rust rows use the pruvc-rs
release built in this worktree after merging main (db2474a, aedd5ab), passed with --pruvc. Besides per-request value
and effects, a case is exact only if the requests that ran equal the script's calls (up to the cut) and the effects
outside requests (timer fires between requests, up to the cut in not-implemented cases) equal the oracle's (none
fire in either set).

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

`same`: Rust and Python pruvc output byte-identical (183064 bytes, after the review and audit fixes).

Perf (Pruv only, `perf --rounds 3 --reps 20000`, after the review fixes, one session, halt with D running
delegations; the loop also puts the record back open, `putback` times that put alone): halt D=10 3.3 µs, D=50
14.6 µs, D=100 29.5 µs (~0.3 µs per id, linear); putback ~32 ns. Not compared with hand Rust / TS yet (M10b).

## Results (P24: synchronous halt)
Gates after P24 (Rust and Python pruvc, byte-identical Rust; raw logs `work/p24/`, not committed):

| gate | rs | py |
|---|---|---|
| q cases.jsonl (Q4/Q5) | 78 exact / 222 not-implemented / 0 fail, 59 queries compared | same |
| q cases_halt.jsonl (derived cuts) | 300 / 0 / 0, halts compared 97 | same |
| replay (cases / cases_halt) | identical, 981 / 1687 calls, 47 / 76 grown | same |
| same | 285490 bytes identical | - |
| mutant_halt.py | control CLEAN, 12/12 mutants KILLED (query_before_settle, no_settle, query_budget_as_session, query_session_as_budget, bad_answer_ok, bad_answer_drops_settle, big_answer_drops_settle, no_stop_loop, stop_reversed, cancel_after_stopmain, no_save, big_answer_continues) | - |

The q comparison now includes the query itself, as it happened: the driver logs the answer `running_ids` actually
returned, at the effect count of the call, and `X armed` when the budget's timer was still armed (a query before
SETTLE). runningDelegationIds must follow SETTLE's timerClear with the ref's sessionId and budgetId, and the stops must
carry that answer. G fixtures (mutant_halt.py): G1 answer order (2 ids, 0 ids); G2 malformed answer → status 2,
G3 an answer too big for fx → 6, G3b an answer too big for the store room → 6, each with exactly SETTLE's timerClear,
the query and nothing else, the session not halted and the record holding SETTLE's result (`keep_settled`); G4 a
forced Grow: the first call returns -N with no query and `st`/fx unchanged, the retry queries once.
`query_before_settle` must die in q with a reason naming `X armed` or `runningDelegationIds`. The `X armed` check is
replayed host-side after the call, so it checks the host-visible order. A run op rewrites its settled record before a
nested halt, so a nested halt that fails keeps a consistent record.

Perf (`run_m10.py perf --rounds 7 --reps 50000`, interleaved with `hand_m10.rs`, ns; the driver names each stopped
id as the hand lane does): halt D=10 319.8 vs 332.5 (0.96, overlap), D=50 682.3 vs 1129.9 (0.60), D=100 1035.7 vs
2118.5 (0.49); putback 28.8-29.2 vs 73.2-78.6 (0.37-0.40). Allocations per rep: Pruv 4, hand 17 / 57 / 107.
Details: L3_RESTRUCTURE_SPEC.md Appendix P24.

## M10b still needs
- `bud_decide.pruv`: raise (limits / unlimited; unlimited clears an armed timer, then changed) and decide (extend:
  remove the halt section, save, changed, REFRESH with a possible re-halt, queueContinue / releaseHeld with the
  captured reason; end: CLOSE('user-end'), dropHeld, save, changed).
- releaseHeld as a synchronous host query at its position (as halt's runningDelegationIds, P24), effect codes for queueContinue / releaseHeld
  / dropHeld, root codes for raise / decide, runner value / effect mapping for them.
- hand_m10.rs covers halt (P24); raise / decide rows in it and perf_ts.mjs; then q on cases.jsonl should leave only hasCard (M11) as
  not-implemented.

## Requests to other owners
- bud_lifecycle.pruv `store`: Grow returned from inside session_put is a defect (already noted in M7 review).
  halt reserves `halt_words` before writing, so it does not hit it, but a nested caller that wrote first could.

## Data
`work/m10/cases.jsonl`, `oracle.jsonl` (generated with q_gen.py), `cases_halt.jsonl`, `oracle_halt.jsonl`
(`run_m10.py derive`) are under work/ and not committed.
