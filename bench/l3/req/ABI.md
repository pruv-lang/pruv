L3-9 uses a separate version-1 request ABI; the existing L3-8 entry ABI remains unchanged.  
Pruv owns handler decisions, opening ownership, error mapping, result construction, and cleanup requests.  
Rust owns transport, immutable value storage, Promise jobs, scripted effects, identity, and observation recording.  
The scheduler executes an explicit Promise graph; transport completions never introduce JavaScript scheduling points.  
This is an implementation contract derived from repository reads, not a claim of compiled or runtime-verified equivalence.

# L3-9 implementation ABI

## 1. Status, authority, and terminology

The layouts and enum assignments below are **new implementation decisions**. They are not descriptions of an existing request implementation.

Semantic authority:

- `docs/ENGINE_IMPORT_REQUEST_SPEC.md`, especially §§3–7 and §12.
- The pinned handler, `bench/l3/req/reference/backend-requests-engine-import.ts`.
- For observation details, the executable oracle takes precedence over descriptive summaries: `bench/l3/req/reference.mjs`, `world.mjs`, and their imported primitives.
- For child packets, the existing code takes precedence over older design prose: `bench/l3/lib/entry_api.pruv`, `ent_packet.pruv`, `ent_state.pruv`, and the other `ent_*` modules.

Evidence aliases used below:

| Alias | Repository-relative file |
|---|---|
| R | `bench/l3/req/reference/backend-requests-engine-import.ts` |
| W | `bench/l3/req/world.mjs` |
| O | `bench/l3/req/reference.mjs` |
| E | `bench/l3/entry/reference/backend-engine-import.ts` |
| EP | `bench/l3/lib/ent_packet.pruv` |
| ES | `bench/l3/lib/ent_state.pruv` |
| EA | `bench/l3/lib/entry_api.pruv` |
| ED | `bench/l3/entry/DESIGN.md` |

“Word” means an `i64`, not a byte. Offsets are zero-based. `[a,b)` denotes a half-open word range. `V` means a three-word value slot. Bare identifiers in record layouts are single protocol words.

A **transport suspension** returns control to Rust to perform a mechanical operation. It does not run another request, a Promise job, or a timer. A **JavaScript suspension** registers a Promise reaction and yields the current execution frame.

The two must not be conflated. This distinction is mandatory in `docs/ENGINE_IMPORT_REQUEST_SPEC.md:375–396`.

## 2. Files and ownership

### 2.1 Pruv modules: W1

`bench/l3/req/root.pruv` declares:

```text
req_packet
req_value
req_state
req_world
req_child
req_async
req_sources
req_bound
req_list
req_open
req_error
req_fill

ent_u16
ent_packet
ent_value
ent_state
ent_dispatch
ent_bound
ent_sources
ent_canon
ent_accounts
ent_path
entry_api
```

Declarations belong in the root, not repeated inside individual modules. Calls use `crate::<module>::<function>`.

| File | Responsibility |
|---|---|
| `root.pruv` | Public forwarding API; no handler policy |
| `req_packet.pruv` | Request/world/bundle envelopes, sections, limits, structural validation |
| `req_value.pruv` | Request value slots, typed views, ordered object construction, numeric predicates |
| `req_state.pruv` | Frames, checkpoints, current-item replay, continuation validation |
| `req_world.pruv` | Active-request ownership and opening-set decisions |
| `req_child.pruv` | Construct entry inputs, invoke `entry_api`, consume source deltas and child results |
| `req_async.pruv` | Pruv-side Promise-frame transitions, timed lifecycle, callback continuations |
| `req_sources.pruv` | Public source projection and first-match source selection through child ops 1/2 |
| `req_bound.pruv` | Fresh streaming search; child ops 8/7; strict matching and early iterator close |
| `req_list.pruv` | Titles map, per-item searches, public item projection |
| `req_open.pruv` | Preflight, acquire/release, read/copy/race/create/catch control flow |
| `req_error.pruv` | Folder errors, importFailure, original-value propagation |
| `req_fill.pruv` | Turns, title, origin, bound thread, slot, seed and fill recipe |
| `pruv.json` | Request root manifest |

Compile the request root with `--lib bench/l3/lib`. Do not copy shared modules into `req/`.

The move already exists: `bench/l3/entry/root.pruv:2–21` declares the shared modules and forwards to `entry_api`; `bench/l3/entry/test_impl.py:64–85` passes the library path.

Use expressible Pruv constructs: `[i64]`, explicit structs, `inout`, `! alloc`, and bounded loops with `invariant`/`decreases`. Encode callbacks and frames as numeric records, not closures or generic types. Examples exist in `ent_state.pruv:173`, `ent_bound.pruv:37–89`, and `ent_u16.pruv:159–173`. Constants may be nullary functions, following `ent_packet.pruv:14–16`.

### 2.2 Rust modules: W2

| File | Responsibility |
|---|---|
| `host.rs` | JSONL entry point, scenario orchestration, generated-core interface |
| `host/src/lib.rs` | Include/export host modules |
| `host/codec.rs` | JV, word packets, UTF-16, ordered properties, typed views |
| `host/driver.rs` | Latest continuation, tickets, effect-once, resource accounting |
| `host/scheduler.rs` | Promise objects, reaction jobs, adoption, callback entry, logical timers |
| `host/world.rs` | Script queues, sessions, iterators, signals, fake dependencies |
| `host/observe.rs` | Trace records, terminal rendering, created-session records |
| `harness.rs` | Generated Pruv functions → `req_host::Sol` |

These paths are the intended split, not a requirement to create empty files.

Reuse the entry host’s codec/primitive-serving logic through a narrow public adapter if useful. Do not call its standalone `drive` loop to execute a child: **Pruv invokes the child API**. The entry host’s existing engine service collapses scripted engine results into immediate completions (`bench/l3/entry/host.rs:1555–1608`); that behavior must not be reused for request scheduling.

## 3. Public API and envelopes

### 3.1 Pruv signatures

```text
pub fn req_begin(world: [i64], input: [i64]) -> [i64] ! alloc

pub fn req_step(
  world: [i64],
  previous: [i64],
  completion: [i64]
) -> [i64] ! alloc
```

Both return one bundle:

```text
[0x52514231, 1, world_words, packet_words, world'..., packet...]
```

Its total length must be `4 + world_words + packet_words`.

Rust interface:

```text
begin(&self, world: &[i64], input: &[i64]) -> Vec<i64>
step(&self, world: &[i64], previous: &[i64], completion: &[i64]) -> Vec<i64>
```

The generated-array calling convention follows `bench/l3/entry/harness.rs:5–13`.

`world` is the latest scenario world, containing separately identified handler instances. `previous` is only the selected request’s last returned continuation, not another request’s packet.

### 3.2 Request packet header

All request input, await, completion, and done packets use this 32-word header:

