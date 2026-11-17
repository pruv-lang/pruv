# Request budget (L3-10) specification

## 0. Migration method (2026-11-17; overrides any conflicting text below)

This piece is the first one built under the rule that a migration is not a drop-in swap. The Lair source is
evidence of what the budget is *meant* to do, not a byte contract to copy.

1. Every observable behaviour of B is classified in §15 as **intended** (a design decision; D1–D9, K4, F05 or a
   comment states it, or the product needs it), **bug** (contradicts the stated intent) or **accidental** (an artefact
   of JavaScript or of statement order that no caller relies on).
2. Only intended behaviour is compared with the Node oracle. Bugs and accidental behaviour that Pruv does not keep are
   listed as *known differences*; the comparator normalizes them, and a small spec judge (`judge.py`) checks the Pruv
   side against this document instead of against Node.
3. The product structure is Pruv-native (§8): one public call is one pure transition
   `(state, call, facts) -> Done(state', effects) | Need(effects_before, query)`. A call needs at most one host
   round trip. The per-step packet/digest ABI of the earlier L3 pieces is not used in the product; it exists only as
   a thin test adapter (`bud_test_abi.pruv`) so the four-pair harness can drive the core.
4. Host code performs side effects only (storage, clock reading, UUID supply, timers, the dependency effects in §3).
   All decisions are in Pruv.
5. Synchronous dependency exceptions are outside the domain (§3): the host contract is that dependency effects do not
   throw. A host failure ends the call with a host-failure terminal and is not compared with the oracle.

Sections 7–16 below were written before this rule; where they say "byte-exact", "exact dependency trace" or
"exception variants", read them through §0 and §15.

**Resident-state amendment (2026-11-17, after `docs/RESIDENT_STATE_SPEC.md` landed).** `BudgetState` is no longer a
value passed in and returned per call. It lives in one host-owned `[i64]` buffer that every root entry receives as
`inout st`; Pruv validates the header and edits the buffer in place. Item 3's `(state, call, facts) -> (state', ...)`
now reads `(inout st, call, facts) -> Done | Need | Grow`, where `Grow(N)` asks the adapter for a larger zeroed
buffer, Pruv copies itself with `grow(prev, inout next)` and the same call is retried. No call encodes, decodes or
copies the whole state; whole-state encoding happens only at the final observation (§8.1–§8.3, §9).

## 1. Scope and evidence

Port `RequestBudgets` and its exported pure helpers into `bench/l3/budget/`. Compare the Pruv implementation against a Node oracle executing the pinned TypeScript, on:

- `py_rust`
- `py_llvm`
- `rs_rust`
- `rs_llvm`

All budget decisions belong in Pruv. Rust provides storage, time, deterministic IDs, timers, dependency effects, and observation recording.

The reference is Lair commit `17001ab3`. Do not edit the pinned files.

| Alias | Repository file |
|---|---|
| B | `bench/l3/budget/reference/backend-request-budget.ts` |
| T | `bench/l3/budget/reference/shared-request-budget.ts` |
| L | `bench/l3/budget/reference/shared-schedule-lists.ts` |
| E | `bench/l3/budget/reference/shared-engine-text.ts` |
| W | `bench/l3/budget/reference/backend-budget-wiring.ts` |
| RA | `bench/l3/req/ABI.md` |
| RS | `docs/ENGINE_IMPORT_REQUEST_SPEC.md` |

Citations such as `B:248–262` refer to the numbered lines of these files.

The design keeps the previous piece’s deterministic oracle and effect-once rule, but not its per-step packet protocol (§0, §8). It does not inherit the engine-import Promise graph, cancellation machinery, or child entry operations: the budget methods are synchronous.

`BudgetWiring` is evidence about dependency boundaries, not code to port. Its queue scans, service stops, notification rendering, and automatic-continuation policy remain outside this piece.

## 2. Supported domain and state

### 2.1 Persistent budget state

Preserve the fields and optional-property distinctions in `T:95–113`:

```text
id, openedAt, origin, state, limits, [unlimited], used,
[activeSince], [halt], [ended], [card], [autoContinue], [follows]
```

Defaults:

```text
limits:
  timeMinutes = 120
  delegations = 24
  commands = 3
  reviews = 2
  checks = 1

used:
  activeMs = 0
  delegations = 0
  commands = 0
  reviews = 0
  checks = 0
  tools = 0
  failures = 0
  retries = 0
```

Sources: `T:23–26,54–64`; initialization: `B:212–220`.

Order-sensitive constants:

```text
BUDGET_COUNT_KEYS = delegations, commands, reviews, checks
LIMIT_KEYS = timeMinutes, delegations, commands, reviews, checks
sizes = 1, 2, 4, 8, unlimited
states = open, halted, closed
reasons = time, delegations, commands, reviews, checks
origins = user, schedule, direct, release
```

Sources: `B:64`; `T:23,71`; `L:30–33`.

Do not enforce `BUDGET_UNIT_RANGE` inside these methods. The class does not validate settings against those ranges.

### 2.2 Volatile state

Each class instance owns:

```text
clocks: Map<budgetId, {
  sessionId,
  runs: Map<runKey, {waiting}>,
  since?: milliseconds,
  timer?: timerToken
}>

