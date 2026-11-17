# Engine Import Request Handlers — L3-9

Status: design specification for implementation. This is not a report of finished or verified work.

**Summary.** Move the request decisions, result construction, bound-session search, timeout/cancel
composition, concurrent-open guard and Grok-copy cleanup ownership of `engine-import-sources`,
`engine-import-list` and `engine-import-open` into Pruv. Reuse the L3-8 source, common-read and bound
helpers. The host only performs external observations and side effects. The comparison covers not only
final values but errors, call order, created sessions, timer/cancel lifetimes, and the shared `opening`
state across concurrent requests.

The pin is Lair `17001ab3` (not L3-8's `744d14af`). The Codex-unsupported Korean message uses the new
pin's wording ("버전").

Evidence: `bench/l3/req/reference/backend-requests-engine-import.ts:54–201`; `RESULTS.md:2292–2308`;
`bench/l3/req/reference/README.md:3–17`.

## 1. Evidence aliases

All paths are relative to the Pruv repository. `R:12` means line 12 of file R.

| Alias | File |
|---|---|
| R | `bench/l3/req/reference/backend-requests-engine-import.ts` |
| IN | `bench/l3/req/reference/backend-inputs-engine-import.ts` |
| SI | `bench/l3/req/reference/shared-engine-import.ts` |
| ET | `bench/l3/req/reference/shared-engine-thread.ts` |
| SC | `bench/l3/req/reference/shared-contracts.ts` |
| REC | `bench/l3/req/reference/logic-records.ts` |
| M | `bench/l3/req/reference/backend-migrate-work-run.ts` |
| C9 | `bench/l3/req/reference/backend-codex-home.ts` |
| E | `bench/l3/entry/reference/backend-engine-import.ts` |
| B | `bench/l3/entry/reference/backend-engine-import-bound.ts` |
| T | `bench/l3/entry/reference/backend-engine-import-tail.ts` |
| CD | `bench/l3/entry/reference/backend-engine-import-codex.ts` |
| G | `bench/l3/grok/reference/grok.ts` |

"Design decision" marks a requirement imposed on the new implementation, not existing behaviour.
The spec was written from reading files only; nothing was compiled or run. Real Lair, live accounts,
external engines, Windows and network services are out of scope.

## 2. Scope and ownership

### 2.1 Decisions owned by Pruv

| Area | Pruv responsibility |
|---|---|
| Sources | build sources per request, first-match lookup, `sourceUnknown`, public projection |
| Folder | where `checkFolder` is called; wrapping its throw as the folder WireError |
| Bound | ordered search over a fresh `sessionList()` per call, stop at first match |
| Account | Grok target account choice and check; source account for other engines |
| Mode | when `same` is allowed; the `fromAppAction()` refusal |
| Concurrency | per-handler-instance `opening` set, existing-bound first, busy, release |
| Time/cancel | `timed` start/end, 20 s abort, external cancel link, cancel-reason precedence |
| Errors | `importFailure` order, keys, messages, passthrough |
| List | titles map, bound title choice, field inclusion rules |
| Open | read → Grok prepare/copy/verify → cancel check → race re-search → create |
| Created data | Turn, EngineThread, ExternalThread, title, threadSlot, fill recipe |
| Cleanup | ownership of the returned Grok copy and the `rm` request on race/failure |

Evidence: R:54–201; REC:7–9.

### 2.2 Calls left to the host

`checkFolder`, `modelFor`, `createImported`, applying the fill recipe to a session and saving it,
`randomUUID`, `Date.toISOString`, `rm`, `tempRoot`, `grokHomeFor`, `locateAcp`, `makeCodexRunDeps`,
account/session observations, engine readers and Grok copy/load execution, clock/timer/AbortSignal.
Pruv decides which call to make, its arguments, and how to interpret failures.

Pruv builds the **contents** of `fill`. The host must not re-implement title choice, turn conversion or
slot choice; it applies the four fields mechanically and preserves the other `engineThreads` slots
(R:188–194).

The Grok file allow-list, safe copy, rename and the copy function's internal cleanup stay at the L3-6
boundary. L3-9 owns the timeout of the copy call and its verify callback, and cleanup of a copy returned
as success (G:182–229; R:163–173,197–201).

### 2.3 Excluded

Valibot input checks, the request gate's local/remote permission, folder permission policy, model
selection policy, account auth/prepare internals, the session store, migration ledger/undo. The target is
the handler that receives validated requests; `_remote`, `_tempVisible`, `_caller` add no new policy
(IN:8–25; R:3,40–51,118–119,145).

## 3. Exact request semantics

### 3.1 Sources and the public list

`sources` keeps L3-8 `importSources` order and duplicates. Each item has `source`, `engine`, `account`
only when truthy, and `label`. `home`, `kind`, `accountName` are not exposed. `label` is
`accountName ?? ''` for account sources, otherwise `''` (R:123–127; E:34–53).

`list` and `open` build the whole `importSources(deps)` every time and take the first
`source === request.source`; none → `sourceUnknown`. A throw while building sources is not turned into
`sourceUnknown`, and `checkFolder` is not called after it (R:80–84,129–130,146–147; E:56–59).

### 3.2 Folder check

After source lookup, call `checkFolder(request.folder)` once and use the returned real folder for all
later work. If the throw is an `Error`, its `message` (including `''`) is the folder WireError message;
otherwise `이 폴더는 쓸 수 없어.` (R:86–89).

This check is outside the list `try` and the open `try`, so `importFailure` and the open catch's
cancel precedence do not apply to it (R:128–162,197–201).

### 3.3 `boundSessionId`

Each call takes a fresh `deps.sessionList()`. In session order:

1. `temporary === true` → excluded without calling `isTemporary`.
2. Otherwise call `isTemporary(session.id)`; truthy → excluded.
3. Walk `sessionThreads(session)` candidates in order; if engine and thread are both strictly equal,
   return `session.id` at once.

Candidate order: external original thread, external forks, truthy terminal cliSession, engineThreads in
Object.entries order. Terminal engine defaults `?? 'codex'`; a slot engine is the part before the first
`:` (R:96–102; B:15–32).

Do not replace this with L3-8 `boundSessionIndex`. Extra `isTemporary` calls after the first match, or
reusing one snapshot across items, change the call semantics. With an empty session ID the search stops
but the caller's `if (known)`/`if (bound)` is false (R:97–100,136–141,155–158,170–174; B:35–44).

### 3.4 Target account and `same`

Non-Grok engines use `source.account` and ignore `targetAccount`. For Grok, a falsy target or one equal
to `DEFAULT_ACCOUNT` gives an undefined account. Otherwise call `grokAccounts().some(id === target)`;
none → `accountUnknown` (R:104–110).

`DEFAULT_ACCOUNT` is defined outside this pin's manifest. Design decision: it is an immutable host
binding given identically to both sides; do not guess the literal (R:18,107).

`same` first refuses Grok or non-home sources with `sameRefused`. Only if that passes is
`fromAppAction()` called; true → `sameByAi`. `fork` makes no such call. The account check and the
existing-bound search come after these refusals (R:150–156).

### 3.5 List

1. Source lookup, real folder check.
2. Inside `timed`, await `readOptions`, then L3-8 `listExternal`.
3. After it completes, one `sessionList()` builds the `titles` map.
4. A separate `boundSessionId` per item.
5. Build new objects in the original item order.

The titles map is **last-write-wins** per session ID and does not exclude temporary sessions (unlike the
bound search). Title: if bound is truthy, `titles.get(bound) || item.title`, else `item.title`. A last
title of `''` falls back to the item title. No 80/200 limit is added to the replaced Lair title
(R:128–143; E:80–81).

| Field | Included when |
|---|---|
| `thread`, `title` | always |
| `updatedAt`, `size`, `messages`, `path` | value is not undefined |
| `boundSessionId` | bound is truthy |
| `maybeOpen` | input is truthy; value `true` |
| `entrypoint`, `origin`, other engine fields | never |

`0`, `''` and explicitly tested `null` must not be dropped by a truthiness test (R:139–141; T:66–78).

### 3.6 Open order and `opening`

Order: source → folder → same refusals → account → initial bound search. A truthy existing bound returns
`{sessionId, existing:true}` before the opening check and before any cancel check (R:145–156).

The key is exactly `${engine}:${request.thread}`; folder, source, account and mode are not part of it. If
the same handler instance's set has the key → busy; otherwise add it. Only a request that added the key
deletes it in its own finally (R:119–121,157–161,201).

### 3.7 `readOptions`

| Case | Calls and shape |
|---|---|
| every read | `{signal, env:deps.env}` |
| Codex list/open | `await makeCodexRunDeps(source, deps.dataDir)` → `codex` |
| OpenCode list | no `locateAcp`, no `tempRoot` |
| OpenCode open | first `await locateAcp('opencode')`; only if binary is truthy, `await tempRoot()` and the `opencode` field |
| other engines | nothing extra |

A `locateAcp` throw is not swallowed; distinguish a missing binary from a failing call (R:112–116,132,162).

### 3.8 Timeout and cancel

`timed` makes a new AbortController. If the external cancel is already aborted it aborts the new
controller at once; otherwise it adds a once abort listener. It registers a 20,000 ms timer and calls the
work. On success and failure the finally clears the timer and removes the listener (R:71–78; T:30–32).

**The timeout is not a racing Promise that forces a reject.** The timer only aborts the controller. If
the work ignores abort and succeeds, `timed` succeeds. Open has no separate deadline-failure check after
a successful read (R:73–77,162–170).

The read's 20 s includes waiting for `readOptions`. The Grok verify callback gets its own 20 s timer.
`grokHomeFor` and the whole copy are not wrapped in one request deadline. List passes no external cancel
(R:132,162–166).

Forwarding external cancel to the new controller calls `controller.abort()` without the external reason.
Open's pre-save check and its catch throw the **original** `cancel.reason`. Preserve this down to error
identity (R:74–75,169,199).

An already-cancelled request can still return the initial existing bound. If cancelled while waiting for
the race-path `rm`, there is no re-check after `rm`, so it returns the existing session. In the catch
path, cancel is checked again **after** cleanup and its reason wins (R:155–156,169–174,197–200).

### 3.9 Grok copy and rollback

After a successful Grok read, call `grokHomeFor(account)`. Its rejection becomes
`EngineImportError('engine-missing')` with detail = message if Error, else undefined, and the original
cause. The outer catch decides final precedence with cancel (R:163–166,197–200).

`copyGrokSession` gets the source, real folder, request thread, target.home, a verify callback, and the
original cancel. The verify callback uses the home given by copy and runs `grokLoadCheck` with
`{binary:target.binary, env:target.env}` under its own `timed`. `copied` stores `.path` **only after copy
returns success** (R:164–166).

| Failure/exit point | Cleanup owner |
|---|---|
| read or target prepare fails | no returned copy; no handler rm |
| file copy inside copy fails | copy cleans its temp |
| verify fails | copy cleans its target; handler `copied` still empty |
| after copy success: cancel, race lookup, clock, UUID, conversion, modelFor, createImported fails | handler rm of truthy copied |
| after copy success: race found | rm, then return the existing session |
| createImported returns normally | copied set to undefined; the copy stays |

Every handler rm uses `{recursive:true, force:true}` and ignores rejection. The handler never guesses a
target path before copy fails (R:160–200; G:201–229). "Rollback on every failure" means a cleanup
**attempt**; rm failures are swallowed. No code undoes a partial save inside createImported.

### 3.10 Race re-check

After the read and any Grok copy, check external cancel first, then call `boundSessionId` again. A truthy
result cleans only its own copy and returns `{sessionId:raced, existing:true}`; no clock, UUID, model or
create calls happen on this path (R:168–175).

### 3.11 Turn and bound data

Without a race: one clock read, then one UUID per `external.turns` entry in order.

| Turn field | Value |
|---|---|
| `id` | that UUID |
| `prompt`, `reply` | original |
| `progress` | `''` |
| `status` | `'completed'` |
| `usage` | `null` |
| `createdAt` | `item.createdAt ?? now` |
| `completedAt` | only when not undefined |
| `imported.engine` | source.engine |
| `imported.tools` | original number only when `item.tools > 0` |
| `imported.clipped` | `true` when truthy |

An empty createdAt is kept; only nullish becomes now (R:175–180; T:80–95; SC:299–333).

EngineThread: `threadId=request.thread`, `lastTurnId` when the last Turn ID is truthy, `fork:true` for
non-Grok fork. Empty turns are allowed (no UUIDs, no lastTurnId) (R:176–182; ET:6–22).

ExternalThread: `engine, source, thread, mode, importedAt`; `omittedBefore:true` when truthy; `lastPlan`
as-is when truthy. Forks are not created and lastPlan is not re-normalized (R:183–185; SI:13–27).

### 3.12 Title and seed

```text
candidate = external.title || external.turns[0]?.prompt || ''
title = collapse_ecmascript_whitespace(candidate).trim().slice_utf16(0, 80)
title = title || basename(folder) || folder
```

A whitespace-only external.title is chosen first as truthy, so even if it becomes empty it does not fall
back to the first prompt. The folder fallback is not collapsed or cut (R:186–187).

The whitespace set is ECMAScript `\s`: reuse `u16::is_trim_unit` (U+0009–000D, 0020, 00A0, 1680,
2000–200A, 2028, 2029, 202F, 205F, 3000, FEFF). U+0085 and U+200B are not whitespace. A cut at unit 80
that splits a surrogate pair keeps the high surrogate (R:187; `bench/l3/entry/u16.pruv:148–173`).

`modelFor(engine, account, external.model)` is evaluated after turn and title construction and before
`createImported`. The seed has engine, account only when truthy, model, real folder, fill. Fill sets
title, turns, externalThread, and spreads the existing engineThreads then overwrites one
`threadSlot(engine, account)`. Normal result is `{sessionId}` without `existing:false`
(R:188–196; REC:7–9).

## 4. Error contract

### 4.1 `importFailure`

First check `error?.name === 'AbortError'`, regardless of Error-ness and before any EngineImportError
code check. Then check the code of a real EngineImportError instance; a plain object with a `code` field
is not one (R:54–68).

All keys are prefixed `wireWork.engineImport.`.

| Input | Key suffix | Exact message |
|---|---|---|
| name `AbortError` or EIE `timeout` | `timeout` | 터미널 대화를 읽는 데 시간이 너무 오래 걸렸어. |
| EIE `not-found` | `notFound` | 터미널 대화를 찾을 수 없어. |
| EIE `codex-unsupported` | `codexUnsupported` | 이 Codex 버전은 바깥 대화 읽기를 지원하지 않아. |
| EIE `grok-exists` | `grokExists` | Lair 의 Grok 폴더에 같은 번호의 대화가 이미 있어. |
| EIE `grok-load-failed` | `grokLoadFailed` | 복사한 Grok 대화를 열지 못했어. |
| EIE `engine-missing` | `engineMissing` | 이 엔진을 찾을 수 없어. |
| EIE `read-failed` | `readFailed` | 터미널 대화 기록을 읽지 못했어. |

Any other value or unknown EIE code is rethrown as-is. EIE detail is never appended to these messages.

### 4.2 WireErrors raised directly

| Key suffix | Message |
|---|---|
| `sourceUnknown` | 고를 수 없는 출처야. |
| `folder` | §3.2 Error.message or fallback |
| `accountUnknown` | 고른 Grok 계정을 찾을 수 없어. |
| `sameRefused` | 원래 대화에 그대로 이어 쓰기는 기본 Claude·Codex·OpenCode·Qwen 에서만 돼. |
| `sameByAi` | 그대로 이어 쓰기는 사람이 확인해야 해서 AI 는 쓸 수 없어. |
| `busy` | 같은 터미널 대화를 이미 여는 중이야. |

The L3-8 `listExternal/readExternal` common rethrow applies first, then the request mapping. A plain
Error from an engine reader becomes read-failed in entry, but a plain Error thrown directly by
`readOptions` does not pass the entry wrapper and may stay as-is (E:64–67,73–94; R:113–115,132,162).

## 5. L3-8 reuse and the state machine

### 5.1 Structure

**Design decision:** one parent Pruv state machine runs the whole request and calls the L3-8
`entry_begin/entry_step` as sub-computations. A Rust host that composes source lookup, list/open
branching or bound decisions is forbidden. Do not mix new request ops into L3-8's op 1–9 numbering
(`bench/l3/entry/root.pruv:20–77`; `state.pruv:5–28`; `sources.pruv:3–8,244–317`).

Shared entry modules move to `bench/l3/lib/` per the repository rule, with no copies. The body of
`root.pruv`'s API moves to `entry_api.pruv` and the entry root forwards the same public API. Behaviour
of `u16`, `packet`, `value`, `state`, `dispatch`, `bound`, `sources`, `canon`, `accounts`, `path` stays;
new request modules use a `req_` prefix (`AGENTS.md:25–32`).

**Implementation decision (name collision):** `mcp`, `supervisor` and `opencode` are compiled with
`--lib bench/l3/lib` and keep their own `packet.pruv`/`path.pruv`; a library file with the same name is
a compile error. The moved modules are therefore renamed with an `ent_` prefix (`ent_u16`, `ent_packet`,
`ent_value`, `ent_state`, `ent_dispatch`, `ent_bound`, `ent_sources`, `ent_canon`, `ent_accounts`,
`ent_path`) and every `crate::X::` reference is renamed mechanically. Only names change.

| Request work | Reuse |
|---|---|
| importSources | op 1, collect each step's source rows in order |
| source choice | op 2, first-match index |
| list/read wrapper | op 3/4 |
| session exclusion | op 8, one session at a time |
| candidates | op 7 |
| boundSessionId | new first-match loop in the request layer |
| title whitespace, basename | existing u16/path pure functions |
| whole bound index op 9 | not used by the handler search |

The L3-8 read result is an opaque record ref. L3-9 gets a typed ExternalRecord view tied to that ref from
the host and processes its fields itself. The list keep/copy-title recipe keeps existing row refs and is
interpreted as projection input by the request layer. Title or turn decisions are not handed to a host
renderer (`dispatch.pruv:112–157,171–184`; `entry/host.rs:1564–1603`).

### 5.2 Request ABI and state

**Design decision:** a separate request envelope version 1 with request ops `1=sources, 2=list,
3=open`. The entry envelope is unchanged; distinct magic/API separate the op namespaces.

```text
req_begin(world, input) -> world', Await | Done
req_step(world, previous, completion) -> world', Await | Done
```

`world` holds the per-handler-instance opening set and request ownership. Request state needs:
instance ID, request ID, op, phase, global await seq; original input, chosen source, real folder,
account; child entry op, call number, child packet, child seq, collected rows; session iterator, search
position, list item position, titles map; timer/signal/listener IDs and liveness; opening-key ownership;
copy call and verify callback progress and the returned copied path; read result, turns being built,
now, seed/fill recipe; current item replay log and all outstanding requests. This extends the existing
magic/op/request/checkpoint/blob/log layout; no new meaning is smuggled into the 8-word checkpoint.

### 5.3 Await numbering and execution boundaries

Every external completion carries a monotonic `seq`. Child entry calls keep
`(requestId, childCallId, childSeq)` mapped to the parent seq. Completed calls consumed during replay are
not re-run.

**Design decision:** separate transport awaits from real JavaScript async suspension. Transport round
trips for synchronous calls (`checkFolder`, `sessionList`, `isTemporary`, UUID, modelFor) are not
scheduling points. Only real Promise awaits let the script advance other requests, timers or cancel.

| Site | Meaning |
|---|---|
| A01 | readOptions `locateAcp` |
| A02 | readOptions `makeCodexRunDeps` |
| A03 | readOptions `tempRoot` |
| A04 | list callback `await readOptions` |
| A05 | open callback `await readOptions` |
| A06 | entry list/read engine Promise |
| A07 | timed `await work` and finally |
| A08 | list/open receiving the timed result |
| A09 | `grokHomeFor(...).catch(...)` |
| A10 | copy progress and verify callback entry/return |
| A11 | load check inside verify timed |
| A12 | race cleanup rm |
| A13 | catch cleanup rm |

An await on an already-resolved Promise is not merged into a synchronous call. A04/A05 exist even when
readOptions has no conditional await. Repeats at one site get an occurrence number.

### 5.4 Copy callback

When copy asks for verify, the host bridge sends `VerifyRequested(copyToken, home)` to the parent Pruv
machine. The parent runs a new timed/load-check, returns the callback result, and copy then completes
with success or failure. Distinguish the suspended copy call from the current outstanding leaf call. A
host fake must not skip verify or apply its own timeout. Until copy succeeds, the parent's `copied` is
empty (R:166; G:222–229).

### 5.5 Replay defence and side effects

Replay state must match not just structurally but exactly the outstanding call regenerated from the
current request: log op/seq continuity, full consumption, checkpoint phase/index/count, child ownership.
Keep entry's `item_start + dropped + kept == current_seq`.

The driver consumes the latest continuation once and runs each external effect once per
`(instance, request, seq)`. Re-submitting a whole consistent past state cannot be detected by pure
functions alone, so latest-state/generation tracking is the trusted host's job. Running a new rm,
create, UUID or opening effect before rejecting a forged state is a failure
(`bench/l3/entry/DESIGN.md:161–186`).

### 5.6 L3-8 regression guard

The move and API extraction change no semantics: fixtures, oracle data, op numbers, packet version and
result policy stay. Run the affected L3-8 fixtures on the four pairs and the existing forged controls. No
full compiler suite or new audit for the move.

## 6. Observations and concurrency

### 6.1 Result format

```text
{ id, requests: [{id, terminal, value}], trace, createdSessions, openingEvents,
  finalOpening, activeTimers, activeListeners, driver }
```

`terminal` distinguishes success, WireError, original thrown value, internal boundary error. Thrown
values keep undefined/null/primitive/object ref. WireError compares key, message and kind; stacks are not
compared. Strings are UTF-16 units; absent and present-undefined fields differ. UUIDs and times come
from the script and stay. Only opaque refs are normalized by first appearance, keeping aliasing.

### 6.2 Trace

Records call order, arguments, return/throw. It must include: L3-8 accounts/env/homedir/cwd/realpath/
stat/engine calls; `checkFolder`, `fromAppAction`, `grokAccounts`; every `sessionList()` call and how far
it was consumed, `isTemporary`; Codex/OpenCode prepare calls and the identity of signal and env in the
options; timer register/fire/clear, listener add/remove, abort; Grok target/copy/verify/load/handler rm;
now, UUID, modelFor, createImported and the session before/after fill; per-request opening add/delete and
busy results.

Each `sessionList` call gets an independent script iterator; record consumption and early stop to show
no needless iteration after a first match. Arbitrary user iterator side effects are excluded; session
changes defined by the fixture scheduler are supported.

### 6.3 Deterministic scheduler

**Design decision:** no real sleeps; a logical clock, deferred Promises, explicit settle and cancel
events. One scenario shares one handler instance by default. Synchronous stretches (source, folder,
isTemporary) run to completion.

| Scenario | Schedule and expectation |
|---|---|
| same-key busy | A takes the key, waits on read → B starts → B busy → A finishes → key gone |
| known-before-busy | while A waits, an external session binds → B starts → B gets existing, not busy |
| bind-during-read | A waits on read → session change → read done → race existing, no create |
| Grok race | through copy success → bind added → one race rm → existing |
| different keys | two requests interleave read waits/completions; each frees only its key |
| same engine/thread, other folder/source/account | same key conflicts within one instance |
| instance isolation | separate handler instances handle the same key independently |
| cancel during cleanup | cancel during race rm → existing; cancel during catch rm → reason wins |

Do not edit the oracle body to see the opening set. A test-only Set instrument may delegate and record
add/delete/has for tracked requests while keeping native Set semantics; also compare busy and retry
results so the check does not rest on the instrument alone.

## 7. Domain and difference classes

### 7.1 Supported domain

Input is a validated request: source length 1–80; thread 1–200 matching `/^[\w.-]+$/`; mode fork/same.
absolutePathSchema and accountSchema internals are not in this pin; do not re-implement or guess them
(IN:4,8–25).

Dependency data is plain objects and dense arrays with SI/T field types. Explicit compatibility fixtures
allow undefined/null/empty strings, an empty session ID, falsy temporary and null metadata. Excluded:
non-string title/prompt, proxies/getters, arbitrary user iterators, cyclic objects, monkey-patched
built-ins. Unicode is not an exclusion: lone surrogates, combining marks and astral scalars in titles,
IDs and messages are kept as UTF-16. OS paths stay within the existing POSIX adapter; no Windows claim.

### 7.2 Resource bounds

**Design decision:**

- Each input/completion/state/output packet: at most 16,777,216 i64 words (as before).
- At most 32 active requests per handler instance.
- At most 1,000,000 new external-call seqs per scenario.
- Over a bound: explicit `CoreError/InputTooLarge`; never a silent cut, success or dropped account.
- Opening ownership, copied path and timer info needed for cleanup are secured before bulk results; a
  resource rejection still performs cleanup of effects already taken.
- Check sizes before extra materialization of large inputs.

The 50-item list, 200-turn and 2 MiB text limits belong to the readers. The handler adds no
`slice(0,50)` or `slice(0,200)`. A fake returning one more valid row than a limit detects such a cut.

### 7.3 Classes

| Class | Allowed when |
|---|---|
| exact | terminal, value, trace, effects and shared state all match |
| P5 | only real native diagnostic prose differs; call, errno, class, cause and follow-up match |
| P6 | a recorded exact resource bound is exceeded with `InputTooLarge`, allowed trace prefix and required cleanup match |
| engine-policy | real runner integration with an existing policy ID, trigger and affected fields |
| excluded-domain | a specific forbidden value/behaviour is identified, with whether it was reached and the boundary result |
| unexplained | every other difference |

P5 never applies to fixed WireError messages or script-given Error.message; an empty synthetic
`checkFolder` message is exact. In common Q both sides get the same engine results, so engine-policy
cannot excuse a handler difference. L3-8's "opaque bound engine stringification" excluded-domain is not
inherited: the request bound comparison is `===`. Bad scripts, oracle load failures, crashes, watchdogs,
missing results and InternalInvariant are not passes. Title cut at 80 units, cancel precedence,
duplicate create/rm and a missing opening release are unexplained.

## 8. Fixed fixtures: 160

These are fixtures **to implement**. Ten per group, IDs in order (`S01–S10`, …). Defaults: fixed time
and UUIDs, empty opening, normal folder, no sessions.

| Group | The ten names |
|---|---|
| S sources | `home-order`, `account-label`, `nullish-label`, `duplicate-source-first`, `unknown-source`, `source-throw`, `no-home-leak`, `fresh-per-request`, `ambient-codex-home`, `invalid-account-skipped` |
| F folder/preflight order | `folder-normalized`, `folder-error-message`, `folder-empty-message`, `folder-nonerror`, `folder-abort-name`, `source-before-folder`, `folder-before-same`, `folder-before-known`, `list-folder-no-timer`, `open-folder-no-lock` |
| B bound | `external-original`, `external-fork`, `terminal-default-codex`, `slot-account-prefix`, `first-session`, `temporary-literal-true`, `temporary-callback`, `early-stop`, `fresh-list-per-item`, `empty-session-id` |
| L list | `unbound-title`, `bound-title`, `duplicate-title-last`, `empty-title-fallback`, `temporary-title-in-map`, `zero-metadata`, `undefined-metadata`, `null-metadata`, `drop-extra-fields`, `truthy-maybeopen` |
| A mode/account | `same-home`, `same-account-refused`, `same-grok-refused`, `same-ai-refused`, `fork-no-app-action`, `grok-target-valid`, `grok-target-unknown`, `grok-default-target`, `nongrok-target-ignored`, `account-before-known` |
| O open basics | `new-session`, `existing-session`, `existing-preaborted`, `empty-turns`, `same-no-fork`, `nongrok-fork`, `grok-no-fork`, `seed-account`, `model-wanted`, `fill-preserve-slots` |
| T turn/origin | `uuid-order`, `single-now`, `created-missing`, `created-empty`, `created-null`, `completed-present`, `tools-zero-positive`, `clipped-truthy`, `omitted-before`, `last-plan-preserved` |
| U title | `external-title`, `first-prompt`, `empty-fallback`, `whitespace-title-no-prompt-retry`, `unicode-whitespace`, `nonwhitespace-0085-200b`, `utf16-split80`, `basename-fallback`, `root-folder-fallback`, `long-folder-unclipped` |
| G Grok | `copy-success`, `home-error-wrap`, `home-nonerror-wrap`, `copy-exists`, `copy-reject-no-handler-rm`, `verify-success`, `verify-reject`, `verify-abort`, `copy-cancel-signal`, `copy-returned-path` |
| R rollback | `race-rm`, `race-rm-reject`, `cancel-after-copy`, `race-query-throws`, `date-throws`, `uuid-throws`, `model-throws`, `create-throws-after-fill`, `success-keeps-copy`, `catch-rm-reject` |
| E errors | `not-found`, `codex-unsupported`, `grok-exists`, `grok-load-failed`, `timeout`, `engine-missing`, `read-failed`, `plain-abort-name`, `unknown-eie-code`, `passthrough-value` |
| C cancel/time | `preabort-unbound`, `cancel-during-read`, `deadline-read-reject`, `deadline-read-ignored`, `deadline-options`, `verify-new-deadline`, `cancel-reason-precedence`, `cancel-during-race-rm`, `cancel-during-catch-rm`, `timer-listener-cleanup` |
| D options | `codex-list`, `codex-open`, `codex-prepare-throw`, `opencode-list-no-locate`, `opencode-open-binary`, `opencode-no-binary`, `locate-reject`, `temp-root-reject`, `env-signal-identity`, `claude-qwen-no-preparation` |
| I concurrency | `same-key-busy`, `known-before-busy`, `bind-during-read`, `bind-after-copy`, `different-keys`, `different-folders-same-key`, `different-sources-same-key`, `different-instances`, `release-after-failure`, `no-release-without-acquire` |
| P packet/limits | `bad-version`, `bad-kind`, `wrong-request-id`, `wrong-seq`, `wrong-call-kind`, `done-reentry`, `packet-too-large`, `active-request-limit`, `sequence-limit`, `resource-cleanup-after-copy` |
| X extra paths | `empty-list-still-title-scan`, `title-scan-throws`, `item-scan-throws`, `app-action-throws`, `grok-accounts-throws`, `abort-name-before-eie-code`, `more-than50-items`, `more-than200-turns`, `duplicate-session-title-and-first-bound`, `migrate-fork-cancel-contract` |

Every fixture checks expected results and trace assertions, the Node oracle, and the four pairs. Report
variant counts separately so "160" is not confused with run counts.

## 9. Oracle and integration

### 9.1 Oracle

On Node 26.4.0, import the req pin's `engineImportHandlers` directly. Do not re-implement `sourceOf`,
`boundSessionId`, `timed`, `importFailure`, the title expression or fill in JavaScript. Extend the
existing loader: entry and bound run the pinned files; `threadSlot` imports req `logic-records.ts`;
source/account/canonical helpers stay the pinned functions; only engine readers, prepare/save deps,
UUID, clock, rm and timers are fakes; EngineImportError uses the entry tail's single class identity; the
wire shim keeps key and message; DEFAULT_ACCOUNT is the same immutable binding on both sides
(`bench/l3/entry/reference-hooks.mjs`, `reference.mjs`).

The createImported fake builds the script's initial Session, calls the real `seed.fill`, and records the
whole session. It must distinguish failure before fill, failure after fill, and normal return. This is
the oracle for Pruv's fill recipe.

### 9.2 Integration: 12

Separate from the 160 fake fixtures: Codex, Grok, OpenCode, Qwen list/open (8); Grok copy/load success,
verify failure, race after copy (3); one engine error through the real entry wrapper to a request
WireError (1). Use the existing runners' exact-domain data. Keep apart the injected-reader integration
and the Grok callback lifecycle check; injecting reader results is not real cancel forwarding. Claude
keeps the L3-1 "no driver" limit. No live accounts, auth or engine installs are required.

## 10. Q generator: 2,000

**Design decision:** seed `20261119`, xorshift32 with rejection-sampled uniform, a fixed family multiset
then Fisher–Yates shuffle. The same JSONL feeds the four pairs and Node.

| Family | Cases | Axes |
|---|---:|---|
| Q1 sources/projection | 200 | accounts, duplicates, env, missing source, label |
| Q2 preflight | 200 | folder errors, mode, AI, Grok account, initial bound |
| Q3 list | 300 | duplicate session IDs, titles, temporary, metadata, per-item snapshot |
| Q4 open data | 300 | turns, time, UUID, slot, Unicode, title, plan |
| Q5 Grok | 250 | stage-wise success/failure of target/copy/verify/rollback |
| Q6 errors/cancel | 250 | class/name/code combos, reason identity, deadline, cancel during cleanup |
| Q7 concurrency | 250 | 2–4 requests, same/other key, session mutation, retry |
| Q8 protocol/domain | 250 | completion corruption, limits, explicit excluded values |
| Total | **2,000** | |

On failure save seed, case, family, input, script, both results and the first trace difference.
Shrinking is limited to dropping requests, sessions, turns, string content and schedule events while the
failure holds. A bad scheduler script is not a product difference.

## 11. Single-point mutants: 16

Each must be killed by the named fixture, which passes on the baseline. One condition, operator or call
per mutant.

| ID | Change | Killer |
|---|---|---|
| M01 | sourceUnknown → empty success | S05 |
| M02 | always the folder fallback instead of Error.message | F03 |
| M03 | last bound match instead of first | B05 |
| M04 | titles map first-write-wins | L03 |
| M05 | title fallback `||` → `??` | L04 |
| M06 | drop the same/fromAppAction refusal | A04 |
| M07 | skip the Grok target check | A07 |
| M08 | folder added to the opening key | I06 |
| M09 | drop opening.delete in the failure finally | I09 |
| M10 | timed deadline 20,000 → 19,999 ms | C03 |
| M11 | add `instanceof Error` to the AbortError test | E08 |
| M12 | drop the catch's cancel.reason precedence | C07 |
| M13 | drop the race re-search after async | I03 |
| M14 | drop the race rm of copied | R01 |
| M15 | drop a trailing high surrogate in the title cut | U07 |
| M16 | `threadSlot` ignores account | O08 |

The M10 fixture includes a boundary schedule where the timer has not fired at 19,999 ms and fires at
20,000 ms. M16 uses an initial fill state with both a default slot and an account slot. Run baseline and
all 16 on rs/rust; report killed, verifier-rejected, survived and build-error separately. Build errors
are not kills; any survivor means not done.

## 12. Forged replay: 32 + 8

**Design decision:** a separate suite of 32 forgeries and 8 honest controls, each changing one region of
a real await packet captured from a normal run.

| Group | Count | Forgery |
|---|---:|---|
| envelope/continuation | 8 | version, kind, request ID, instance ID, seq, op, call kind, done re-entry |
| log/checkpoint | 8 | log op/seq, missing/extra entry, unconsumed tail, item start, dropped count, phase/index |
| child entry | 4 | child op, child seq, duplicated source rows, another child's result |
| effects/ownership | 8 | opening key/owner, copied path, create done, rm done, timer ID, callback token, cancel reason ref |
| shared world | 4 | stale generation, double consume, another instance's completion, reuse of a released owner |

Honest controls: sources, list, basic open, existing bound, Grok verify, rollback, cancel, two interleaved
requests. Rejection is `InvalidPacket` with no new host effect after the rejection point. Reuse of a
wholly consistent past world is rejected by the driver's latest-generation check. A length check alone
is not a replay-safety claim.

## 13. Complexity and performance

Let S be sessions, K the max candidates per session, M list items, N turns, U string units.

- `boundSessionId`: worst `O(S·K)`, stops at first match.
- list: titles `O(S)`, then `O(M·S·K)` search and projection.
- open: two `O(S·K)` searches (initial and race), then `O(N+U)`.
- The titles map is a hash map or equivalent with last-write semantics.
- Do not swap the handler search for a whole index to save `isTemporary`/sessionList calls.

**Measure:** `S=1,000/5,000/20,000`, `M=1/10/50`, `K≤8`; no match, first-session match, last match. With
all temporary flags false and no match, list makes `M+1` `sessionList` calls and `M·S` `isTemporary`
calls; changing these counts as an optimization is a failure.

Replay must use item checkpoints so finished sessions and items are not re-run from the start (no extra
factor of S). Keep the source canonical climb's depth checkpoints and section offset table. Copying the
whole session snapshot on every isTemporary completion is not allowed. Report TS and Pruv wall time,
peak RSS, max packet words, host call counts and replayed items; separate trace cost from core cost. Do
not declare a pass number without measurement; fix any extra quadratic growth in S at fixed M/K.

## 14. Pin change impact

| Changed file | Finding and L3-9 handling |
|---|---|
| request handler | only the codexUnsupported message "판" → "버전"; compare the new text exactly |
| codex-home | codexAccountFolder meaning for sources unchanged; cleanup-failure recheck/notice added |
| migrate/work-run | suggestAdd added; openThread/cancel path unchanged |
| shared/contracts | handler-relevant Turn/Session bodies unchanged; do not read the engine list change as wider import support |

The Codex indirect path is `readOptions → makeCodexRunDeps → prepareCodexAccount →
assertCodexAccountSpawnAllowed`; prepare errors follow the new pin. Do not add an automatic
`recheckCodexAccountCleanup` call. The migration port always uses fork and passes its signal to
openThread; the caller checks cancel state and the AbortError name, so replacing cancel.reason with a
timeout WireError would change the caller. Keep this link contract as a fixture (X10) without porting
migration.

## 15. Implementation split (disjoint files)

- **W1 — Pruv request machine and shared entry:** `bench/l3/req/*.pruv`, `pruv.json`; moved entry
  modules and `bench/l3/lib/entry_api.pruv`; `bench/l3/entry/root.pruv` and build paths for the move.
  No entry semantic change, compiler change or host decision.
- **W2 — Rust host and deterministic runner:** `bench/l3/req/host.rs`, `host/`, `harness.rs`,
  `dev_build.sh`, `forge.rs`, `forge_build.sh`; only the public adapter boundary of
  `bench/l3/entry/host.rs`. Typed codec, sync/async split, scheduler, callbacks, fake deps, effect-once,
  trace. No account/title/bound/error decisions in the host.
- **W3 — Node oracle and evidence:** `bench/l3/req/reference*.mjs`, `fakes/`, fixtures, generator,
  comparator, `test_impl.py`, `integrate.py`, `q.py`, `q_mutants.py`, `forge_test.py`, `perf_*.py`, req
  DESIGN/README/IMPL_NOTES. No edits to pinned `reference/`, no re-implemented oracle, no live services.

## 16. Done conditions

1. The three handlers' decisions live in Pruv, compared against a Node oracle that runs the original
   handler.
2. The Python and Rust compilers verify the module graph.
3. 160 fixtures and 2,000 Q on py/rust, py/llvm, rs/rust, rs/llvm with 0 unexplained.
4. 16 mutants killed or meaningfully verifier-rejected; no survivors or build errors.
5. 32 forged replays rejected and 8 honest controls accepted.
6. 12 integration results and unsupported boundaries reported separately.
7. The performance range and the L3-8 shared-move impact checked.
8. RESULTS records exact/P5/P6/engine-policy/excluded-domain/unexplained, fixture and variant counts,
   commands and limits.

Not verified by this spec: the L3-9 implementation and its runs, real engines/services/Windows, and the
external `DEFAULT_ACCOUNT` literal (closed here as an explicit host binding).