| Offset | Field |
|---:|---|
| 0 | Magic `0x52514531` |
| 1 | Version `1` |
| 2 | Kind: `0=input`, `1=await`, `2=completion`, `3=done` |
| 3 | Request operation: `1=sources`, `2=list`, `3=open` |
| 4 | Instance ID, positive integer |
| 5 | Request ID, positive integer, scenario-unique |
| 6 | Transport sequence |
| 7 | World generation at emission |
| 8 | Continuation ticket |
| 9 | Current frame phase |
| 10 | Outstanding call kind; zero for input/done |
| 11 | Current frame ID |
| 12 | Child call ID, or zero |
| 13 | Child entry operation, or zero |
| 14 | Child sequence, or zero |
| 15 | Mode: `0=ordinary`, `1=resource-unwind` |
| 16 | Checkpoint `item_start_seq` |
| 17 | Checkpoint `dropped` |
| 18 | Checkpoint `kept_log_entries` |
| 19 | Checkpoint current item index |
| 20 | Checkpoint item count |
| 21 | Checkpoint source rows already accepted for current child |
| 22 | Checkpoint cleanup flags |
| 23 | Checkpoint resume PC |
| 24 | Number of sections, at least 6 |
| 25 | Total packet words |
| 26–31 | Reserved, exactly zero |

Following the header:

```text
section_lengths[nsec]
section_0
...
section_(nsec-1)
```

Sections are contiguous. No overlap, trailing data, or unaccounted section is allowed.

| Section | Contents |
|---:|---|
| 0 | UTF-16 pool |
| 1 | Input/call/completion/done record |
| 2 | Request control state |
| 3 | Current item anchor blob |
| 4 | Current item completion log |
| 5 | Nested entry packet, or empty |
| ≥6 | Value tables referenced by slots |

Input has sequence/ticket/child fields zero and empty sections 2–5. An await has a positive sequence and ticket. A completion echoes the outstanding sequence, ticket, call kind, frame, and child identity. Done carries the last sequence and no outstanding call.

Sequences are consecutive **within a request**. The driver separately counts new external calls across the scenario. Interleaving another request does not create a gap in this request’s log.

### 3.3 World packet

World layout:

```text
[0x52515731, 1, generation, next_owner, next_ticket,
 instance_count, active_count, opening_count, pool_words,
 external_calls_used, cleanup_credits_reserved,
 0, 0, 0, 0, 0,
 pool...,
 instance_rows...,
 active_rows...,
 opening_rows...]
```

Rows:

| Table | Width | Fields |
|---|---:|---|
| Instance | 2 | `instance_id, active_count` |
| Active request | 10 | `instance_id, request_id, owner_token, op, last_seq, ticket, issued_generation, owns_opening, resource_unwind, reserved_zero` |
| Opening | 6 | `instance_id, key_pool_offset, key_units, request_id, owner_token, acquired_generation` |

Opening rows preserve Set insertion order. Keys are UTF-16 strings. Request/owner pairs are unique. A request has at most one opening row.

The driver constructs the initial empty world from `scenario.instances`, defaulting to `["A"]`. Pruv thereafter constructs world changes. The driver supplies the latest accepted world and authenticates its exact bytes.

On a valid core transition:

1. Consume the selected continuation once.
2. Advance generation by one.
3. Allocate a fresh ticket if another await is returned.
4. Store the new world and exact continuation bytes.
5. Only then dispatch the returned operation.

A pending request’s packet may have an older generation because another request advanced the shared world. Accept it only when its ticket and issued generation still match its active row and the driver’s latest packet for that request. Do **not** require every pending packet’s generation to equal the current world generation.

Completed request IDs and released owner tokens are tombstoned by the driver for the scenario and never reused.

### 3.4 Opening operations

Pruv computes exactly `engine + ":" + request.thread`, performs `has`, and decides whether to add or report busy. It emits opening observations through call kind 70 below.

Acquire and release change the Pruv world. Rust records those changes and verifies ownership; it does not independently perform the busy decision.

Only the owner that acquired a row may release it. Existing-bound and busy exits own nothing and emit no delete. This follows R:155–161,201.

## 4. Values, views, and continuation state

### 4.1 Value slots

Use the entry three-word slot representation:

| Tag | Meaning | `a` | `b` |
|---:|---|---|---|
| 0 | undefined | 0 | 0 |
| 1 | null | 0 | 0 |
| 2 / 3 | false / true | 0 | 0 |
| 4 | protocol integer | integer | 0 |
| 5 | JS Number | low 32 IEEE bits | high 32 IEEE bits |
| 6 | UTF-16 string | pool offset | unit count |
| 7 | dense array | section number | element count |
| 8 | ordered object | section number | property count |
| 9 | opaque identity | positive reference ID | 0 |
| 10 | absent own property | 0 | 0 |
| 11 | inspected object marker | 0 | 0 |

Request generic arrays have three-word element rows. Generic objects have six-word rows `[key V, value V]`; keys must be strings. Specialized entry tables retain their existing widths.

Tag 4 is never a JS Number. Tag 10 is allowed only for property views. Reading it yields undefined; reconstructing an object omits that property. Present-undefined remains present.

Request value helpers must treat tags 7/8 as truthy; the entry scalar `truthy` helper does not handle them (`ent_value.pruv:38–52`).

Use identity refs for signals, environments, opaque dependency objects, errors, records, rows, and persistent core-owned values. Do not stringify them.

### 4.2 Error observation

An error observation `E` is the unchanged entry layout, six slots, 18 words:

```text
valueRef
isEngineImportError
isError
name
message
code
```

`valueRef` exists even for primitive throws. `message` is a string when `isError=true`, otherwise undefined. See ED:249–256.

For internally constructed errors, Pruv emits a construction descriptor:

```text
[kind, name V, message V, code V, key V, hasCause, causeRef V]
```

Kinds:

- `1=WireError`
- `2=EngineImportError`
- `3=Error`
- `4=TypeError`

The host allocates the identity and stores the supplied fields mechanically. Pruv chooses class, key, code, text, cause, and precedence.

### 4.3 Typed views

A host ref is not permission for Rust to interpret handler policy. The following views expose fields without deriving decisions:

| View ID | Fixed slot order |
|---:|---|
| 1 Source | `source, engine, kind, account, accountName, home` |
| 2 List item | `thread, title, updatedAt, size, messages, path, maybeOpen` |
| 3 External record | `title, turnsRef, turnCount, model, omittedBefore, lastPlan` |
| 4 External turn | `prompt, reply, createdAt, completedAt, tools, clipped` |
| 5 Session | `id, title, temporary, engine, externalThread, terminal, engineThreads` |
| 6 Grok target | `home, binary, env` |
| 7 Copy result | `path, skipped` |
| 8 Locate result | `binary` |

Each slot is a `V`; `turnCount` uses tag 4. Missing fields use tag 10. Nested values are either generic values or identities with subsequent views.

Session conversion into the child’s 30-word session record must preserve the layout in ED:218–233 and `ent_bound.pruv:1–3`. Conversion does not decide exclusion or candidate matching.

The host binds every view to its originating reference and immutable snapshot. A record from another child cannot satisfy a view request for this child.

### 4.4 Core-owned persistent storage

Do not carry all prior sessions, titles, turns, or projected items in every continuation. Use immutable, request-owned arena blocks.

The host supports only:

- allocate a block from Pruv-supplied words;
- read an indexed range;
- append a Pruv-supplied block to a sequence;
- read an indexed sequence element.

These are transport operations, not JS calls or scheduling points. The host cannot search for a bound session, choose a title, deduplicate sources, or construct a fill field.