toolSeen: Map<turnId, Set<activityId>>
```

Preserve Map/Set insertion order and replacement behavior. Clock keys are budget IDs, not `(sessionId, budgetId)` pairs. Tool deduplication keys are turn IDs, not budget references. Sources: `B:53–60,162–165,269–309`.

Sessions and their budgets have stable object identities during an operation. `current` and successful `setCard` return the existing budget object, not a newly synthesized budget.

### 2.3 Input domain

The normal domain consists of:

- Plain session, budget, reference, unit, and auto-continuation records with the pinned field types.
- Dense arrays and ordinary own properties.
- Arbitrary UTF-16 strings, including empty IDs, astral characters, combining marks, and lone surrogates.
- Safe integral arithmetic inputs for usage, limits, charge amounts, and clock milliseconds.
- Date inputs within JavaScript’s valid integral millisecond range.
- Typed sizes, reasons, origins, and choices.
- Arbitrary plain values in card fields, because card validation is part of the implementation.

Compatibility fixtures also cover:

- Missing versus present-undefined optional fields.
- Null charge amounts, which use `?? 0`.
- Negative charge amounts.
- Halted budgets without a `halt` field.
- Stale references and unusual but type-valid persisted state combinations.
- Raw IEEE Number values in card target counts, including fractions, negative zero, NaN, infinities, and integral values outside the safe-integer range.

`Number.isInteger` for card target counts must not be replaced with `Number.isSafeInteger`.

Excluded from the initial differential domain:

- Proxies, getters, sparse arrays, cyclic records, custom prototypes, and monkey-patched built-ins.
- Reentrant dependency implementations that call back into the same class before returning.
- Arbitrary external mutation of returned object aliases.
- Wrongly typed numeric fields outside card validation.
- Fractional/non-finite budget arithmetic inputs.

Report excluded inputs explicitly; never count them as exact successes. Do not narrow Unicode support.

## 3. Dependency boundary

The original dependency interface is `B:13–46`; actual bindings are `W:110–129`.

| Dependency | Boundary | Responsibility |
|---|---|---|
| `now()` | Host query | Return scripted milliseconds. Pruv decides when to call and formats ISO text. |
| `unit()` | Answered from Pruv state | Read the admitted settings-unit value at this exact call site. Record the logical dependency call. |
| `findSession(id)` | Answered from Pruv state | Pruv selects the session by ID. Host stores indexed records but performs no budget lookup policy. |
| `save(session)` | Host effect | Record/persist the exact Pruv-selected session snapshot. |
| `changed()` | Host effect | Record notification at the exact point emitted. |
| `stopMain(sessionId)` | Host effect | Record the stop request. |
| `stopDelegation(sessionId, delegationId)` | Host effect | Record each call, preserving order and duplicates. |
| `runningDelegationIds(ref)` | Host query | Return the scripted external-service list. Pruv copies and consumes it. |
| `stopAttachedTurns(ref)` | Host effect | Record the request; do not implement W’s scan here. |
| `holdQueued(session, budgetId)` | Host effect | Record arguments and current session snapshot. |
| `releaseHeld(session, budgetId)` | Host effect with boolean reply | Record the effect and return the scripted result. Pruv chooses the wake fallback. |
| `carryHeld(owner, oldBudgetId, newRef)` | Host effect | Record the migration request. |
| `queueContinue(session, budgetId, reason, wake)` | Host effect | Record exactly these arguments; do not independently choose text, reason, or wake. |
| `dropHeld(session, budgetId)` | Host effect | Record the request. |
| `cancelRecovery(session)` | Host effect | Record the request. |
| `isQuiet(session)` | Host query | Return the scripted service-level fact. Pruv decides whether to ask and whether to close. |
| `announce(session)` | Host effect | Record the call. UI wire rendering is outside this piece. |

Additional primitives:

| Primitive | Responsibility |
|---|---|
| `randomUUID()` | Host returns the next deterministic UUID. |
| `setTimeout(callback, delay)` | Host registers the Pruv-selected callback/ref and delay. |
| `timer.unref()` | Host records the optional method invocation. |
| `clearTimeout(timer)` | Host cancels the named timer. |
| Arena read/write | Mechanical storage of Pruv-owned words; no dependency trace. |

The external answers `runningDelegationIds`, `isQuiet`, and `releaseHeld` cannot be inferred from `BudgetSession`, whose interface contains only `id` and `budget`. Do not invent queue/service logic in Rust to derive them.

In particular:

- W finds running delegations across sessions: `W:134–143`.
- W releases held queues across sessions: `W:171–184`.
- W renders continuation text and allocates a queue-item UUID: `W:215–222`.
- W computes quietness from additional service state: `W:234–247`.

Those implementations are outside the port. The budget oracle records one dependency call at each corresponding boundary, not W’s nested calls.

Dependency effects are assumed not to throw (§0.5). B has no rollback layer, so a throw would leave a partial prefix; that case is excluded-domain (§15 #26) and not generated by Q.

## 4. Behavioral contract

### 4.1 Notation

The following macros describe exact call expansion:

- `CURRENT(ref)`: `findSession(ref.sessionId)`, then test budget existence and exact ID equality.
- `SETTLE`, `DROP`, `CLOSE`, `REFRESH`, `BEGIN`, `ARM`, and `FIRE`: the algorithms in §5.
- `void`: JavaScript `undefined`, not null.

All dependency calls inside macros remain observable. Do not coalesce repeated `findSession` calls.

Unless listed, a method emits no text and makes no `save` or `changed` call.

### 4.2 Construction, lookup, opening, and replacement

| Method | Exact behavior |
|---|---|
| `constructor(deps)` | Create empty clocks and toolSeen; retain deps. No dependency calls. `B:162–167`. |
| `current(ref)` | `CURRENT(ref)`. Return the current budget object, including halted/closed budgets, or undefined. `B:169–173`. |
| `isHalted(ref)` | `CURRENT(ref)`; return whether its state is exactly `halted`. Missing/stale references return false. `B:175–178`. |
| `attach(session, origin, size?)` | Read the supplied session directly. If a budget exists and is not closed, return its reference unchanged. Do not apply size or origin to it. Otherwise run OPEN below. `B:180–185`. |
| `replaceForUserMessage(session, size?, origin='user')` | If the supplied budget is halted: `CLOSE(old,'user-message')`, OPEN, then return immediately for origin `schedule`. Otherwise set the new budget’s `follows=old.id`, call `carryHeld(session,old.id,newRef)`, and return newRef. No extra save/changed after follows. All non-halted cases delegate to attach. The implementation tests only `origin==='schedule'`; `direct` and `release` also take the follows/carry path. `B:187–203`. |
| `reopen(ref)` | `CURRENT`. Return false unless budget is closed with `ended.how==='quiet'`. Success sets state open, deletes ended, calls changed, returns true. Does not refresh a clock or save. `B:227–235`. |

OPEN is private but normative, `B:205–225`:

1. If `session.budget` exists, `DROP(stale.id)`.
2. Call `unit()`.
3. Choose `size ?? (unit.defaultUnlimited ? 'unlimited' : 1)`.
4. Set multiplier to one for unlimited, otherwise the selected size.
5. Call `randomUUID()`.
6. Call `now()` and format its value as `new Date(ms).toISOString()`.
7. Construct the budget in source property order:
   `id, openedAt, origin, state, limits, [unlimited], used`.
8. Assign `session.budget`.
9. Call `changed()`.
10. Return `{sessionId:session.id,budgetId:budget.id}`.

Unlimited retains the numeric unit limits. OPEN does not call save.

### 4.3 Charging and activity counters

#### `charge(ref, what)`

Source: `B:244–262`.

1. `CURRENT(ref)`.
2. Missing or closed: return `{ok:true}` without charging.
3. Halted: return `{ok:false,reason:budget.halt?.reason ?? 'delegations'}`.
4. Visit `delegations,commands,reviews,checks` in that order.
5. For each, let `amount=what[key] ?? 0`.
6. If not unlimited, amount is positive, and `used[key]+amount > limits[key]`, invoke `halt(ref,key)` and return `{ok:false,reason:key}`.
7. Only after every capped counter passes, add each amount to its counter.
8. Add `what.retries ?? 0` to retries.
9. Return `{ok:true}`.

Equality to a limit succeeds. A rejected multi-counter charge updates none of its requested counters or retries, although halt may settle elapsed time. Negative amounts are added without a lower-bound clamp. Successful charge neither settles time nor saves/notifies.

#### Other counter methods

| Method | Exact behavior |
|---|---|
| `noteFailure(ref)` | `CURRENT`; if present increment failures, regardless of open/halted/closed state. Return void. `B:264–267`. |
| `noteTool(ref,turnId,activityId)` | Get/create the turn’s Set first. If activity is already present, return immediately without CURRENT. Otherwise insert activity, then CURRENT, and increment tools if a budget exists. A missing-budget call still consumes the dedup key. Return void. `B:269–277`. |
| `forgetTurn(turnId)` | Delete that entire Set; no session lookup. Return void. `B:279–282`. |

### 4.4 Run registration

| Method | Exact behavior |
|---|---|
| `runStarted(ref,runKey)` | CURRENT; missing returns void. Otherwise get/create the budget clock with owner `ref.sessionId`, set the run to `{waiting:false}`, then REFRESH. Existing keys keep their Map position but reset waiting to false. Halted/closed current budgets are not rejected at the first check. `B:286–290,305–309`. |
| `runWaiting(ref,runKey,waiting)` | Look up clock by budgetId and run by key. Missing run or unchanged waiting returns without CURRENT. Otherwise update waiting, then REFRESH. Return void. `B:292–297`. |
| `runEnded(ref,runKey)` | Look up clock by budgetId; delete the key. Missing clock/key returns without CURRENT. A successful deletion invokes REFRESH. Return void. `B:299–303`. |

Neither waiting nor ending validates session ownership before modifying the clock’s run map. Preserve this behavior for stale/mismatched references.

### 4.5 Halting and quiet closure

#### `halt(ref, reason)`

Source: `B:380–397`.

Exact order:

1. `findSession(ref.sessionId)` for the session.
2. `CURRENT(ref)`, causing a second `findSession`.
3. Return void if session or budget is absent, or budget is not open.
4. `SETTLE(budget,ref.sessionId)`.
5. `runningDelegationIds(ref)`.
6. Set state to halted.
7. Call `now()`, then assign:

   ```text
   halt = {
     reason,
     at: ISO(now),
     stoppedDelegationIds: copy of returned list
   }
   ```

8. `cancelRecovery(session)`.
9. `stopMain(ref.sessionId)`.
10. For every returned ID in order, `stopDelegation(ref.sessionId,id)`.
11. `stopAttachedTurns(ref)`.
12. `holdQueued(session,budget.id)`.
13. `save(session)`.
14. `changed()`.
15. `announce(session)`.
16. Return void.

No deduplication of delegation IDs. No additional model run or summary generation. The run map remains; settle stops consumption but does not remove runs.

Repeated halt on the same halted budget still makes the two lookups, then performs no further effects.

#### `maybeClose(session)`

Source: `B:399–408`.

1. Read supplied budget; missing/non-open returns false.
2. If its clock has any runs, return false without calling `isQuiet`, even if every run is waiting.
3. Call `isQuiet(session)`; false returns false.
4. `CLOSE(budget,session.id,'quiet')`.
5. Call changed.
6. Return true.

Do not save here.

### 4.6 Raising and deciding

#### `raise(ref,size)`

Source: `B:410–446`.

1. CURRENT; missing/closed returns false.
2. Read clock by budgetId.
3. For `unlimited`:
   - Already unlimited returns false.
   - Set unlimited true.
   - Clear a truthy clock timer, if present.
   - Set clock.timer to undefined when a clock exists.
   - Preserve since, activeSince, used, runs, and halted state.
   - Call changed; return true.
4. Size one returns false without unit.
5. Call unit.
6. In LIMIT_KEYS order compute `next=max(current,size*unit[key])`.
7. Update only changed limits and track whether timeMinutes changed.
8. No changes returns false.
9. If time increased, clock.since is defined, and budget is not unlimited: SETTLE, then REFRESH.
10. Call changed and return true.

Numeric raise can increase stored limits on an already-unlimited budget. It never clears unlimited, and it never automatically changes halted to open.

#### `decide(sessionId,budgetId,choice,size?)`

Source: `B:448–477`.

1. Call `findSession(sessionId)` once.
2. Missing session/budget or ID mismatch returns `{ok:false,code:'not-found'}`.
3. State other than halted returns `{ok:false,code:'not-halted'}`.
4. For extend:
   1. Capture `budget.halt?.reason`.
   2. Unlimited size sets unlimited true without unit.
   3. Otherwise call unit and add `(size ?? 1)*unit[key]` to **every** LIMIT_KEYS entry.
   4. Set state open.
   5. Delete halt.
   6. Save session.
   7. Changed.
   8. REFRESH the budget reference.
   9. If captured reason is truthy, `queueContinue(session,budget.id,reason,false)`.
   10. Call `releaseHeld(session,budget.id)`.
   11. If false, call `queueContinue(session,budget.id,null,true)`.
   12. Return `{ok:true}`.
5. For end:
   1. CLOSE with `user-end`.
   2. `dropHeld(session,budget.id)`.
   3. Save.
   4. Changed.
   5. Return `{ok:true}`.

Extend is additive, unlike raise’s maximum. Usage is preserved. Save/changed precede REFRESH, so their snapshots need not contain the newly established activeSince.

If REFRESH immediately halts again, the source still proceeds to queue/release using the previously captured reason. Do not add an early return.

### 4.7 Work cards

#### `setCard(ref,card,language='ko',by?)`

Source: `B:479–514`; validators: `B:145–154`.

1. Select the language’s card-error table.
2. Call `findSession(ref.sessionId)`.
3. CURRENT, causing a second lookup.
4. Missing session/budget or closed budget returns the missing error.
5. Validate deliverable: string whose trimmed UTF-16 length is 1–300.
6. Validate done: dense array of 1–5 validText strings.
7. Construct:

   ```text
   next = {
     deliverable: trim(input.deliverable),
     done: input.done.map(trim),
     updatedAt: ISO(now())
   }
   ```

   This now call happens before optional-field validation.

8. If targets is not undefined:
   - Read confirmed, candidate, unknown.
   - Require every count to satisfy `Number.isInteger(count) && count>=0`.
   - Require targets to be truthy.
   - Note may be undefined, or a string of raw UTF-16 length at most 300.
   - Store the three counts.
   - Include note only when it is a nonempty string; do not trim it.
9. If verify is not undefined:
   - Require an array of at most five items.
   - Each item is truthy, label passes validText, and status belongs to planned/done/skipped.
   - Store trimmed labels and unchanged statuses.
10. If conflicts is not undefined:
    - Require 0–5 validText strings.
    - Store their trimmed forms.
11. If state is not undefined:
    - Require a string in working/done/waiting-user/blocked.
    - Store it.
12. If by is truthy, store only `{sessionId:by.sessionId,turnId:by.turnId}`.
13. Assign `budget.card=next`.
14. Save session.
15. Changed.
16. Return `{ok:true,budget}` with the original budget identity.

First error wins in this order:

```text
missing → deliverable → done → targets → verify → conflicts → state
```

On validation failure, return `{ok:false,error:text}`; preserve the old card. An optional-field error may nevertheless have consumed one now call.

Ignore supplied card.updatedAt, card.by, and unknown fields. An omitted optional field replaces, rather than merges with, the old card field.

`hasCard(ref)`, `B:516–518`, returns `Boolean(CURRENT(ref)?.card)`, without validating the card or requiring an open budget.

### 4.8 Automatic-continuation records

| Method | Exact behavior |
|---|---|
| `clearAutoContinue(ref)` | CURRENT; delete autoContinue only if truthy. Return void. No save/changed. `B:520–524`. |
| `recordAutoContinue(ref,record)` | CURRENT; missing returns false. If record is truthy assign it directly; otherwise delete autoContinue. Return true. No state restriction, copy, save, changed, or count-limit enforcement. `B:526–532`. |
| `stopAutoContinue(sessionId,ref?)` | Algorithm below. `B:534–553`. |

`stopAutoContinue`:

1. If ref exists, CURRENT(ref); otherwise pointed is undefined.
2. Find owner using `ref.sessionId` if pointed exists, otherwise the supplied sessionId.
3. Choose `pointed ?? owner?.budget`.
4. Return void if owner/budget is absent or budget is not open.
5. If record.sessionId equals supplied sessionId:
   - Truthy stopped returns immediately.
   - Otherwise set record.stopped=true.
6. Else, only if there is no record, card.state is working, and card.by.sessionId equals supplied sessionId:
   - Assign `{sessionId,userTurnId:'',count:0,stopped:true}`.
7. Otherwise return.
8. Save **owner**, then changed.

An existing pointed closed/halted budget prevents fallback to the caller’s own open budget. A stale pointed reference allows fallback. Do not overwrite another session’s record.

`AUTO_CONTINUE_MAX=2` exists in `T:92–93`, but this class does not enforce it.

### 4.9 Restoration and disposal

| Method | Exact behavior |
|---|---|
| `bootRestore(session)` | Missing budget: return. Otherwise DROP its clock and delete activeSince. Preserve used, state, halt, card, autoContinue, and toolSeen. Do not charge downtime or arm a timer. Return void. `B:555–560`. |
| `forgetSession(sessionId)` | Visit clocks in insertion order and DROP every clock whose stored sessionId matches. Do not remove sessions, budgets, toolSeen, or persisted activeSince. Return void. `B:562–565`. |
| `dispose()` | Visit clocks in insertion order and clear each truthy timer. Clear clocks, then toolSeen. Do not settle usage or edit session budgets. Return void. `B:567–572`. |

## 5. Time-consumption clock

### 5.1 REFRESH

Source: `B:311–325`.

1. CURRENT(ref).
2. Read clock by budgetId.
3. Missing budget/clock returns.
4. Compute:

   ```text
   consuming =
     budget.state == open
     && runs.size > 0
     && some run has waiting == false
   ```

5. Compare consuming with `clock.since !== undefined`.
6. If equal, return without now, clear, or rearm.
7. If consuming is false, SETTLE.
8. Otherwise BEGIN followed by ARM.

The source materializes `[...runs.values()]` before `some`; reference work is linear in run count. The implementation may scan directly because run entries are ordinary data, but must preserve all observable behavior.

### 5.2 BEGIN

Source: `B:327–331`.

1. Call now.
2. Assign clock.since.
3. Assign `budget.activeSince=ISO(clock.since)`.

No save/changed.

### 5.3 ARM

Source: `B:333–342`.

1. Unlimited returns without a timer.
2. Compute `left=limits.timeMinutes*60000-used.activeMs`.
3. If left is at most zero, invoke FIRE synchronously.
4. Otherwise call `setTimeout(() => FIRE(ref),left)`.
5. Assign returned timer to clock.timer.
6. Invoke its optional unref method if available.

The reference fake uses object timer tokens, including a truthy token for its first timer. Do not use numeric zero as that first timer.

Record both the requested delay and the scheduler’s effective delay. For integral delays, emulate Node’s timer boundary: values greater than `2147483647` become an effective delay of 1 ms. Positive in-range integral delays are unchanged. No timer is requested for nonpositive left.

### 5.4 SETTLE

Source: `B:360–372`.

1. Look up clock using `budget.id`.
2. Missing clock or different stored sessionId returns zero without clearing anything.
3. If timer is truthy, clearTimeout(timer).
4. Set timer undefined.
5. If since is undefined, return zero.
6. Call now.
7. Compute `added=max(0,now-since)`.
8. Set since undefined.
9. Delete budget.activeSince.
10. Add added to used.activeMs.
11. Return added.

The private numeric return is useful for unit-level tests even though public callers ignore it.

A backward clock contributes zero. Time is wall-clock union time while at least one run is active, not a sum across runs.

### 5.5 FIRE

Source: `B:344–358`.

1. CURRENT(ref).
2. Read clock by budgetId.
3. If clock exists, set clock.timer undefined.
4. Missing budget, non-open state, or unlimited returns.
5. SETTLE.
6. If a clock exists, used.activeMs is still below the time limit, and runs.size is positive:
   - BEGIN.
   - ARM.
   - Return.
7. Otherwise `halt(ref,'time')`.

The early-fire branch tests run count, **not whether any run is non-waiting**. Preserve that distinction.

Normal timer delivery removes the scheduler registration before calling FIRE. Because FIRE clears the clock’s timer field before SETTLE, this path normally emits no clearTimeout for the fired timer.

Late callbacks can refer to a replaced budget or an unlimited budget and must still perform the source’s initial lookup and timer-field clearing.

### 5.6 DROP and CLOSE

DROP, `B:374–378`:

1. Find clock by budgetId.
2. Clear a truthy timer if present.
3. Delete the clock.
4. Do not settle or delete persisted activeSince.

CLOSE, `B:237–242`:

1. SETTLE.
2. Set state closed.
3. Call now and set `{how,at:ISO(now)}`.
4. DROP.

A consuming CLOSE can call now twice: once for settling and once for ended.at. Do not combine them.

## 6. Text and pure helpers

### 6.1 Language selection

`engineText(entry,language)` chooses:

```text
entry[language] ?? entry.ko ?? entry.keep
```

It throws only if that result is undefined. Source: `E:53–58`.

Budget public text methods default omitted/undefined language to ko. The typed language domain is ko/en. No process-global language, screen-language conversion, or answer-language selection belongs here.

### 6.2 Helper behavior

| Helper | Behavior | Source |
|---|---|---|
| `limitText(budget,limit,language='ko')` | Unlimited returns localized no-limit text; otherwise JavaScript String(limit). | `B:110–115` |
| `amountText` | Time uses `floor(activeMs/60000)` and timeMinutes; other reasons use used[reason] and limits[reason]. Join with `/`. Private helper. | `B:117–121` |
| `refusalWire(reason,budget)` | `{key:'budget.refusal.'+reason,args:{used,limit}}`; floor time usage to minutes. Keep numeric limit even when unlimited. | `B:123–128` |
| `refusal(reason,budget,language='ko')` | Localized refusal template with counter name and amount. | `B:130–133` |
| `continueText(reason,budget,language='ko')` | Localized continuation template with counter name and amount. | `B:135–138` |
| `cardMissing(language='ko')` | Localized fixed string. | `B:140–143` |
| `budgetUnitOf(setting)` | Falsy setting returns copied defaults plus defaultUnlimited:true. Otherwise project the five limit fields only. | `B:156–160` |

None of these helpers calls a dependency or settles live elapsed time.

Counter names, `B:68–74`:

| Counter | ko | en |
|---|---|---|
| time | 시간 | time |
| delegations | 위임 | delegations |
| commands | 빌드·명령 | commands |
| reviews | 감사 | reviews |
| checks | 실제 확인 | checks |

No-limit text: `제한 없음` / `no limit`.

Exact templates, where `{counter}` and `{amount}` are substituted without additional formatting:

```text
refusal.ko:
예산 한도({counter} {amount})에 닿아 사용자가 늘리거나 끝내기를 고를 때까지 멈춘다. 새 위임·명령을 시도하지 않고 지금까지 결과와 확인하지 못한 것만 짧게 보고한다.

