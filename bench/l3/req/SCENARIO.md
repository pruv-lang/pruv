# L3-9 scenario and observation format

Fixture file: `fixtures/<G>.json` (groups S F B L A O T U G R E C D I P X, 10 each, IDs `S01`..`X10`), written by
`gen_fixtures.py`. A fixture is `{id, name, rule, expect, scenario, layer?, killer?, note?}`. The oracle ignores
`name rule expect layer killer note`; only `scenario` (plus `id`) is run.

## Scenario
`{id, world, instances?: ['A',...] (default ['A']), script}`.

World fields: dataDir, env, defaultAccount, clockStart, uuids (list; entry may be `{throw}`; default
`00000000-0000-4000-8000-00000000000N`), clock `{throw}`, sessions, accounts `{engine:{ok:[rows]}|{throw}}`, ambient `{ok}`,
readers.<engine>.<list|read> (queue; n-th call takes entry n, last repeats; entry `{ok|reject|defer:label|onAbort:ERR|sync_throw}`;
n-th use of a label is `label`, `label#2`, ...), copy, loadCheck, locateAcp, makeCodexRunDeps, tempRoot, grokHomeFor, rm,
grokAccounts, isTemporary, checkFolder, modelFor, createImported (queue `{mode: ok|fail-before-fill|fail-after-fill, id, session, throw}`),
sessionList (queue; null = normal; `{throw, throwAt?}`), fromAppAction, newSession.
ERR spec: `{class: Error|TypeError|EngineImportError|plain|primitive, ref, code?, message?, name?, value?}`; all ERRs are pre-built
into `world.errs` (gen_fixtures `F()` collects them).

Script steps: `start {req,i?,op,input?,remote?,tempVisible?,caller?,cancel?,fromAppAction?}`, `settle {label,ok?,reject?}`,
`advance {ms}`, `cancel {handle,reason?}`, `handle {name}`, `mutate {op:add|remove|replace, session|id}`, `quiesce`.

## Observation
`requests[{id,terminal,value}]`, `trace`, `createdSessions[{id,seedKeys,seed,before,after,outcome}]`, `finalOpening`,
`openingEvents`, `activeTimers`, `activeListeners`, `driver`.
Terminals: ok | wire | thrown | eie | native | pending. Thrown values: `{ref}`, `{cancelReason:handle[,ref]}`, `{domAbort:1}`, `{wire:{key,message}}`.
Trace record `{c, r, i, a?, o?}` (c = reader, locateAcp, makeCodexRunDeps, tempRoot, grokHomeFor, loadCheck, rm, copy, copy.verify,
uuid, now, grokAccounts, sessionList*, isTemporary, checkFolder, modelFor, createImported, fromAppAction, timer.set/fire/clear,
listener.add/remove, abort, settle, advance, opening.has/add/delete, prim, mutate). Signals are `sigN` by first sight, or the handle name.
JV codec: plain strings are plain; lone surrogates `{u16:[...]}`; objects `{obj:[[k,v]]}`; `{arr}`, `{undef:1}`, `{ref}`, `{num}`.

## Expectation DSL (check_oracle.py)
Per request `req`: terminal key message ref reason noReason code value(subset) keys notContains itemKeys itemCount.
Global: counts, matchCounts [[spec,n]], has, absent, order, noneBetween, created, createdAfter/Before/Meta, createdKeys,
createdLen, opening, openingOps (`op:key:true|false`), timers, listeners. Subset matches are length-checked for lists and
type-strict for scalars; observed `{ref:x}` compares as the string `@ref:x`.

## Quirks
- Cancel via handle records `abort` on the handle signal with reason `{cancelReason:h}`; the internal read signal gets the same reason
  (`domAbort` only when the cancel has no reason).
- The 20,000 ms timer only aborts; a reader that ignores abort still completes (C04).
- An EngineImportError with an unmapped code is rethrown as-is (E09); an AbortError-named error of any class maps to wire timeout (X06).