Pruv owns the titles-map algorithm. Use a persistent radix trie keyed by UTF-16 units, with 16 binary levels per unit and a separate terminal value. Updating a key replaces its terminal value; host storage only preserves the nodes Pruv supplies. This avoids repeatedly copying the whole title map and gives last-write behavior without host policy.

Arena handles carry `(instance, request, owner, allocation_id)`. Completed allocations remain immutable. A continuation contains handles, current indices, and the current item, not the full session snapshot.

As implemented, list storage is one chain per request. Each finished list item is packed into a block whose `prev` is the previous item's block (op 60, pc 13). Its record digest is folded into a running chain digest that the continuation keeps (work slot 25). The titles map is a stored sorted table, read back one record at a time (op 61, pcs 11–12 and 15). The finish reads the whole chain in one op (op 65, pc 14) and checks the chain digest and the item count against the continuation before it builds the result. Because every stored word is checked against a digest kept in Pruv state, a host that alters, drops or reorders a block yields InvalidPacket (forgery rows F41–F45 in section 11), not a wrong result. The radix trie described above is not used; the sorted table keeps the same last-write semantics.

### 4.5 Control state and frames

Section 2 starts with:

```text
[0x52515331, 1, owner_token, frame_count, next_frame_id,
 source_rows_ref, titles_root_ref, result_sequence_ref,
 turns_sequence_ref, original_input_ref, chosen_source_ref,
 folder_ref, account_ref, copied_path_ref, pending_error_ref,
 reserved_zero]
```

All `*_ref` words are arena/reference IDs; zero means absent. A referenced undefined JS value is distinct from zero.

Then `frame_count` rows, each 16 words:

```text
[frame_id, role, pc, return_promise, awaited_promise,
 parent_frame, child_call_id, child_op, child_seq,
 timer_id, controller_id, listener_token, copy_token,
 callback_token, local_data_ref, status]
```

Roles:

| ID | Role |
|---:|---|
| 1 | Handler |
| 2 | `readOptions` |
| 3 | Async work arrow |
| 4 | Entry list/read wrapper |
| 5 | `timed` |
| 6 | Grok-home catch reaction |
| 7 | Copy verify callback |
| 8 | Cleanup catch reaction |

Status: `0=running`, `1=waiting`, `2=returned`, `3=thrown`.

Handler phases:

```text
0 source construction
1 source lookup
2 folder
3 same checks
4 account
5 initial bound search
6 opening acquire
7 timed read setup
8 options
9 child list/read
10 timed read completion
11 titles scan
12 list item search/projection
13 Grok target
14 copy
15 verify timed
16 cancel-before-save
17 race search
18 race rm
19 now
20 turn conversion
21 title/origin/bound
22 modelFor
23 createImported
24 catch rm
25 catch precedence/mapping
26 opening release
27 terminal
```

Sub-PCs count the next action within the corresponding sequence in this document. They are stored and validated, not inferred by Rust.

Cleanup flag bits at header offset 22:

```text
1 opening owned
2 copied path owned and truthy
4 timed timer live
8 external cancel listener registered
16 create returned normally
32 handler rm completed
64 resource rejection pending
```

Bits are checked against frame state, world ownership, and accepted effect receipts. They are not independently authoritative.

### 4.6 Item checkpoint and replay

Item boundaries are:

- one child source/account item;
- one streamed session;
- one list item;
- one converted turn;
- one timed setup/finally;
- one copy callback;
- one cleanup operation.

Section 3 contains the accepted state immediately before the current item, using the same state schema, plus the immutable handles needed by that item.

Section 4 contains entries:

```text
[entry_words, call_kind, frame_id, seq, completion_words, completion_packet...]
```

Require:

```text
item_start_seq + dropped + kept_log_entries == outstanding_seq
```

Within retained entries, sequences are strictly increasing and below the outstanding sequence. W1 regenerates the outstanding operation by replaying the current item against its completion log, compares the complete arguments, and consumes the complete log. Extra tail entries are invalid.

At an item boundary, commit the new anchor and discard the preceding item log. Never replay completed sessions from the beginning of a search. Preserve the child’s own canonical-climb checkpoints.

This extends the existing discipline rather than changing the entry checkpoint: ES:25–28,258–302,332–385; `ent_sources.pruv:244–280`.

The driver additionally compares submitted continuation bytes with its stored latest continuation. Pure packet validation does not authenticate a wholly consistent old history.

## 5. Call and completion vocabulary

### 5.1 Common records

Section 1 of an await contains the arguments specified below; header offset 10 identifies the call.

Ordinary completion record:

```text
[status, body...]
```

Status:

| Value | Meaning |
|---:|---|
| 0 | Synchronous success; body is the specified result |
| 1 | Synchronous throw; body is `E` |
| 2 | Async invocation returned a Promise; body is `promise_id` |
| 3 | Async copy returned a Promise and entered an inline callback; body below |
| 4 | Scheduler event delivery; body below |
| 5 | Resource refusal; body is `1` (`InputTooLarge`) |

Async completion is not delivered merely because a dependency Promise settled. Promise reactions determine when a Pruv frame resumes.

All calls except `PARK` return their transport completion without advancing the scheduler. Calls that invoke an async function return its Promise identity immediately.

### 5.2 Dependency calls

`p` below is a protocol integer; other arguments marked `V` are slots.

| Kind | Name | Arguments | Successful result / behavior |
|---:|---|---|---|
| 1 | `ENTRY_LEAF` | nested entry await in section 5 | nested completion, or Promise ID for engine invocation |
| 2 | `CHECK_FOLDER` | `path V` | real folder `V` |
| 3 | `SESSION_OPEN` | none | `iterator_id p` |
| 4 | `SESSION_NEXT` | `iterator_id, expected_index` | `done p, sessionRef V`; done has undefined value |
| 5 | `SESSION_CLOSE` | `iterator_id, consumed` | undefined |
| 6 | `IS_TEMPORARY` | `id V` | raw result `V` |
| 7 | `FROM_APP_ACTION` | none | raw result `V` |
| 8 | `GROK_ACCOUNTS` | none | dense account array `V` |
| 9 | `UUID` | none | value `V` |
| 10 | `NOW` | none | ISO string `V` |
| 11 | `MODEL_FOR` | `engine V, account V, wanted V` | model `V` |
| 12 | `CREATE_IMPORTED` | seed `V`, fill recipe `V` | session ID `V` |
| 13 | `LOCATE_ACP` | name `V`, exactly `"opencode"` | Promise of locate result |
| 14 | `MAKE_CODEX_RUN_DEPS` | source `V`, dataDir `V` | Promise of opaque run deps |
| 15 | `TEMP_ROOT` | none | Promise of string |
| 16 | `GROK_HOME_FOR` | account `V` | Promise of Grok target |
| 17 | `COPY` | source `V`, folder `V`, thread `V`, targetHome `V`, callback token, cancel signal `V` | Promise of copy result; callback protocol below |
| 18 | `LOAD_CHECK` | check `V`, home `V`, folder `V`, thread `V`, signal `V` | Promise of raw result |
| 19 | `RM` | copied path `V`, recursive `1`, force `1` | Promise of raw result |
| 20 | `VIEW` | reference `V`, view ID, index | fixed view slots |
| 21 | `INTERN_VALUE` | constructed value `V` | identity `V` |
| 22 | `MAKE_ERROR` | error construction descriptor | `E` for the new identity |