refusal.en:
The request budget limit ({counter} {amount}) was reached, so work is paused until the user extends it or ends it. Do not try new delegations or commands; briefly report the results so far and what you could not verify.

continueText.ko:
사용자가 예산 한도({counter} {amount})를 늘렸다. 앞서 끊긴 도구 호출은 사용자 중단이 아니라 이 예산 한도에 닿아 앱이 멈춘 것이다. 멈춘 지점에서 이어 간다.

continueText.en:
The user raised the request budget limit ({counter} {amount}). The tool call that was cut off earlier was stopped by the app at this budget limit, not by the user. Continue from where you stopped.

cardMissing.ko:
위임 전에 work_card 도구로 산출물과 완료 조건을 먼저 적는다.

cardMissing.en:
Before delegating, write the deliverable and the done conditions with the work_card tool.
```

Exact card errors, `B:88–107`:

| Error | ko | en |
|---|---|---|
| missing | 지금 예산을 찾을 수 없어 작업 카드를 저장하지 못했다. | The current budget was not found, so the work card was not saved. |
| deliverable | deliverable 은 1~300자여야 한다. | deliverable must be 1–300 characters. |
| done | done 은 각 300자 이하 1~5줄이어야 한다. | done must be 1–5 lines of at most 300 characters each. |
| targets | targets 의 수는 0 이상 정수여야 하고 note 는 300자 이하여야 한다. | targets counts must be non-negative integers and note at most 300 characters. |
| verify | verify 는 label 1~300자와 status(planned·done·skipped)를 가진 0~5개 항목이어야 한다. | verify must be 0–5 items with a label of 1–300 characters and a status (planned, done or skipped). |
| conflicts | conflicts 는 각 300자 이하 0~5줄이어야 한다. | conflicts must be 0–5 lines of at most 300 characters each. |
| state | state 는 working·done·waiting-user·blocked 중 하나여야 한다. | state must be one of working, done, waiting-user or blocked. |

String equality is by UTF-16 units. Do not normalize punctuation, whitespace, Korean text, or Unicode normalization form.

## 7. Oracle, scenarios, and observations

### 7.1 Execute the pinned class

Use a Node loader in the style of `bench/l3/req/reference-hooks.mjs`:

- Load B directly.
- Redirect its shared imports to the corresponding pinned files.
- Redirect `node:crypto` to a deterministic UUID provider.
- Install deterministic timer functions before constructing a class instance.
- Resolve E’s supporting imports to the pinned supporting files already present in `reference/`.
- Do not load W as the implementation.
- Do not substitute a rewritten JavaScript budget state machine.

The pin manifest in `reference/README.md` includes SHA-256 values. Verify those when building the oracle.

### 7.2 Deterministic UUIDs and time

Each scenario owns one UUID stream shared across its instances:

```text
world.uuids = optional explicit list
default nth UUID =
  00000000-0000-4000-8000-<n encoded as 12 lowercase hexadecimal digits>
