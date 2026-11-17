# L3-10 scenario, observation and expectation format

Fixture files: `fixtures/<G>.json` (groups A..N, ten each, ids `A01`..`N10`, 140 total), written by `gen_fixtures.py`.
A fixture is `{id, name, rule, expect, kd?, world, instances?, script}`. The oracle and an implementation read only
`{id, world, instances?, script}` (`common.strip_case` drops `name rule expect layer killer note kd`).
`rule` cites B line numbers (`bench/l3/budget/reference/backend-request-budget.ts`) and a spec section of `docs/REQUEST_BUDGET_SPEC.md`.
`kd` lists the known-difference rows (KD-1, KD-2) a fixture touches; `judge.py` (later) checks the Pruv side for them.

## Scenario
```
{id, domain?, world:{sessions:[JV session], unit: JV BudgetUnit, clockStart?: ms (default 1700000000000),
 uuids?:[string], deps?:{runningDelegationIds?:[[string]...], isQuiet?:[bool], releaseHeld?:[bool], throws?:{dep:[n]}},
 timerHasUnref?: bool (default true)}, instances?:["A"], script:[step]}
```
`deps.throws` exists in the contract but is never generated (spec 0.5). Dependency queues are consumed in call order; an exhausted queue
yields the defaults `runningDelegationIds []`, `isQuiet false`, `releaseHeld false` (the use is traced like any reply).
UUIDs: explicit `world.uuids` are consumed in order, otherwise the nth is `00000000-0000-4000-8000-<n as 12 lowercase hex>` (n from 1).
Persisted budget ids in corpus worlds are unique UUIDs (`11111111-1111-4111-8111-…` in fixtures, `<8hex>-0000-4000-8000-…` in Q).
`instances` (default `["A"]`) are separate `RequestBudgets` objects sharing the sessions and the UUID stream; a call picks one with `i`.

### JV (values)
Plain JSON for strings/booleans/null/safe integers; `{u16:[units]}` strings with lone surrogates; `{undef:1}` present-undefined;
`{num:"16 lowercase hex IEEE bits"}` for fractions, -0, NaN, ±Infinity and unsafe integers; `{obj:[[k,v]...]}` ordered object;
`{arr:[...]}`; plain objects/arrays are also accepted. `{sess:"id"}` = the currently stored session object with that id (identity).
Fixtures use plain objects for simple records and `{obj}`/`{arr}` wherever a value inside is not plain (cards, charge amounts with `{undef:1}`).
Arithmetic is integral and safe everywhere except card target counts (`Number.isInteger` is the spec, so `{num}` values are allowed there).

### Steps (`ev`)
| ev | fields | meaning |
|---|---|---|
| `call` | `req`, `i?`, `op`, `args:[JV]`, `now?:[ms]` | `op` is a `RequestBudgets` method (`current isHalted attach replaceForUserMessage reopen charge noteFailure noteTool forgetTurn runStarted runWaiting runEnded halt maybeClose raise decide setCard hasCard clearAutoContinue recordAutoContinue stopAutoContinue bootRestore forgetSession dispose`) or a helper (`limitText refusalWire refusal continueText cardMissing budgetUnitOf`). `args.length` is the arity (omit trailing args to make them absent). The corpus never sets `now` (KD-1: one `now` per public call). |
| `advance` | `ms` | now += ms, firing due timers in (due, registration) order; while a timer callback runs, `now()` reads that timer's due time. |
| `setTime` | `now` | absolute virtual time, may go back, fires nothing. |
| `deliverTimer` | `token:"t<k>"`, `captured?` | tokens are numbered `t1, t2, …` by `setTimeout` registration order across the scenario. A pending token is removed from the scheduler and its callback runs now (early delivery allowed). A token that is not pending (fired, cleared) needs `captured:true` (a late callback); otherwise it is a driver error. |
| `setUnit` | `unit` | replace the admitted unit between calls. |
| `putSession` / `removeSession` | `session` / `id` | replace (new object identity) or remove a stored session. |
| `setDepReply` | `dep`, `replies` | replace a dependency's remaining reply queue. |
| `snapshot` | `label` | diagnostic snapshot in the trace. |

Fixtures and Q avoid the one ambiguous corner: an `advance` that crosses a due timer by more than it lands on. Fixtures advance exactly to the due
time (`advance 60000` for a 60000 ms timer) or use `setTime` + `deliverTimer` for overshoot; Q3 only advances exactly onto the next due timer or short of it.

## Observation
```
{id, requests:[{id,instance,op,terminal,value}], trace:[{c,r,i,a?,o?}], finalSessions, finalClocks, finalToolSeen,
 activeTimers, driver, excluded, domain?}
```
Terminals: `ok`, `thrown` (never expected in this corpus), `core`. Session arguments in the trace are `{sref:"s<k>",v}` with the budget as `{bref:"b<k>",v}`.
Void returns are observed as `{undef:1}`.
Trace `c` names (decided here; the oracle must emit exactly these): dependency calls `now unit findSession save changed stopMain stopDelegation
runningDelegationIds stopAttachedTurns holdQueued releaseHeld carryHeld queueContinue dropHeld cancelRecovery isQuiet announce`;
primitives `uuid timer.set timer.unref timer.clear timer.fire`; script mutations `script.<ev>`. `r` = request id, `i` = instance.
`finalSessions` is addressable by session id; `finalClocks` has one entry per live clock, identifiable by budget id; `activeTimers` lists pending timers.