`ENTRY_LEAF` covers unchanged entry call codes 1–9:

- accounts/count and account-at;
- ambient `CODEX_HOME`;
- homedir, cwd, realpath, stat;
- single-session temporary predicate;
- routed engine list/read.

Entry code 8 for request bound search must contain exactly one ID. Route numbering remains `0=claude,1=codex,2=opencode,3=qwen,4=grok`. See ED:235–263.

The host’s public `IS_TEMPORARY` operation and the entry code-8 adapter use the same req script table and trace function. Do not use the standalone entry fixture’s call-order `isTemporary` queue: the req oracle uses an ID-keyed table, W:391–395.

`SESSION_OPEN` snapshots references in current session order. `SESSION_NEXT` reproduces the scripted throw-at-index behavior. Close after an early match or an abrupt loop-body exit; do not close after normal exhaustion or after `next()` itself throws. Iterator close must not replace an already pending body throw. The oracle’s iterator implementation is W:370–389.

### 5.3 Timers, signals, and listeners

| Kind | Name | Arguments | Result |
|---:|---|---|---|
| 30 | `CONTROLLER_NEW` | none | controller ID, signal `V` |
| 31 | `SIGNAL_STATE` | signal `V` | aborted `0/1`, reason `V` |
| 32 | `CONTROLLER_ABORT` | controller ID, reason mode, reason `V` | undefined |
| 33 | `LISTENER_ADD` | external signal `V`, listener token, target controller ID, once `1` | listener ID |
| 34 | `LISTENER_REMOVE` | external signal `V`, listener token | undefined |
| 35 | `TIMER_SET` | delay `20000`, target controller ID | timer ID |
| 36 | `TIMER_CLEAR` | timer ID | undefined |
| 37 | `COPY_VERIFY_RETURN` | copy token, callback token, callback Promise ID | undefined |

Abort reason mode `0` means call `abort()` without an argument; mode `1` supplies a reason. The handler’s internal controller uses mode 0.

Creating a controller does not assign its observable `sigN` name. Naming occurs at the same first-sight locations as `signalName` in W:91–103,193–199,227–230,241–243. A signal aborted before first sight must not acquire a retrospective abort trace.

A cancel listener performs only the registered controller abort, synchronously. Pruv chooses registration, target controller, and removal. Timer fire similarly invokes only its registered abort action.

### 5.4 Promise and scheduler operations

These are mechanical transport operations, not observable dependency calls.

| Kind | Name | Arguments | Result |
|---:|---|---|---|
| 50 | `PROMISE_NEW` | none | Promise ID |
| 51 | `PROMISE_REACT` | source Promise, destination Promise or zero, reaction mode, frame ID, site, occurrence | reaction ID |
| 52 | `PROMISE_SETTLE` | Promise ID, resolution kind, value `V`, adopted Promise ID | undefined |
| 53 | `PARK` | none | next legal event for this request |
| 54 | `QUEUE_FRAME` | frame ID, site, occurrence, outcome kind, value `V` | undefined |

Resolution kinds: `0=fulfill value`, `1=reject value`, `2=resolve with Promise`.

Reaction modes:

- `0=await`: deliver fulfillment or rejection to the frame; no destination Promise.
- `1=then`: enter frame on either branch; frame settles destination.
- `2=catch`: fulfillment passes through to destination in its reaction job; rejection enters frame.
- `3=identity`: pass fulfillment/rejection to destination in a reaction job.
- `4=observer`: deliver terminal observation; no destination.

`QUEUE_FRAME` represents an explicit `queueMicrotask`, not normal await.

A scheduler event completion is:

```text
[4, event_kind, frame_id, site, occurrence,
 promise_id, outcome_kind, value V,
 copy_token, callback_token]
```

Event kinds: `1=Promise reaction`, `2=VerifyRequested`, `3=explicit microtask`.

For event 2, `value` is verify home; copy/callback tokens are nonzero. For other events those token fields are zero. Error values have identities; their `E` view remains available.

A `PARK` ticket is consumed by exactly one event. The resumed core may perform many synchronous transport calls, register more reactions, and park again. Other ready events remain queued.

A request is done only when its handler result is ready and its terminal observer has run. It may remain `pending` at scenario end.

### 5.5 Storage and opening observations

| Kind | Name | Arguments | Result |
|---:|---|---|---|
| 60 | `BLOCK_NEW` | previous block handle (0 for none), raw words | `[0, handle]` |
| 61 | `BLOCK_READ` | handle, offset, count | `[0, words...]` |
| 65 | `CHAIN_READ` | last block handle | `[0, n, (len, words...) × n]`, oldest first |
| 70 | `OPENING_EVENT` | op `0=has/1=add/2=delete`, key `V`, result `0/1`, owner token | acknowledgment |

Storage ops are awaits of call kind 4 (header 10). The host answers each one at once. A handle that belongs to another request, a range outside the block or an unknown op gets the refusal `[1]`. Pruv treats a refusal, and any reply with a pool section, as InvalidPacket (F46–F48). A storage op is neither a trace line nor an external call (section 10). Ops 62–64 of the original plan are not needed, because op 60 links each block to the previous one.

Storage operations and opening recording do not introduce microtasks. Opening changes must already be represented in `world'`; the host validates the event against that transition before recording it.

### 5.6 Seed and fill recipe

Seed is an ordered object:

```text
engine
account                 only if Pruv includes it
model
folder
```

The host reports `seedKeys` as these keys followed by `"fill"`.

Fill recipe is an ordered object with exactly:

```text
title
turns
externalThread
slot
bound
```

Rust mechanically performs:

```text
session.title = recipe.title
session.turns = recipe.turns
session.externalThread = recipe.externalThread
session.engineThreads = spread_existing_then_set(recipe.slot, recipe.bound)
```

Preserve other slots and ordinary JS property enumeration order. The host does not calculate the slot, title, turns, origin, fork flag, or last turn ID.

Creation modes and bookkeeping follow W:413–430:

- failure before fill: no created-session record and no created counter increment;
- failure after fill: record before/after, mark failure, do not append to sessions;
- normal return: append to sessions and return ID.

No handler rollback undoes a partial save.

## 6. Child entry computations

### 6.1 Unchanged child envelope

The entry packet remains:

```text
[1, kind, op_id, seq, payload_words,
 op, nsec, section_lengths..., sections...]
```

Its state magic remains `1162761298`; its eight checkpoint words retain their original meaning. Evidence: EP:1–6; ES:25–28; EA:22–67.

For each parent, allocate `childCallId=1,2,...`. Use that value as the child `op_id`, scoped by parent instance/request/owner.

Maintain this mapping:

```text
(instance, request, owner, childCallId, childSeq)
    -> parent transport seq and tick
```

For nested entry completions, compare every component, not only child sequence.

### 6.2 Operations used

| Purpose | Child op |
|---|---:|
| Build fresh sources | 1 |
| Find first matching source | 2 |
| List | 3 |
| Read | 4 |
| Candidate threads for one session | 7 |
| Exclude one session | 8 |

Never use child op 9 for request bound search.