n starts at 1
```

Explicit values are consumed in order. Exhaustion of an explicit stream is a driver error, not permission to invent another value.

Record every UUID call. The host uses the identical stream. Do not alpha-normalize UUIDs in comparisons.

Only OPEN consumes UUIDs in this piece. The UUID allocated inside W’s queueContinue is outside this boundary.

`now()` reads the virtual clock unless a per-call scripted result overrides it. It never increments automatically. Overrides allow consecutive now calls in one synchronous method to return different values.

Pruv performs ISO formatting. Rust must not return preformatted openedAt, activeSince, halt.at, ended.at, or updatedAt.

### 7.3 Scenario format

Use top-level scenarios, avoiding the stale nested-format description noted by the req implementation:

```text
{
  id,
  world: {
    sessions,
    unit,
    clockStart,
    uuids?,
    deps?,
    timerHasUnref?
  },
  instances?: ["A"],
  script
}
```

Script operations:

| Operation | Meaning |
|---|---|
| `call` | Invoke a named public method/helper, with request ID, instance, and arguments. |
| `advance` | Advance virtual time, delivering due timers in `(due,registrationOrder)` order. |
| `setTime` | Change now without delivering timers; supports backward-clock and overshoot tests. |
| `deliverTimer` | Deliver one registered or explicitly captured callback; supports early/late callback fixtures. |
| `setUnit` | Replace the admitted unit between synchronous calls. |
| `putSession` / `removeSession` | Replace/remove a session between calls. |
| `setDepReply` | Change the next scripted service fact. |
| `snapshot` | Request a diagnostic snapshot at a chosen point. |

Method calls and callback bodies run to completion before the next script event. A transport Await is not JavaScript scheduling permission.

Captured late callbacks have explicit tokens. The driver distinguishes a permitted captured callback from an invalid duplicate normal timer delivery.

### 7.4 Dependency scripts

External deps accept ordered reply scripts. Entries are explicit success values or synchronous throws with stable identities.

- `runningDelegationIds`: ordered string list.
- `releaseHeld`, `isQuiet`: booleans.
- Void effects: undefined on success.
- `now`: admitted milliseconds.
- UUID: string.
- Timer set: fresh token with declared optional unref capability.

No dependency fake silently changes budget state. Session changes are emitted by Pruv or by explicit between-call scenario events.

### 7.5 Observation

Emit one observation per scenario:

```text
{
  id,
  requests: [{id, instance, op, terminal, value}],
  trace,
  finalSessions,
  finalClocks,
  finalToolSeen,
  activeTimers,
  driver
}
```

Use req-style trace records:

```text
{c, r, i, a?, o?}
```

Requirements:

- Record every logical dep call, including unit and findSession.
- Record timer.set, timer.unref, timer.clear, timer.fire, uuid, now, and explicit script mutations.
- Preserve invocation order.
- Use stable object-reference labels for session/budget identity.
- Capture session contents at save and mutation-bearing effect boundaries; later mutation must not rewrite an earlier snapshot.
- Snapshot returned values at method return, while separately preserving identity labels.
- Record final volatile state without exposing native timer objects.
- Do not implicitly dispose or drain timers at scenario end.
- Test-adapter encoding traffic is not a logical dependency trace.

Use the req JV conventions for undefined, raw Number bits, ordered objects, arrays, and lone-surrogate strings. Distinguish absent properties from present undefined.

Normal terminals are `ok` and `thrown`. Protocol/resource terminals use `core`, outside ordinary Node equivalence.

The comparator checks the entire observation, not fixture expectation subsets.

## 8. Product interface (Pruv-native) and test adapter

### 8.1 Core transition

The core is one Pruv module set with a single root entry per public method. The budget state is resident: it lives
in a host-owned `[i64]` buffer (`docs/RESIDENT_STATE_SPEC.md`) that every entry takes as `inout st` and edits in
place.

```text
init(inout st) -> Status                               // writes an empty state; Grow(N) if st is too small
step(inout st, call: Call, facts: Facts, inout out) -> Outcome
Outcome = Done { effects: [Effect], result: Result }
        | Need { effects: [Effect], query: Query, resume: Resume }
        | Grow { words: N }                            // st unchanged; adapter grows and retries the same call
        | InvalidInput | ResourceBound(kind)            // st unchanged
