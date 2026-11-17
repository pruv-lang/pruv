# L3-10 M10 — halt and decisions (status: design only, implementation not started)

Stage cap (~1 h) was exceeded during reading/design. Per the task rules this commit records the state; no Pruv
code, driver or runner exists yet, so there are no Q / parity / perf results.

## Scope (REQUEST_BUDGET_SPEC.md §14 M10)
- `bud_halt.pruv`: halt(ref, reason) with ordered effects; halt section codec in the record's `rest`.
- `bud_decide.pruv`: raise (limits / unlimited) and decide (extend / end).
- Nested flows: charge over-limit (M8 status 32+k), FIRE → HALT_TIME (M9 status 4), decide extend → REFRESH re-halt.
- `m10_test/`: root_m10.pruv, driver_m10.rs, hand_m10.rs, perf_ts.mjs, run_m10.py.
- Not implemented paths (report not-implemented, host never fills in): M11 hasCard/setCard, M12 auto/cleanup.

## Design decisions (fixed)
1. Halt section in `rest` (M7 keeps rest opaque): binary prefix
   `[-2, reason(0 time, 1..4 delegations/commands/reviews/checks), atKind(1 text, 2 ms), atMs, atLen, atUnits..,
   nIds, (len, units)*]`, followed by the remaining JSON units. The runner codec moves a well-formed `halt` object
   between JSON and the prefix; decide extend removes the section. M8's cv[3] halt reason is read from it.
2. Queries (§8.1 Needs). halt's `runningDelegationIds` and decide's `releaseHeld` cannot use M7's
   retry-with-nothing-written Need: SETTLE (timer.clear) precedes runningDelegationIds in the reference order, and
   M9's HALT_TIME has already written state. Test adapter choice: the root passes prefetched answers in the op
   (next runningDelegationIds list from the deps queue head, default []; next releaseHeld bool, default false), like
   M7's uuid, and Pruv emits a "consumed" marker effect at the exact query position; the host advances its queue and
   logs the query there. isQuiet keeps M7's status-5 Need retry. To record as KD / spec note: §8.1 Need/resume does
   not fit a query that follows a write.
3. halt effect order in fx: SETTLE (may emit timer.clear 13) → [rdi marker] → cancelRecovery 10 → stopMain 3 →
   stopDelegation [4, idx] per id (duplicates kept) → stopAttachedTurns 5 → holdQueued 6 → save 1 → changed 2 →
   announce 11. Host sizes fx >= 32 + 2·D.
4. decide extend: save, changed, then REFRESH; if REFRESH re-halts, queueContinue / releaseHeld still run with the
   captured reason. decide end: CLOSE('user-end'), dropHeld 9, save, changed. raise unlimited: timer.clear if armed,
   then changed.
5. Comparison: per-request value; effect sequence with args for stopDelegation (id), queueContinue (reason, wake),
   isQuiet answer; timer.* as codes; final sessions as JV via norm_obj with halt.at rendered by iso(ms). save
   snapshots are not compared.
6. Driver: M7 World + M9 timer scheduler (token counter, due = now + effective, by_bid; advance fires in
   (due, token) order through FIRE; on HALT_TIME the root runs halt itself).

## Helpers to reimplement locally (private in bud_lifecycle.pruv, not editable)
session_of, blob_at, blob_n, state_of, id_is, current_entry, str_at, str_len_in, put_str, blob_copy, rewrite,
store, reserve, grow, view, view_in, close_rec.

## Requests to other owners
- bud_lifecycle.pruv: export the record helpers above (or a `rewrite_rest` entry) so M10 does not duplicate them.
- bud_lifecycle.pruv `store`: Grow returned from inside session_put is a defect (already noted in M7 review).

## Next steps
1. bud_halt.pruv (codec + halt), bud_decide.pruv (raise, decide).
2. root_m10.pruv integrating M7/M8/M9 entries; driver_m10.rs; run_m10.py (q/derive/same/perf, --pruvc, --py).
3. Oracle: `work/m10/cases.jsonl`, `work/m10/oracle.jsonl` (300 cases, Q4 150 / Q5 150) were generated but are
   under work/ (not committed); regenerate with q_gen.py. Derive a 300-case set without hasCard/setCard.
4. fail 0 both sets × both compilers, byte parity, perf D=10/50/100 + retained, Codex review.