Source rows are emitted in section 3 of **both await and done packets**. Accept each emission once, keyed by `(childCallId, childSeq, kind)`, preserving order and duplicates. If the child fails, discard the accumulated source result. See `ent_sources.pruv:261–284` and ED:297–303.

The parent invokes `entry_begin` and `entry_step` directly through `crate::entry_api`. The host only serves emitted leaves.

For op 3:

1. Host retains immutable original row identities and their typed views.
2. Child receives `[rowRef,title]` rows.
3. Child returns keep/copy-title recipe.
4. Parent uses that recipe as projection input.
5. Parent performs bound-title replacement and field inclusion.

For op 4, the child returns an opaque record ref. Parent requests view 3 and processes fields/turns itself. The existing child read result is deliberately only a ref: `ent_dispatch.pruv:172–184`; `bench/l3/entry/host.rs:1595–1603`.

### 6.3 Child async boundary

Only child engine invocation is a genuine async boundary here. Source/path/account transport and op-8 temporary checks remain synchronous.

The engine fake’s async wrapper is represented by the scheduler. Its settled Promise resumes the entry frame; that frame calls `entry_step` and resolves/rejects the entry-wrapper Promise with the child’s semantic result.

Do not add an extra await merely to move a child packet through Rust.

## 7. Exact scheduler model

### 7.1 Queue semantics

Use:

- one scenario-global FIFO microtask queue;
- Promise states `pending/fulfilled/rejected`;
- reaction lists in registration order;
- a logical timer collection ordered by `(due_time, registration_id)`;
- saved request context `(r,i,fromAppAction)` on each registered job.

Promise settlement is synchronous. It enqueues eligible reactions at the queue tail. A reaction on an already settled Promise still enqueues one job.

`resolve(P, Q)` where `Q` is a Promise performs Promise adoption:

1. Enqueue an assimilation job.
2. That job registers forwarding reactions on `Q`.
3. The forwarding reaction runs as a later job and settles `P`.

Do not replace adoption with direct forwarding or one arbitrary tick. There is no universal “one await = one total tick” rule.

Frame execution is run-to-completion until it returns, throws, or registers an await. Transport calls made by the frame are completed in the same execution stretch.

The supported domain has native Promises created by this oracle, plain values, and no user-defined thenables or patched Promise constructors. This does not remove any current fixture or specified Q family.

### 7.2 Scenario events and timers

Replicate O:46–59:

1. Drain microtasks before each script event.
2. Execute the event.
3. Drain microtasks after it.

`quiesce()` uses two real `setImmediate` waits in Node (W:315), but those add no observation. Rust drains the corresponding finite Promise-job queue; it does not sleep.

For `advance(ms)`:

1. Record `advance` in scheduler context.
2. Select the earliest timer with `due <= target`, tie-breaking by ID.
3. Remove it before invoking its callback.
4. Set logical time to its due time.
5. Run the callback in its saved request context.
6. Drain microtasks completely.
7. Repeat, then set logical time to target.

This is W:317–330. Timers do not fire merely because wall time passed.

### 7.3 Required Promise graph

W1 must emit the following graph; W2 executes it without selecting handler branches.

```text
readOptions async frame
    -> options Promise

work async arrow:
    await options Promise
    invoke child entry list/read
    return entry Promise                 [adoption]

timed async frame:
    synchronous controller/listener/timer setup
    invoke work
    await work Promise
    synchronous finally clear/remove
    return value or throw

handler:
    await timed Promise
    continue list/open decisions

terminal observer:
    Promise.resolve(handler result).then(done, fail)
```

For the verify path, the callback is **not** an async arrow. It directly returns the Promise from `timed(signal => grokLoadCheck(...), cancel)`. Do not insert the options await or work-arrow adoption into this path. R:162–166.

### 7.4 Await sites and job costs

A “job” below means a FIFO Promise reaction or assimilation job, not elapsed logical milliseconds.

| Site | Required scheduling |
|---|---|
| A01 `await locateAcp` | One await reaction after the returned fake Promise settles |
| A02 `await makeCodexRunDeps` | One await reaction |
| A03 `await tempRoot` | One await reaction |
| A04/A05 `await readOptions` | One reaction even when readOptions returned without any internal await |
| A06 entry engine await | One entry-frame reaction after the async reader wrapper settles |
| A07 `await work` in timed | One reaction; clear timer/remove listener execute inside this reaction |
| A08 handler receiving timed | One reaction after timed settles |
| A09 Grok-home `.catch` and await | One catch reaction on success or failure, then one handler await reaction |
| A10 copy/verify | Graph below; inline callback entry itself costs zero jobs |
| A11 load check | One timed await reaction after load Promise settles |
| A12/A13 `rm(...).catch(...); await` | One catch reaction, then one handler reaction |

Additional mandatory jobs:

- Ordinary immediate `asyncServe`: one `Promise.resolve().then(finish)` job.
- `traced`: one reaction that updates the original trace record’s `o`.
- Deferred `asyncServe`: gate identity reaction, then finish reaction, then traced reaction.
- Every `async function { return anotherPromise; }`: assimilation plus forwarding reaction.
- Terminal `.then`: one observer reaction.
- Already-aborted `gate` with `onAbort`: explicit `queueMicrotask(fire)` before its settlement chain.

Evidence: W:149–186; O:94–98; req `fakes/engine-claude.ts:3–4` and equivalent reader wrappers; E:73–93.

Useful isolated-path counts, excluding unrelated queued jobs:

| Path | Jobs to the indicated continuation |
|---|---:|
| No-prepare readOptions → A04/A05 continuation | 1 |
| Immediate prepare fake invocation → its readOptions await continuation | 3: finish, trace-update, await |
| Immediate Grok-home invocation → handler after A09 | 4: finish, trace-update, catch, await |
| Immediate rm invocation → handler after A12/A13 | 4: finish, trace-update, catch, await |
| Immediate reader invocation → handler after A08 | 9 total jobs across fake finish/update, reader adoption, entry await, work adoption, timed await, handler await |
| Immediate load invocation → verify timed settlement | 3: finish, trace-update, timed await |

The nine reader jobs are a **count**, not permission to enqueue nine delay tokens. Adoption jobs are inserted when their corresponding async functions return and may run before the dependency has settled. Only the graph establishes the correct interleaving.

A synchronous throw from `readerCall` becomes an immediate rejection of the async reader wrapper. There are no fake finish/update or wrapper-adoption jobs on that branch, but entry still resumes through its await. A synchronous throw from `locateAcp` occurs inside readOptions before its await registration and rejects the readOptions Promise. Preserve these distinctions.

These counts are derived from the source structure; they have not been measured against a built Rust scheduler in this task.

### 7.5 Copy and `VerifyRequested`

The copy fake differs from `asyncServe`; implement W:241–258 literally at the boundary:

1. Record `copy` pending before running its async body.
2. With no defer, its body proceeds immediately.
3. With defer, await the gate Promise, including the gate identity reaction.
4. If configured, record `copy.verify` and enter the Pruv callback synchronously.
5. Await the callback’s returned timed Promise.
6. On failure, apply the copy boundary’s existing AbortError/grok-load-failed translation.
7. Settle the copy body’s Promise.
8. Its trace-update reaction settles the externally returned copy Promise.
9. The handler’s A10 reaction resumes.

`COPY` status 3 body:

```text
[promise_id, copy_token, callback_token, home V]
```

This means the invocation entered verify before returning normally. W1 starts the callback inline, obtains its return Promise, sends `COPY_VERIFY_RETURN`, and then completes the outer invocation’s normal Promise/await setup.

For a deferred copy, `VerifyRequested` is delivered while executing the copy body’s resume job. Suspend that job’s host stack, enter the Pruv callback without inserting a microtask, obtain its Promise, and resume the host copy body.

Callback token ownership includes instance/request/owner/copy token. A callback result is consumed once. The outer copy Promise remains pending while the verify leaf is active.

`copied_path_ref` remains absent until the copy Promise fulfills and the handler reads `.path`. A failed verify never creates handler-owned cleanup. R:160–166; specification §5.4.

### 7.6 Abort and finally ordering

Abort dispatch is synchronous in the context that called abort:

- script cancellation: `r/i=null`;
- timeout: the timer creator’s saved context.

Handle instrumentation records the handle abort before registered user listeners run and removes once-listeners from its active accounting. A forwarded controller abort calls `abort()` with **no original reason**.

The internal signal therefore has a DOM AbortError reason, while later handler cancellation checks throw the original external `cancel.reason`. R:74–75,169,199. The contrary sentence in `SCENARIO.md` is not the executable contract.

Pending fake gates settle synchronously during abort dispatch unless the signal was already aborted on registration; that case queues `fire`. Their Promise reactions run after dispatch ends.

Timed finally runs before timed settles. Open finally releases opening before the handler Promise settles. The terminal observer runs after those actions.

A deadline only aborts. It does not reject a racing Promise or force a timeout terminal. R:71–77; current C04/C05 fixtures.

## 8. Handler decisions and exact outputs

### 8.1 Sources and preflight

Pruv must:

- build sources afresh for every request;
- preserve source order and duplicate rows;
- use child op 2 for first match;
- finish full source construction before folder checking;
- project sources to ordered keys `source,engine,[account],label`;
- include account only if truthy;
- use account name nullish fallback for account labels.

Folder wrapping uses `Error.message`, including empty string, otherwise the fixed folder fallback. It lies outside the handler import-failure try scopes. R:80–89,123–130,145–147.

Open order is:

```text
sources → folder → same refusals → account → initial bound
→ opening.has → opening.add → timed read
```

An existing truthy bound wins before busy and cancel. A matched empty session ID stops the search but does not trigger the caller’s truthy-existing branch.

### 8.2 Bound search and list

For every search:

1. Open a fresh session iterator.
2. Get one session.
3. Invoke child op 8.
4. If excluded, continue.
5. Invoke child op 7.
6. Compare engine and thread using strict equality.
7. On first match, close iterator and return that session ID.

Do not stringify candidate values or build a whole bound index.

As implemented, the list keeps only the current item in its continuation. Finished items go to the request's storage chain (section 4.4), so the continuation stays small for any session count. A list of 100,001 sessions runs within the 16,777,216-word packet cap.

List first builds its complete last-write-wins titles map from one separate iterator, including temporary sessions. Then it opens a fresh iterator per list item.

Public item key order:

```text
thread, title, [updatedAt], [size], [messages], [path],
[boundSessionId], [maybeOpen]
```

Metadata inclusion uses “not undefined,” retaining null, zero and empty string. `maybeOpen` uses truthiness and emits literal true. Bound title uses `titles.get(bound) || item.title`. No new title cap is applied to a Lair title. R:133–141.

### 8.3 Open data

Pruv reads the clock once and obtains one UUID per turn, in order.

Turn key order:

```text
id, prompt, reply, progress, status, usage, createdAt,
[completedAt], imported
```

Values:

- progress `""`;
- status `"completed"`;
- usage null;
- createdAt uses nullish fallback to now;
- completedAt included unless undefined;
- imported keys `engine,[tools],[clipped]`;
- tools retained only when numerically greater than zero;
- clipped becomes true when truthy.

EngineThread:

```text
threadId, [lastTurnId], [fork]
```

ExternalThread:

```text
engine, source, thread, mode, importedAt, [omittedBefore], [lastPlan]
```

Keep truthy lastPlan as supplied. Do not normalize it.

Title:

```text
candidate = external.title || first_turn.prompt || ""
collapse ECMAScript whitespace runs to one space
trim
take the first 80 UTF-16 units
fallback to basename(real_folder), then real_folder
```

Do not retry first prompt after a whitespace-only external title collapses to empty. Do not collapse or clip the folder fallback. Reuse `ent_u16::is_trim_unit`; retain a trailing high surrogate after slicing. R:175–193; `ent_u16.pruv:148–173`.

`threadSlot` is decided by Pruv from engine/account using the pinned `logic-records.ts` rule.

### 8.4 Cleanup and terminal records

Done section-1 records:

| Tag | Body |
|---:|---|
| 0 | success value `V` |
| 1 | original thrown identity `V` |
| 2 | WireError key `V`, message `V` |
| 3 | constructed error identity `V` |
| 5 | core error code |

Core codes: `0=InvalidPacket`, `1=InputTooLarge`, `2=InternalInvariant`.

Normal new open returns `{sessionId}`. Existing returns `{sessionId,existing:true}`. Never emit `existing:false`.

For truthy owned copied paths:

- race path: await swallowed rm rejection, then return existing without another cancel check;
- catch path: await swallowed rm rejection, then inspect cancel again;
- normal create return: clear copy ownership without rm.

Open catch checks external cancel before importFailure. `importFailure` checks the AbortError name before EIE codes, regardless of `instanceof Error`. All fixed keys/messages come from R:55–68,82,88,108,151–158. Pruv constructs them; Rust does not choose a mapping.

## 9. Observation construction

Emit exactly the oracle’s outer shape:

```text
{
  id,
  requests,
  trace,
  createdSessions,
  openingEvents,
  finalOpening,
  activeTimers,
  activeListeners,
  driver
}
```

### 9.1 Trace

Every record has:

```text
{c, r, i, a?, o?}
```

Absent `a/o` remain absent. Do not replace absence with null or undefined markers.

Host recording rules:

- append async-call records at invocation with `{pending:true}`;
- update that same record’s `o` when its traced reaction executes;
- do not append a second completion record;
- retain the request context captured at registration;
- convert entry primitive traces to `c:"prim", a:[primitive_name,...]`;
- account-at/view/arena/packet traffic produces no trace;
- entry code-8 and engine calls use req `isTemporary` and `reader` trace shapes, not the standalone entry shapes.

Evidence: W:63–73,181–189; O:31–37.

The host records Pruv-chosen call arguments. It may render names and identity labels, but may not repair or recompute arguments.

### 9.2 Requests and terminals

`requests` retains start order. Each initially has `terminal:"pending", value:null`.

Success values use the oracle JV encoder. For throws, apply this rendering precedence:

1. Identity equals an aborted handle’s reason → `{cancelReason:handle[,ref]}`.
2. Registered thrown/sentinel identity → `{ref:name}`.
3. WireError → `{wire:{key,message}}`.
4. Unregistered EngineImportError → `{eie:{code,message,hasCause[,cause]}}`.
5. DOM AbortError → `{domAbort:1}`.
6. Other Error → `{native:{name,message}}`.
7. Other value → JV.