resume(inout st, Resume, answer, facts, inout out) -> Outcome   // at most one Need per public call (two for decide extend)
grow(prev: [i64], inout next) -> Status                // copies, rehashes and compacts into a larger buffer
observe(st, inout out) -> Status                       // final observation only
```

- `Facts` is supplied by the host with every call: one `now` (ms) and one fresh UUID. Pruv uses the UUID only when it
  opens a budget. One `now` per public call replaces B's repeated `now()` reads (known difference KD-1).
- `Effect` is an ordered list of host side effects: `save(sessionId)`, `changed`, `stopMain`, `stopDelegation`,
  `stopAttachedTurns`, `holdQueued`, `carryHeld`, `queueContinue`, `dropHeld`, `cancelRecovery`, `announce`,
  `timerSet(budgetId, delayMs)`, `timerClear(budgetId)`. The host executes them in order after receiving the outcome.
  `save` carries the session id and the session record words Pruv wrote into `out`; the host persists those words and
  never reads budget fields from `st`.
- `Query` is one of `runningDelegationIds(ref)` (halt), `isQuiet(sessionId)` (maybeClose),
  `releaseHeld(sessionId, budgetId)` (decide extend; a query because its boolean picks the wake fallback). The host
  first executes `effects`, then answers the query, then calls `resume`. `Resume` carries the pending sequence number
  Pruv stored in the state header; a `resume` whose number does not match is `InvalidInput`.
- A halt that happens inside another call (charge over limit, timer fire, decide re-halt after refresh) is the only
  nesting; it still costs one Need (`runningDelegationIds`). `decide` extend can need both a re-halt and
  `releaseHeld`; that call is allowed two Needs, in that order. No other call may exceed one.
- `Grow` is not a Need: it is not a host decision and is not counted against the Need budget. A call reserves its
  worst-case entry and arena words, derived from its input, before its first write, so a `Grow` outcome always leaves
  `st` exactly as it was and the retry starts from the same state. `Need` and `Done` never follow a partial write.
- Timers: Pruv keeps the deadline in its clock record and emits `timerSet`/`timerClear`. The host calls the public
  entry `fire(budgetId)` when a timer elapses. A stale delivery (no armed timer for that budget in Pruv state) is a
  no-op decided by Pruv.
- `unit()` and `findSession()` are answered from Pruv state, never by the host.

### 8.2 State

`BudgetState` holds the sessions' budget records (§2.1), clocks keyed by budget id (§2.2), and the per-turn
tool-activity sets, all inside the resident buffer: a fixed header (magic, version, total words, region bounds,
counts, pending-Need sequence), one open-addressing hash index over all keyed entries, an entry table, and an
append-only key/record arena. Every map keeps insertion order with links stored in its entries; nested maps (runs
of a clock, activities of a turn) are owned by their parent entry. Representation is chosen for the bounds in §9:
keyed-map operations are linear in key length, nothing scans or copies the whole state per call.

The host owns the buffer's memory and nothing else. It does not inspect, edit or interpret its words; it only
allocates a zeroed larger buffer on `Grow` (at least `max(N, 2·len)` words, `docs/RESIDENT_STATE_SPEC.md`), passes
the old one to `grow` and drops it afterwards. Pruv re-validates the header (O(1)) on every entry and treats every
index it reads as untrusted: all accesses are total, and an inconsistent header is `InvalidInput` with no write.
Removed entries go to a free list and keep their key/blob regions for reuse; removing an entry also frees its
children at once (e.g. `forgetTurn` frees its activities, O(children + key length)). Arena records outgrown by a
longer key or blob are reclaimed by the next `grow`.

### 8.3 Test adapter (not product)

`bud_test_abi.pruv` encodes `Call/Facts` and `Outcome` as flat `[i64]` words so the existing Rust/LLVM harness can
drive the four pairs. It has a magic/version header, length fields and a call code; nothing else. No per-step digest,
no ticket/generation machinery, no re-emission of earlier packets. The state is never part of the encoding: it stays
in the resident buffer between calls, and the adapter encodes only the call arguments, facts and the outcome, whose
size is linear in their own content and independent of S, C, R and A. Whole-state encoding (`observe`) is allowed
only for the final observation.

Performance gate per milestone (§9, AGENTS L3 rule 1): measure the TS class method in-process against the Pruv core
call on the resident buffer, excluding adapter encoding, at two or three sizes; report encoding time separately.
Per-call cost must stay flat as the state grows (amortized `grow` reported separately), and the core must not be
slower than TS at any measured size. If it is, find the cause before the next milestone. Decode errors return
`InvalidInput` without touching state.

### 8.4 Bounds and refusal

- At most 16,777,216 words in any encoded input or output; larger inputs are refused with `ResourceBound(input)`.
- Arithmetic is integral (§2.3); values outside the supported range are refused with `ResourceBound(number)` and
  classified excluded-domain, never rounded.

## 9. Complexity contract

Let:

- `S`: sessions.
- `C`: live clocks.
- `R`: runs in the addressed clock.
- `A`: distinct retained tool activity keys.
- `D`: returned running-delegation IDs.
- `N`: script/public calls.
- `U`: UTF-16 units actually read, validated, or formatted.
- `O`: serialized observation words.

Keyed-map operations are linear in key length in expectation with the open-addressing hash index (tombstones are cleared by an amortized in-place rehash); the worst case of a probe is the slot count. Fixed-sized field operations are constant, excluding key/string traversal and output.

| Flow | Bound, excluding output |
|---|---|
| Initialization / session ingestion | `O(S + U)` |
| current, isHalted, hasCard | `O(key length)` |
| attach / OPEN | `O(key length + U)` |
| replaceForUserMessage | `O(key length + U)` plus one opaque carryHeld effect |
| reopen | `O(key length)` |
| Successful/no-op charge | `O(key length)`; four capped counters |
| Rejected charge | halt bound |
| noteFailure | `O(key length)` |
| noteTool | `O(turnId + activityId + ref key lengths)`; no scan of A |
| forgetTurn | `O(turnId length + A of that turn)`: the turn and its activities are freed at once |
| runStarted/runWaiting/runEnded | `O(R + key lengths)` when REFRESH is reached |
| REFRESH | `O(R + key lengths)` |
| BEGIN/ARM/SETTLE/DROP/CLOSE | `O(key lengths + U)` absent synchronous FIRE |
| FIRE | `O(key lengths + D + U)` if halting; constant clock checks otherwise |
| halt | `O(D + key lengths + U)`; exactly D stopDelegation calls |
| maybeClose | `O(key lengths + U)` plus one opaque isQuiet query if reached |
| raise | `O(R + key lengths + U)` when refresh is needed; halt bound if immediate FIRE halts |
| decide extend | `O(R + key lengths + U)` plus reached FIRE/halt work |
| decide end | `O(key lengths + U)` |
| setCard | `O(U + inspected input elements)`; line-list length failures short-circuit |
| clear/record/stopAutoContinue | `O(key lengths + copied/output record size)` |
| bootRestore | `O(key lengths)` |
| forgetSession | `O(C + compared owner-string units)` |
| dispose | `O(C)` control work plus linear storage reclamation |
| Text helpers / budgetUnitOf | `O(U + output text length)` |
| Final observation | `O(S+C+R_total+A+O)` |
| grow (resident buffer) | `O(new buffer words)`; amortized `O(1)` words per reserved word because the buffer at least doubles |
| Header validation per entry | `O(1)` |

`runningDelegationIds`, `isQuiet`, queue operations, and attached-turn stopping are opaque dependency boundaries. Their real W implementations may scan application sessions; this piece claims no performance improvement for that external work.

For N calls, total core work must be linear in the sum of these per-call bounds, plus observation output and amortized storage work. In particular:

- No `N*S` copying merely to perform N charges.
- No `N*A` copying merely to deduplicate tools.
- No `D²` replay during halt.
- No full-history replay after each transport completion.
- No full-world snapshot after every public call unless explicitly requested.

### Performance measurements

Use deterministic counters and wall-time measurements:

| Axis | Small milestone sizes | Final sizes |
|---|---|---|
| Sessions, fixed addressed session | 100, 500, 1000 | 1000, 5000, 20000 |
| Repeated charge/tool calls | 100, 500, 1000 | 1000, 5000, 10000 |
| Runs per clock | 10, 50, 100 | 100, 500, 2000 |
| Delegation IDs on halt | 10, 50, 100 | 100, 1000, 5000 |
| Clocks for forget/dispose | 10, 50, 100 | 100, 1000, 5000 |

Measure one REFRESH at each prepared R to verify linear run scanning; do not mistake the sum of R successive starts for a single linear operation.

Report TS and Pruv time, peak RSS, encoded input/output sizes, map-node visits, run visits, dependency calls, and serialized output size. Separate initialization, core execution, and observation serialization. Counter growth must satisfy the stated bounds; timing is supporting evidence, not a substitute.

## 10. Fixed scenarios

Provide 14 groups of ten fixtures: 140 total. Each fixture may contain several calls to cover both a branch and its persistence consequences.

| Group | Fixtures 01–10 |
|---|---|
| A — attach/open | Missing budget; existing open; existing halted; existing closed with stale timer; configured default; defaultUnlimited; explicit size 1 overriding unlimited default; sizes 2/4/8; explicit unlimited; all origins and UUID/now/changed ordering. |
| B — replacement/reopen | Halted user replacement; halted schedule replacement; direct/release follows behavior; open reuse; absent/closed attach fallback; quiet reopen; user-end rejection; user-message rejection; stale/missing reference; replacement closes old object and invalidates old reference. |
| C — charging | Missing; closed; halted with reason; halted without halt; exact limit; first over-limit key; multi-counter atomic rejection including retries; unlimited; negative/nullish/zero amounts; failures/tools on all states. |
| D — tools | First activity; duplicate; different activities; same activity different turns; same turn across budgets; missing-ref consumption; stale-ref consumption; forgetTurn re-count; duplicate with no lookup; large turn set without session-wide scanning. |
| E — runs | First start; second run; duplicate start resets waiting; missing run; same waiting no-op; one active among waiting; all waiting settles; resume; end last run; halted/closed/stale-ref behavior. |
| F — timers | Exact deadline; early delivery rearm; late overshoot; backward clock; zero remaining synchronous fire; unlimited no timer; raise unlimited cancels without settling; stale captured callback; optional unref and oversized delay; since=0, owner mismatch, and early-fire run-count quirk. |
| G — halt/close | Missing session; stale budget; non-open/repeated halt; ordered/duplicate delegation IDs; active settle and snapshots; empty delegation list; quiet false; runs suppress quiet query; quiet close/no save; halt emitted from charge, fire and decide re-halt. |
| H — raise | Missing/closed; size1; numeric maximum; no numeric change; time-only increase while consuming; non-time increase; unlimited transition; already unlimited; numeric increase while unlimited; halted remains halted and activeSince preservation. |
| I — decide | Not found; not halted; default additive extend; size2/4/8; unlimited; reason plus release true; reason plus release false; missing reason with both release results; user-end; retained runs and immediate re-halt after refresh. |
| J — card validation | Missing/closed; deliverable type/empty/300/301; done counts and invalid line; targets missing/negative/fractional; raw note 0/300/301; verify count/label/status; conflicts count/empty/invalid; all state values and invalid state; multi-error precedence; old-card preservation and optional-error now call. |
| K — card representation | Trimmed success; astral 300-unit boundary; lone surrogate; ECMAScript trim versus U+0085/U+200B; empty optional arrays; empty note omitted; whitespace note retained; by projection and ignored supplied metadata; halted success and hasCard; Number.isInteger edge values including negative zero and large integers. |
| L — auto-continuation | Clear present/absent; record missing ref; record/delete on closed; pointed owner; stale fallback; existing pointed non-open blocks fallback; matching record stop; already stopped; working own-card synthetic record; other-owner/nonworking/no-card no-op. |
| M — restore/cleanup | Restore missing; restore running; restore halted; restore preserves usage; forgetSession matching clocks; forgetSession nonmatching clocks; dispose timers in order; dispose clears toolSeen; cleanup does not settle or edit persisted markers; two instances and repeated cleanup. |
| N — text/helpers | All counter names in ko; all in en; default language; minute floor boundaries; unlimited text versus numeric wire limit; refusal exact bytes; continuation exact bytes; cardMissing/errors exact bytes; budgetUnitOf absent/configured; helpers do not consume live elapsed time. |

Do not generate dependency exceptions (§0.5). Each fixture that touches a §15 KD row also records the `judge.py` expectation.

Every fixture states its source line/rule and expected calls or return. The oracle records the complete trace even when an expectation checks only a subset.

## 11. Q plan

Implement the oracle and Q generator before semantic Pruv modules.

Use deterministic xorshift32 with rejection-sampled choices, a recorded seed, and Fisher–Yates family ordering. Initial seed: `20261117`.

Eight families, 250 cases each:

| Family | Coverage |
|---|---|
| Q1 | Opening, replacement, reopen, stale references, origins, sizes |
| Q2 | Charges, atomicity, tools, failures, retries |
| Q3 | Multi-run waiting/running transitions and timer delivery |
| Q4 | Halt, quiet close, delegation order, nested halts |
| Q5 | Raise/decide combinations, retained runs, queue/release order |
| Q6 | Card validation, UTF-16, optional fields, Number classification |
| Q7 | Auto-continuation ownership, restoration, cleanup, multiple instances |
| Q8 | Text/helpers, admitted compatibility values, resource-bound inputs |

Generate sequences, not just isolated calls. Maintain both valid references and retained stale ones. Bias toward branch boundaries: equality to limits, zero time, 300/301 units, 0/1/5/6 list lengths, and missing-versus-undefined.

The oracle runs each selected corpus twice to check determinism. Malformed-input cases (§12) are driven separately through the test adapter.

Failure artifact:

```text
{
  seed, family, case,
  input, script,
  oracle, implementation,
  firstDifference,
  pair,
  boundOrForgeDescriptor?
}
```

Comparison classes:

- `exact`
- `P6`: named resource bound with approved prefix/state behavior
- `excluded-domain`
- `unexplained`

No engine-policy class is needed for this class-only piece. Fixed text requires exact equality; KD rows are normalized as stated in §15 and checked by `judge.py`. Crashes, missing lines, driver violations, and InternalInvariant are failures.

## 12. Malformed host input plan

The product core trusts its host the way the rest of Lair trusts the host process; there is no per-step
authentication to attack. What is tested is that malformed or stale input cannot make Pruv take a wrong decision:

| ID | Input | Required behaviour |
|---|---|---|
| F01 | Wrong magic/version or call code in the test adapter | `InvalidInput`, state unchanged |
| F02 | Truncated, overlong or inconsistent length fields | `InvalidInput`, state unchanged |
| F03 | String slot outside the text pool, lone surrogate where not admitted | `InvalidInput` |
| F04 | `resume` with a Resume that does not match the pending query kind | `InvalidInput`, no effect |
| F05 | Wrong answer type (non-boolean isQuiet/releaseHeld, non-string delegation id) | `InvalidInput`, no effect |
| F06 | `fire(budgetId)` for an unknown, closed or already-fired budget | Pruv no-op, no effect |
| F07 | `now` going backwards between calls | Settled time never negative (`max(0, …)`) |
| F08 | Duplicate UUID supplied for a second open | Pruv refuses with `InvalidInput` (budget ids must be unique) |

Plus eight honest controls: open, charge, tool duplicate, timer, halt, decide, card, restore.

Do not claim the core can reject a well-typed but factually wrong external answer (a wrong delegation list or quiet
flag). Those are external facts.

## 13. Mutant plan

Build mutants of the Pruv product, not of the oracle. Each changes one decision. A mutant is killed by an observable difference or a meaningful verifier rejection; syntax/build failure is not a kill.

| Mutant | Change | Required killer |
|---|---|---|
| M01 | Charge uses `>=` instead of `>` | C05 |
| M02 | Apply counters before completing all checks | C07 |
| M03 | Ignore unlimited during charging | C08 |
| M04 | Deduplicate tools by budget instead of turn | D05/D06 |
| M05 | Require all runs to be active for consumption | E06 |
| M06 | Rearm timer on every refresh | E02/E05 |
| M07 | Remove `max(0,elapsed)` | F04 |
| M08 | Early timer always halts | F02 |
| M09 | Raise unlimited settles time | F07/H10 |
| M10 | Reorder save/changed/announce or stop calls in halt | G04/G05 |
| M11 | Schedule replacement carries held/follows | B02 |
| M12 | Raise adds instead of taking maximum | H03/H04 |
| M13 | Decide uses maximum instead of addition | I03/I04 |
| M14 | Wake before release, or always wake afterward | I06/I07 |
| M15 | Validate card length by Unicode scalar count | K02/K03 |
| M16 | Stop auto-continuation overwrites another owner | L10 |

Other branches remain covered by fixed/Q cases even when they have no dedicated mutant.

Run each mutant only against its named killer and, if needed, its narrow Q family. Do not repeat the full four-pair corpus for every mutant.

## 14. Milestones

Each milestone is sized for approximately 45–60 minutes of one implementation worker. One hour is a stop-and-rescope boundary, not permission to report a partial bundle as complete.

All paths below are relative to `bench/l3/budget/`. File ownership is disjoint between milestones. Generated files under `work/` are not shared source ownership.

To keep ownership disjoint, the build script generates a temporary dispatch root from the implemented module list. The final committed `root.pruv` is added only at the integration milestone. Incomplete milestones use narrow generated test roots; unsupported methods are reported as not implemented, never counted as passes.

| Milestone | Owned files | Deliverable and end checks |
|---|---|---|
| M0 — executable oracle | `reference.mjs`, `reference-hooks.mjs`, `oracle-world.mjs`, `fakes/*`, `oracle.py` | Load the real pin; deps, clock, UUID, timers, trace. Run 300 deterministic oracle scenarios from a minimal in-memory seed stream; small 100/500-call oracle perf check. |
| M1 — corpus first | `q_gen.py`, `gen_fixtures.py`, `fixtures/*`, `common.py`, `SCENARIO.md`, `Q_README.md` | Generate all fixed groups and family definitions. Run 300 Q cases through the oracle twice; session/call-size oracle perf check. |
| M2 — comparison and runners | `compare.py`, `q.py`, `check_oracle.py`, `perf_budget.py`, `test_impl.py` | Full-observation comparison, stable diff artifacts, selectable pair/group/Q counts, perf counters. Run 300 oracle repeat/codec cases and 100/500/1000-call measurement. |
| M3 — core interface and test adapter | `bud_api.pruv`, `bud_test_abi.pruv`, `ABI.md` | `Call/Facts/Outcome/Need/Resume` types and the flat test encoding (§8). Run 300 generated encode/decode/rejection cases; encoded-size check. Not semantic Q passes. |
| M4 — state storage | `bud_store.pruv`, `bud_world.pruv`, `bud_map.pruv` | Core-owned maps on the resident buffer, insertion order, in-place edits, capacity reservation and `grow`, bounds. Run 300 state-operation Q cases; S=100/500/1000 and activity growth checks. |
| M5 — Rust mechanics | `host.rs`, `leaf.rs`, `harness.rs`, `host/Cargo.toml`, `host/src/lib.rs`, `dev_build.sh` | Storage, synchronous dispatch lane, effects, deterministic timers/IDs, JV observations. Run 300 adapter Q cases using generated effect programs; small timer/storage perf check. No budget decisions in these files. |
| M6 — text and number/date helpers | `bud_text.pruv`, `bud_num.pruv`, `bud_date.pruv` | Exact text, ISO time, supported integral arithmetic, raw card Number classification. Run 300 Q8/K numeric-text cases against Node; 100/500/1000 formatting checks. |
| M7 — lifecycle | `bud_lifecycle.pruv` | current, isHalted, OPEN, attach, replacement, reopen, CLOSE/maybeClose orchestration. Use clock sub-operation boundaries. Run 300 Q1 cases on implemented paths; session-size/attach perf check. |
| M8 — activity accounting | `bud_charge.pruv`, `bud_tools.pruv` | charge, noteFailure, noteTool, forgetTurn; halt is a semantic sub-operation. Run 300 Q2 cases, including isolated over-limit halt dispatch; charge/activity growth perf check. |
| M9 — clocks | `bud_clock.pruv` | Run methods, refresh/begin/arm/settle/fire/drop, explicit timer callback state. Run 300 Q3 cases; R=10/50/100 and timer-cycle perf check. |
| M10 — halt and decisions | `bud_halt.pruv`, `bud_decide.pruv` | Ordered halt effects, raise, decide, all nested clock flows. Run 300 Q4/Q5 cases; D=10/50/100 and retained-run perf check. |
| M11 — cards | `bud_card.pruv` | Full validation/projection, hasCard, language errors. Run 300 Q6 cases; raw text-length and repeated-card perf check. |
| M12 — auto and cleanup | `bud_auto.pruv`, `bud_cleanup.pruv` | Auto-continuation methods, bootRestore, forgetSession, dispose. Run 300 Q7 cases; C=10/50/100 cleanup and owner-lookup checks. |
| M13 — integration and malformed input | `root.pruv`, `pruv.json`, `judge.py`, `malformed_test.py` | Wire all modules; §12 malformed-input cases, KD judge, resource cases. Run 300 mixed Q cases plus controls; small S/R/D performance grid. |
| M14 — mutants and completion evidence | `q_mutants.py`, `RESULTS.md`, `IMPL_NOTES.md` | Kill 16 mutants; run the full fixed suite and 2,000 Q on all four pairs once; final defined perf grid. First run a 300-case mixed Q subset and small perf check to catch integration defects before the full pass. |

M0–M5 checks are explicitly oracle/apparatus/storage checks. They do not satisfy semantic implementation completion.

Semantic modules communicate through named Pruv sub-operation frames. The generated dispatch root resolves those names; the Rust host never implements a missing semantic operation. Milestone reports must list unimplemented dependent paths instead of substituting host decisions.

No compiler-wide test suite is required for this piece unless compiler/shared-library code actually changes. Shared modules are expected to remain unchanged.

## 15. Quirks and open questions

### 15.1 Classification of observed behaviour

Checked against B (line numbers are B's). "Keep" means Pruv implements it and Q compares it with Node. "KD" means a
known difference: Pruv does not copy it, the comparator normalizes it, and `judge.py` checks the Pruv side.

| # | Behaviour | Class | Decision |
|---:|---|---|---|
| 1 | OPEN notifies but does not save (222) | intended (comment) | keep |
| 2 | Replacement sets `follows` after OPEN's `changed` (195–197) | accidental (statement order) | KD-2: Pruv builds the whole new budget, then emits `changed` once |
| 3 | Schedule origin skips follows/carryHeld (188–196) | intended (comment, F4) | keep |
| 4 | Charge allows reaching the limit; over-limit rejection charges nothing (244–259) | intended (D1) | keep |
| 5 | Charge does not look at elapsed time | intended (time is enforced by the clock, D7) | keep |
| 6 | noteTool records the activity even when the ref has no current budget (270–276) | accidental | keep (dedup is per turn and harmless; cheaper than branching) |
| 7 | Failure/tool counters change closed budgets (264–276) | accidental | keep (a quietly closed budget can be reopened, D3; counts stay consistent) |
| 8 | runStarted creates a clock for a halted or closed current budget (286–289) | intended for halted (decide resumes time, 465), accidental for closed | keep both (closed clock is dropped on reopen/close paths) |
| 9 | `since = 0` counts as consuming (317) | not a quirk; JS truthiness avoided | Pruv uses `Option`; nothing to compare |
| 10 | settle checks clock ownership, other clock ops do not (363) | accidental; mismatch unreachable (budget ids are unique UUIDs and `current` checks the session) | excluded-domain |
| 11 | Early fire re-arms on `runs.size > 0`, not on "some run not waiting" (352) | accidental; equivalent in reachable states (a timer only exists while consuming) | Pruv uses the consuming predicate; Q has no reachable difference |
| 12 | Unlimited keeps usage and numeric limits (205, 218) | intended (comment: display) | keep |
| 13 | Numeric raise changes limits of an unlimited budget (431–437) | accidental | keep (display only, harmless) |
| 14 | raise never resumes a halted budget (411) | intended (decide is the human path) | keep |
| 15 | decide extend adds to all five limits (448, 459) | intended (comment) | keep |
| 15b | decide saves before refresh, so `activeSince` is not in that save (463–466) | accidental; harmless (bootRestore drops activeSince, D8) | keep order |
| 16 | Halt without reason suppresses the continuation text (455, 468) | defensive; unreachable in domain (halt always sets a reason) | excluded-domain |
| 17 | `queueContinue(null, true)` = wake only (36, 469) | intended (interface doc) | keep |
| 18 | setCard reads `now` before validating optional fields (488) | accidental | KD-1 covers it (one `now` per call) |
| 19 | Card text length is UTF-16 units after trim; `targets.note` is measured raw (146, 492) | accidental (JS string length; note path forgot trim) | keep UTF-16 after trim for the trimmed fields; note untrimmed kept; recorded for the L3 restructure bundle |
| 20 | Empty note omitted, whitespace-only note kept (494) | accidental | keep |
| 21 | A new card replaces optional fields rather than merging (488–510) | intended (full card per call) | keep |
| 22 | recordAutoContinue enforces no count maximum (527) | intended (caller policy) | keep |
| 23 | A found, non-open pointed budget blocks the fallback to the session's own budget (540–543) | intended ("look at ref first; only if absent, own") | keep |
| 24 | bootRestore discards the running interval without charging (555) | intended (D8, under-count) | keep |
| 25 | forgetSession/dispose leave persisted `activeSince` (563–571) | intended (cleanup of memory only; next boot clears it) | keep |
| 26 | A throwing dependency leaves a partial prefix of mutations/effects | accidental (no rollback layer) | excluded-domain (§0.5) |
| 27 | charge on a halted budget without `halt` returns reason `'delegations'` (251) | defensive; unreachable | excluded-domain |
| 28 | Several `now()` reads inside one call can differ (e.g. halt: settle, then `halt.at`) | accidental | KD-1: one `now` per public call |

Known differences in one place:

- **KD-1** One `now` per public call. Normalization: the comparator compares ISO timestamps and settled `activeMs`
  with the oracle driven by a clock that returns the same value for every read inside one public call. The oracle
  fake clock already does this, so no normalization is needed in Q; `judge.py` checks it on the Pruv side.
- **KD-2** `follows` is present in the record at the single `changed` of a replacement. The comparator ignores the
  position of `changed` relative to the `follows` write (it compares the final record and the effect list without
  the extra snapshot at `changed`).

Bugs found: none in B itself. One wiring observation outside this piece (owner's own held items are left alone on
replacement, W:186) is documented as intended there (F05).

### 15.2 Open implementation questions

These do not change the required normal-domain semantics:

- **Arithmetic outside the integral domain:** determine whether a later extension needs arbitrary finite fractional budget values. The first implementation must report these as excluded-domain, not silently round them.
- **Reentrant dependencies:** W can invoke service code whose broader reentrancy behavior is not established by this read-only class specification. Initial fakes are non-reentrant. Do not claim proof of live service reentrancy.
- **Storage implementation:** confirm the hash-map/storage operations fit the current Pruv verifier within each one-hour slice. A replacement data structure is acceptable only if it preserves the stated bounds and host split.
- **Native timer extremes:** the deterministic scheduler’s explicit integer-delay policy must be checked against the Node version used by the oracle. Do not rely on real wall-clock waits.
- **Alias exposure:** current and setCard preserve identity, but arbitrary external mutation through retained returned aliases is outside the initial scenario language. A later alias-mutation extension must model it in Pruv rather than let Rust edit budgets directly.

## 16. Completion criteria and verification status

The implementation is complete only when:

1. All public methods and exported helpers above run in Pruv.
2. Host code contains no budget, card, quiet-close, continuation, or limit decisions.
3. Both compilers verify the module graph and all four build pairs execute it.
4. All 140 fixed fixtures and 2,000 Q cases are classified on all four pairs with zero unexplained differences.
5. All §12 malformed-input cases behave as stated; honest controls pass; `judge.py` passes on every KD row.
6. Resource refusals identify the exact bound and preserve the specified prefix/state.
7. All 16 mutants are killed or meaningfully verifier-rejected.
8. Performance counters and measurements satisfy §9.
9. RESULTS records commands, pair names, corpus seeds, classifications, exclusions, and actual counts.

Confirmed in this specification task:

- The complete 573-line class source was read.
- Shared budget types/constants and the language picker were inspected.
- Actual dependency bindings and relevant W implementations were inspected.
- The previous req specification, ABI, implementation notes, scenario/Q documentation, root/host/leaf structure, and shared-library guidance were inspected.
- No repository files were changed.

Not runtime-verified in this task:

- The proposed core interface, Pruv proofs, oracle implementation, compiler outputs, fixtures, Q, forgeries, mutants, or performance.
- Live Lair service behavior, external repositories, or files outside `/Users/spica4345/Projects/pruv`.

The milestone durations are planning estimates, not measured implementation times.