## Expectation DSL (`expect`, checked by `check_oracle.py` on the observation)
All values are compared after JV decoding (`{sref,v}`/`{bref,v}` unwrap to `v`; `{u16}`, `{num}`, `{undef}` decode to distinct values and are written
in the same JV form in `expect`). Object matches are *subset* matches (every expected key must match, extra keys are fine); arrays and scalars match exactly.
```
expect = {
  requests?: [ {req, terminal?, value?, valueKeys?} ],  // req = request id; value = subset match; valueKeys = exact ordered key list of the value
  counts?:   { c: n },        // exact number of trace records with name c, over the whole trace (0 allowed)
  has?:      [ c ... ],       // each name appears at least once
  absent?:   [ c ... ],       // each name never appears
  order?:    [ [c1, c2] ... ],// the first c1 record precedes the first c2 record (both must exist)
  session?:  [ {id, path, value} | {id, path, absent:true} | {id, path, keys:[...]} ],
                              // finalSessions[id] followed by a dotted path (array indexes numeric); keys = exact ordered key list at that path
  clocks?:   { count?: n, has?: [budgetId], absent?: [budgetId] },   // finalClocks
  timers?:   n                // number of pending timers at the end (activeTimers)
}
```
Every fixture has at least one expectation. The comparator of a later milestone checks the whole observation; the DSL only states what the fixture is about.

## Q cases
Each line of `work/q/cases.jsonl` is `{id:"Q<f>-<seed>-<n>", family, seed, world, instances?, script}`; the oracle ignores `family` and `seed`.
Never generated: dependency throws, per-call `now`, a halted budget without `halt`, halt without reason, owner-mismatch settles (spec 15.1 rows 10, 16, 26, 27).

| Family | Coverage |
|---|---|
| Q1 | opening, replacement, reopen, stale references, origins, sizes (also putSession/removeSession/setUnit) |
| Q2 | charges, atomicity, tools, failures, retries (null/negative/undefined amounts, equal-to-limit boundaries) |
| Q3 | multi-run waiting/running transitions and timer delivery (early, late, captured, backward clock, raise) |
| Q4 | halt, quiet close, delegation order, nested halts (charge, fire, decide re-halt) |
| Q5 | raise/decide combinations, retained runs, queue/release order |
| Q6 | card validation, UTF-16 boundaries, optional fields, raw Number classification |
| Q7 | auto-continuation ownership, restoration, cleanup, two instances |
| Q8 | text/helpers, admitted compatibility values, larger inputs (64/200 delegation ids, 300/301-unit texts, activeMs up to 2^53-1 in helpers) |

## Fixture groups (ten each)
| G | Fixtures 01–10 |
|---|---|
| A | attach/open: missing, existing open/halted, closed with live clock, configured default, defaultUnlimited, size 1 override, sizes 2/4/8, explicit unlimited, origins and call order |
| B | replacement/reopen: halted user/schedule/direct+release, open reuse, absent/closed fallback, quiet reopen, user-end, other rejections, stale refs, old-reference invalidation |
| C | charging: missing/stale, closed, halted reasons, time reason, exact limit, first over-limit key, atomic rejection with retries, unlimited, negative/nullish/zero amounts, failures/tools in all states |
| D | tools: first, duplicate, different activities, different turns, same turn across budgets, missing-ref and stale-ref consumption, forgetTurn, no lookup on duplicates, large turn set |
| E | runs: first start, second run, duplicate start, missing runs, same waiting, one active among waiting, all waiting settles, resume, end last run, halted/closed/stale refs |
| F | timers: exact deadline, early rearm, late overshoot, backward clock, zero remaining, unlimited, raise unlimited, stale/unlimited late callbacks, oversized delay and unref, since=0 and run-count quirk |
| G | halt/close: missing, stale, repeated, delegation ids, settle ordering, empty list, quiet false, runs suppress quiet, quiet close without save, halts from charge/fire/decide |
| H | raise: missing/closed, size 1, maximum, no change, time-only while consuming, non-time, unlimited, already unlimited, numeric while unlimited, halted stays halted |
| I | decide: not-found, not-halted, additive extend, sizes, unlimited, release true/false, time reason with both results, user-end, retained runs with re-halt |
| J | card validation: missing/closed, deliverable, done, targets, raw note, verify, conflicts, state, precedence, old-card preservation |
| K | card representation: trimming, astral boundary, lone surrogates, ECMAScript trim, empty arrays, empty/whitespace note, by projection, halted success and hasCard, Number.isInteger edges |
| L | auto-continuation: clear, record on missing/closed, pointed owner, stale fallback, non-open pointer blocks fallback, matching stop, already stopped, synthetic record, no-ops |
| M | restore/cleanup: restore missing/running/halted, usage preserved, forgetSession matching/non-matching, dispose, toolSeen cleared, no settle, two instances |
| N | text/helpers: ko/en counter names, default language, minute floor, unlimited text vs numeric wire limit, refusal/continuation bytes, card texts, budgetUnitOf, no live time |

## Decisions taken by the corpus author
- Fixture rows that the spec table lists but the domain rules forbid were replaced: C04 (halted without `halt`) -> halted `time` reason returned verbatim;
  I08 (missing reason) -> `time` reason with both release results; F10 drops the owner-mismatch part (row 10).
- M07 checks that dispose clears three timers but not their order (the DSL has no per-record argument match).
- Trace names, void value encoding, timer token numbering and the `advance` clock rule above are this document's contract for the oracle.