Terminal classification is:

- wire marker → `"wire"`;
- eie marker → `"eie"`;
- native/domAbort → `"native"`;
- otherwise thrown → `"thrown"`.

Thus a scripted Error can have terminal `"thrown"` because registered identity takes precedence. W:45–59; O:102–107.

For implementation-only protocol/resource tests use `terminal:"core"` and `value:{code:"InvalidPacket|InputTooLarge|InternalInvariant"}`. Such results are compared by the protocol/resource contract, not falsely claimed to be Node terminals.

### 9.3 Other fields

| Field | Source |
|---|---|
| `createdSessions` | Mechanical application of Pruv seed/fill plus scripted before/after failure stage |
| `openingEvents` | Accepted Pruv opening events, `{i,r,op,key,result}` |
| `finalOpening` | Pruv world opening rows grouped in scenario instance order and Set insertion order |
| `activeTimers` | Remaining timers in registration order, `{id,delay,due}` |
| `activeListeners` | Every created external handle in creation order mapped to active listener count |
| `driver` | `null` normally; `{violations:[...]}` for scenario/driver faults |

Do not run pending timers or invent completion events to empty these fields at scenario end. Pending requests and live effects are observable.

### 9.4 Identity and JV

Use UTF-16 throughout. Render well-formed strings normally, lone-surrogate strings as `{u16:[...]}`. Preserve negative zero, NaN, infinities and non-safe-integer bit patterns through `{num:hex}`.

Objects render as `{obj:[[key,value],...]}`; arrays as `{arr:[...]}`. Preserve JS integer-key enumeration order, duplicate-key replacement position, and ordinary `__proto__` own properties. ED:488–502,604–610.

Preserve scripted reference names in raw observations. The comparator may alpha-normalize opaque refs by first appearance, using one shared map across the **entire observation**, not one per field. Never rename UUIDs, times, request IDs, instance IDs, handles, paths, or signal labels.

Primitive thrown-name selection follows the oracle’s most-recent Object.is-equal scripted throw rule, not “one ref per primitive value”: `bench/l3/entry/reference-primitives.mjs:116–128`.

## 10. Bounds and rejection behavior

Limits:

- each request/world/input/completion/state/output packet: at most `16,777,216` words;
- at most 32 active requests per instance;
- at most 1,000,000 new external dependency/effect calls per scenario;
- no handler list/turn truncation.

Internal Promise bookkeeping, immutable storage transport and observation acknowledgments do not count as external dependency calls. They still receive per-request transport sequences and effect-once tickets.

All arithmetic is checked before allocation or indexing. Unused fields are zero. Unknown tags, invalid lengths, wrong ownership and impossible PCs are InvalidPacket.

Resource handling:

1. Secure opening ownership, copied path and timed cleanup descriptors before bulk materialization.
2. Before admitting an effect, reserve the remaining necessary cleanup calls within the scenario call budget.
3. Track exact outstanding cleanup credits: timer clear, listener removal when present, and returned-copy rm when required.
4. Refuse ordinary work when its admission would consume reserved cleanup capacity.
5. Drop nonessential bulk accumulators on resource unwind.
6. Complete Pruv-directed cleanup and release before returning InputTooLarge.
7. Never convert malformed/replayed packets into cleanup commands.

A returned copy path must be decoded into the small cleanup descriptor before decoding optional bulk copy data. An oversized read/list result must be reported before materializing its whole typed view.

### 10.1 The call budget as implemented

World word 9 counts the external calls of the scenario. World word 10 counts the cleanup credits reserved. Both are zero in the host's initial world. `world_ok` requires `0 <= w9`, `0 <= w10` and `w9 + w10 <= 1,000,000` (forgeries F49 and F50).

- **Unit.** One external-call seq (spec §7.2). Each await of call kind 1 (child), 2 (folder) or 3 (host call) costs one call, because each is one new call seq that leaves a dependency or effect trace line. Session-iterator steps that make a trace line (host call 14: `sessionList`, `sessionList.next`, `sessionList.done`; host call 16: `sessionList.return`) count. Host call 17 costs nothing: it only re-reads the current snapshot session for the next child call (packet op 7) and makes no trace line, so it is internal bookkeeping (section 10). Storage ops (kind 4) cost nothing. `await_cost(ck, call)` in `root.pruv` is this rule. Before this rule a host call 17 was counted as well, and a 45,000 × 11 list ended with InputTooLarge after 333,327 `isTemporary` lines; it now finishes (1,035,038 trace lines, `perf_req.py --sizes 45000 --ms 11 --ops list`).
- **Credits.** A request holds one credit while its pending await is at a pc where host call 25 still has cleanup to do. For a list (op 2) that is pcs 3 and 4: the timed reader list from host call 11 until the timer clear in host call 12 at pc 5. For an open (op 3) it is every pc from 3 on, except 21, 22 and 34: the list timer, the read window's timer and listener, the returned copy and the opening. `held(op, pc)` in `root.pruv` is that table. One credit stands for the single host call 25 that does all of this cleanup.
- **Admission.** `charge` admits an await at pc only when `w9 + cost + w10 - held(previous pc) + held(pc) <= 1,000,000`. If so, it adds the cost to w9 and moves the request's credit to the new pc. `done_ev` gives the request's credit back when it ends.
- **Refusal.** A refused request that holds no credit ends with core InputTooLarge, without any new effect. A refused request that holds a credit spends it on the unwind await at pc 34. That is host call 25, with tag 12 for a Grok open that already has its returned copy (rm first; this includes a copy that returned after a cancel in phase 1, `open_fail` tag 4) and tag 2 otherwise, and with the opening release events. Its cost always fits, because the credit it spends was reserved. When the call answers, the request ends with InputTooLarge. Forge rows B01–B06 seed w9 near the limit and check this order: timer clear, then rm when a copy exists, then opening delete, then `request.done` with InputTooLarge.

InvalidPacket rejection has no new external effects, including “helpful” cleanup. The trusted driver retains the last honest state; a forged attempt must not mutate it.

InternalInvariant means a core bug and is never an accepted differential class.

The 50-item, 200-turn and 2 MiB reader policies are not new request limits. Current X07/X08 intentionally exercise more than 50 items and 200 turns.

## 11. Forgery suite: exact mapping

Capture a real await; mutate one designated region; submit it through the production validation path. Every rejection must occur before another host effect.

| ID | Mutation | Field/check |
|---|---|---|
| F01 | Version | Header 1 |
| F02 | Kind | Header 2 |
| F03 | Request ID | Header 5 and active-row identity |
| F04 | Instance ID | Header 4 |
| F05 | Sequence | Header 6 |
| F06 | Operation | Header 3 |
| F07 | Call kind | Header 10 and regenerated arguments |
| F08 | Done re-entry | Kind-3 input to step |
| F09 | Log op | Embedded completion header 3 |
| F10 | Log seq | Log entry seq and embedded header 6 |
| F11 | Missing log entry | Continuity/count/full replay |
| F12 | Extra log entry | Count/full consumption |
| F13 | Unconsumed tail | Replay cursor must reach log end |
| F14 | Item start | Header 16 and anchor |
| F15 | Dropped count | Header 17 and continuity |
| F16 | Phase/index | Header 9/19/20/23 versus frame/anchor |
| F17 | Child op | Header 13, child packet op, active frame |
| F18 | Child seq | Header 14, child packet seq, mapping |
| F19 | Duplicate source rows | Header 21 and emission receipt key |
| F20 | Another child’s result | ChildCallId plus ref/view provenance |
| F21 | Opening key | World opening row versus Pruv-computed key |
| F22 | Opening owner | World row owner versus active owner |
| F23 | Copied path | Control-state ref versus accepted copy receipt |
| F24 | Create done | Cleanup flag 16 versus create completion receipt |
| F25 | Rm done | Cleanup flag 32 versus rm completion receipt |
| F26 | Timer ID | Frame timer versus registered owner/controller |
| F27 | Callback token | Frame copy/callback pair versus pending invocation |
| F28 | Cancel reason ref | Signal reason identity versus registered handle |
| F29 | Stale world generation | Driver latest-world comparison |
| F30 | Double consume | Ticket already consumed |
| F31 | Other instance completion | Completion identity and pending-call ownership |
| F32 | Released owner reuse | Driver owner tombstone |
| F33–F34 | Completion header 18, a seventh section | Completion must echo the await; exact section count |
| F35–F39 | List reply slots: unknown tag, string past the pool, ref word, bound-id tag, excluded word | Value-slot validation |
| F40 | Child completion one word short | Child envelope length |
| F41–F42 | Stored item record changed, one word short | Record digest in the chain (section 4.4) |
| F43–F44 | Chain element changed, chain count differs | Chain digest and item count in the continuation |
| F45 | Stored titles word changed | Titles-table digest |
| F46–F48 | Storage op refused, chain append handle 0, storage reply with a pool section | Storage replies (section 5.5) |
| F49–F50 | World calls used past 1,000,000; calls used plus credits past 1,000,000 (both resealed) | `world_ok` budget range (section 10.1) |
| F51 | Session re-read (call 17) record changed, section 5 honest | Call-17 reply is `[0]` with an empty pool |
| F52 | Folder success slot outside the empty completion pool | `slot_ok` on the folder slot |

Budget rows B01–B06 are not forgeries. They reseal an honest world with a high w9 and expect InputTooLarge after exactly the cleanup the request still owes (section 10.1).

Eight honest controls: sources, list, new open, existing open, Grok verify, rollback, cancellation, and two interleaved requests.

Core structural/replay checks and driver latest-state checks are complementary. A self-consistent old packet cannot be rejected by a stateless function solely because it is old; the driver closes that gap, as required by specification §5.5 and ED:178–186.

## 12. Comparator and staged implementation

### 12.1 Comparator contract

Provide a Python comparator in `bench/l3/req/compare.py` with:

```text
compare_observation(expected, actual, policy=None)
    -> {class, path, expected, actual}
```

Classes:

```text
exact
P5
P6
engine-policy
excluded-domain
unexplained
```

Requirements:

- Compare the complete observation, not only `expect` subsets.
- Preserve array and ordered-object entry order.
- Distinguish missing, undefined, null, false, zero and empty string.
- Normalize only opaque reference names, consistently across the observation.
- Report the first difference using a stable path such as `trace[17].a.options.signal`.
- Require exactly one result per input scenario and matching IDs/order.
- Crashes, missing results, malformed JSON, driver violations and InternalInvariant are failures.
- P5 applies only to explicitly identified native diagnostic prose.
- P6 requires a named bound trigger, allowed trace prefix and required cleanup.
- Common fake Q cases cannot use engine-policy to excuse handler differences.

The existing fixture shape is top-level `{id,world,script,...}`, not a nested `scenario` member. Use `common.strip_case`, `bench/l3/req/common.py:13–31`.

### 12.2 Important P-fixture distinction

Current P fixtures carry `layer:"packet"` but execute honest Node scenarios. They do not themselves inject bad packets.

In particular:

- P08 starts 33 concurrent opens and expects all 33 to succeed in Node.
- P09 is an ordinary 20-turn case, not sequence exhaustion.
- P10 checks ordinary copy cleanup after create failure.

Evidence: `bench/l3/req/gen_fixtures.py:555–572`.

Therefore:

- P01–P07/P09/P10 honest baselines may be compared normally when within bounds.
- The P08 implementation run must be classified through the explicit 32-active-request P6 policy; it must not be required to equal Node’s unbounded 33-success result.
- Separate protocol injections and reduced-budget test configuration exercise the named faults.
- Reduced-budget configuration is test metadata shared by the driver/core admission logic, never a changed production constant.
- Do not report 160 exact cases by silently omitting P08 or ignoring its difference.

### 12.3 Milestones

**M0 — codec and entry bridge**

- W1: envelopes, validation, world lifecycle, child ops 1/2, source projection.
- W2: JSONL interface, JV/identity codec, synchronous entry primitives, trace and terminal observer.
- Freeze the numeric ABI in this document before either side adds independent enum assignments.

**M1 — sources end to end**

- Run S01–S04 and S07–S10 through Node and implementation.
- S05/S06 also contain list/open preflight; include them only after those paths exist.
- Compare complete observations.
- This is the smallest useful parallel-integration milestone.

**M2 — preflight and list**

- W1: folder/same/account checks, streaming bound loop, titles and item projection.
- W2: iterators, views, Promise graph, prepare/read fakes, timers/signals.
- Exercise F/B/L/A/D and empty-list behavior.

**M3 — ordinary open**

- W1: opening world, race search, turn/title/seed/fill, errors.
- W2: clock/UUID/create modes, complete creation observations, concurrency contexts.
- Exercise O/T/U/E and non-Grok I cases.

**M4 — Grok/cancellation**

- W1: target catch, copy callback, verify timed, cleanup and cancel precedence.
- W2: copy Promise graph, inline/deferred verify entry, abort gates, swallowed rm.
- Exercise G/R/C and remaining I/X paths.

**M5 — protocol/resource closure**

- Add all 32 forgeries and eight honest controls.
- Exercise packet size, active-request and sequence-admission limits with cleanup.
- Explicitly verify that rejected forgeries cause no additional effects.

**M6 — required completion evidence**

- Four compiler/backend pairs, full fixed-fixture classification, Q families, mutants, integration and specified performance ranges.
- Check the shared-entry move’s affected regressions if that move is changed.
- Do not run unrelated compiler suites merely because the request ABI was implemented.

The staging does not change the specification’s final completion conditions. A sources milestone, successful build, or packet suite alone is not completion.

## 13. Confirmed and not verified

**Confirmed by repository reading**

- The handler ordering, await structure, copy ownership, cancellation precedence and fill contents cited above.
- Existing child API, packet fields, replay checkpoints, emitted source deltas and opaque read results.
- Oracle trace mutation, iterator behavior, timer ordering, signal naming and thrown-identity rendering.
- The actual 160 fixture files and generator, including the current P-fixture limitations.
- The stale descriptive statements about nested scenario objects and forwarded abort reasons.

**Not runtime-verified in this task**

- Compilation/proofs of this proposed ABI.
- Differential behavior of a Rust implementation of the specified Promise graph.
- Fixture, Q, mutation, forgery, integration or performance pass counts.

No files were changed. Live engines, accounts, services, Windows, external documentation and files outside this repository were outside this read-only task.