# Engine Import Common Entry — L3-8

Status: design specification for L3-8. Implementation and verification results are recorded in `RESULTS.md` §140.

Target: Lair commit `744d14af`.

This slice moves common source construction, source lookup, dispatch selection, list-title clipping, exception classification, and bound-session indexing into Pruv. Existing engine implementations remain separate and retain their current interfaces.

## 1. Evidence and citation convention

All paths below are relative to the Pruv repository. The following citation aliases expand to exact reference files; `E:34–53`, for example, means `bench/l3/entry/reference/backend-engine-import.ts:34–53`.

| Alias | File |
|---|---|
| E | `bench/l3/entry/reference/backend-engine-import.ts` |
| B | `bench/l3/entry/reference/backend-engine-import-bound.ts` |
| A | `bench/l3/entry/reference/backend-accounts.ts` |
| C | `bench/l3/entry/reference/backend-codex-home.ts` |
| T | `bench/l3/entry/reference/backend-engine-import-tail.ts` |
| Q | `bench/l3/entry/reference/backend-engine-import-qwen.ts` |
| D | `bench/l3/entry/reference/backend-engine-import-codex.ts` |
| R | `bench/l3/entry/reference/backend-requests-engine-import.ts` |
| M | `bench/l3/entry/reference/backend-migrate-work-run.ts` |
| S | `bench/l3/entry/reference/shared-engine-import.ts` |
| SC | `bench/l3/entry/reference/shared-contracts.ts` |
| O | `bench/l3/grok/reference/opencode.ts` |

The entry reference manifest identifies the files as verbatim pinned copies, lists the file-to-path mapping, and supplies their hashes. The Grok manifest supplies provenance for `grok.ts`, `opencode.ts`, and `plan.ts`.
Evidence: `bench/l3/entry/reference/README.md`; `bench/l3/grok/reference/README.md:3–18`.

The SHA-256 values of E, B, A, C, and Q were checked against the entry manifest. The Qwen reference under `bench/l3/qwen/reference/qwen.ts` was also compared with Q and was byte-identical. Use Q for new pinned line citations.

### 1.1 Collision behavior

**`boundSessionIndex` is first-write-wins, not last-write-wins.**

Its insertion is guarded by `if (!out.has(key))`. A duplicate key retains the first eligible session ID. The separate request-handler title map uses unconditional `titles.set(session.id, session.title)` and is last-write-wins for duplicate session IDs. These are different maps and different contracts.
Evidence: B:35–44; R:133–138.

This specification preserves both facts. Changing the bound index to last-write-wins would violate the pin and is a required mutation-test failure.

## 2. Scope

### 2.1 Ported common behavior

| Surface | Required Pruv ownership | Reference |
|---|---|---|
| `importSources` | Environment trimming and fallback, fixed home sources, account iteration order, account inclusion and per-account failure swallowing | E:23–53 |
| `findSource` | Strict source-ID equality, first matching entry, absent result | E:56–59 |
| `listExternal` | Dispatch selection, exactly one engine invocation, title postprocessing, common exception classification | E:61–83 |
| `readExternal` | Dispatch selection, exactly one engine invocation, unchanged successful record, common exception classification | E:85–94 |
| `accountHome` | ID validation and lexical path construction | A:10–13,54–57 |
| Relevant `codexAccountFolder` logic | Expected-home comparison, canonical-path algorithm, same-file comparisons, host-home rejection | C:49–97 |
| `sessionThreads` | Candidate generation, order, defaults, slot parsing | B:15–27 |
| `excludedFromBound` | Literal-true temporary flag and lazy temporary callback | B:29–32 |
| `boundSessionIndex` | Exclusion, key construction, first-write collision behavior, insertion order | B:34–44 |

The environment helpers used by source construction are included: `hostCodexHome`, `opencodeDataDir`, `qwenGlobalDir`, and `qwenRuntimeDir`. Only their source-directory behavior is included.
Evidence: C:40–43; O:38–42; Q:15–23.

### 2.2 Excluded behavior

The following remain outside L3-8:

- Account creation, name validation, login configuration, folder creation, chmod, authentication-file inspection, token access, provider preparation, and account-process management. `accountHome` does not perform these operations.
  Evidence: A:16–50,54–71; C:92–135.
- `makeCodexRunDeps`, executable discovery, and account preparation. The caller supplies already prepared execution dependencies.
  Evidence: D:23–35; R:113–115.
- Request authorization, folder authorization, source-error localization, UI source projection, bound-title substitution, opening locks, session creation, copy rollback, and request timers.
  Evidence: R:3,54–89,96–115,119–201.
- Migration installation checks, sorting, ledger writes, opening, cancellation, and recovery orchestration.
  Evidence: M:217–255,379–400,518–535.
- Grok copy/load/removal operations, even though E re-exports some of them.
  Evidence: E:9,20; R:163–173.
- `boundThreadIndex`; its existing Set helper is not needed to implement the accepted scope.
  Evidence: B:47–54.
- Rewriting the five engine implementations into a common state machine.
- Windows behavior, real external cancellation support absent from existing pieces, compiler changes, and unrelated shared-library refactoring.

The common wrapper must not add new authorization or preparation behavior under the guise of forwarding.

## 3. Exact reference semantics

### 3.1 Supported value domain

The equivalence domain is an ordinary, unmodified JavaScript runtime with plain data objects, dense arrays, and the field types described below. Optional fields may be absent; explicitly named nullish and malformed-value compatibility cases are also supported.

Proxies, throwing getters, custom iterators with side effects, prototype pollution, and monkey-patched built-ins are excluded. This is an explicit representation limit, not permission to alter normal empty-string, nullish, ordering, Unicode, or error behavior.

Source and session strings use exact UTF-16 code-unit identity. OS-facing paths are restricted to representable non-Windows paths accepted by the existing adapters. Pure lookup, title, and bound helpers still support lone UTF-16 surrogates.

Reference field shapes: T:49–102; S:17–27; SC:530–538,575–603,1068–1070.

### 3.2 Environment trimming and paths

Use ECMAScript `String.prototype.trim`, not ASCII-only trimming and not a host-language whitespace approximation.

The trim set is:

- U+0009–U+000D;
- U+0020;
- U+00A0;
- U+1680;
- U+2000–U+200A;
- U+2028, U+2029;
- U+202F;
- U+205F;
- U+3000;
- U+FEFF.

U+0085 and U+200B are not trimmed. Internal whitespace remains unchanged. The set is the expansion of the `.trim()` operations at E:36,50; C:42; O:40; Q:17,22.

For each `trim() || fallback` expression, an absent value or a trimmed empty string selects the fallback. A nonempty relative value is retained unless the specific helper requires an absolute path.

| Source | Exact selection |
|---|---|
| Claude | Trim `deps.env.CLAUDE_CONFIG_DIR`; otherwise `join(homedir(), '.claude')` |
| Codex | Trim **ambient `process.env.CODEX_HOME`**; otherwise `join(homedir(), '.codex')` |
| Grok | Trim `deps.env.GROK_HOME`; otherwise `join(homedir(), '.grok')` |
| OpenCode | Trim `deps.env.XDG_DATA_HOME`; use `join(xdg, 'opencode')` only if nonempty and absolute; otherwise `join(homedir(), '.local', 'share', 'opencode')` |
| Qwen | Trim `deps.env.QWEN_RUNTIME_DIR`; otherwise trim `deps.env.QWEN_HOME`; otherwise `join(home, '.qwen')` |

Evidence: E:36,43,50–52; C:41–42; O:39–41; Q:16–22.

Consequences:

1. `deps.env.CODEX_HOME` does not control Codex source construction.
2. `deps.env.HOME` does not replace the `homedir()` observation.
3. Nonempty Claude, Codex, Grok, and Qwen overrides are not resolved or normalized by source selection.
4. OpenCode’s accepted XDG path is joined and normalized.
5. The Qwen call evaluates `homedir()` even when `QWEN_RUNTIME_DIR` is nonempty, because `homedir()` is an argument to `qwenRuntimeDir`.
6. No environment value is expanded as `~`, `$HOME`, or shell syntax.

Evidence: E:34–52; C:41–42; O:39–41; Q:16–22.

Lexical path operations follow non-Windows `node:path`: `join`, `resolve`, `dirname`, `basename`, and `isAbsolute`. Pruv owns these pure operations. `resolve` uses an observed working directory when required. Do not silently make every returned home absolute: the reference can return a relative environment override or a relative account path.
Evidence: A:4,56; C:7,49–68; O:7,39–41.

### 3.3 Source construction and account iteration

The successful output order is exactly:

1. `claude:home`;
2. accepted Claude accounts, in callback array order;
3. `codex:home`;
4. accepted Codex accounts, in callback array order;
5. `grok:home`;
6. `opencode:home`;
7. `qwen:home`.

Each home source has `kind: 'home'`, the named engine, and its selected `home`. Account fields are absent.

Each accepted account source has:

```text
source      = engine + ":" + account.id
engine      = "claude" or "codex"
kind        = "account"
account     = account.id
accountName = account.name
home        = computed account home
```

Names are copied unchanged, including empty strings and whitespace. Source construction does not invoke `accountName`, enforce `MAX_ACCOUNTS`, sort accounts, or deduplicate IDs.
Evidence: E:34–53; compare A:9,16–20.

The calls to `accounts('claude')` and `accounts('codex')` occur at their corresponding loop positions. A callback failure is outside the per-account `try` and escapes the whole operation. A failure inside one account’s body skips only that account and continues. There is no common `rethrow` wrapping around `importSources`.
Evidence: E:38–48,64–67.

Do not return a partial source list after an escaping failure.

### 3.4 `accountHome`

A valid ID is a string matching:

```text
^acc_[a-f\d]{16}$
```

Only lowercase ASCII hexadecimal digits are accepted after `acc_`. No trimming, case folding, coercion, trailing newline, slash, or extra character is accepted.

Invalid IDs throw the wire error identified by `wireEngine.account.invalidId`. Valid IDs return:

```text
join(dataDir, "engine-homes", engine, id)
```

There is no filesystem observation and no folder creation.
Evidence: A:10–13,54–57.

The public L3-8 helper accepts only the two engines needed by common source construction, Claude and Codex. It is not a replacement for all uses of Lair’s broader `EngineId` helper.

### 3.5 `codexAccountFolder`: pure decisions and observations

The function first recomputes `expected = accountHome(dataDir, 'codex', account.id)`.

- If `account.home !== expected`, throw the plain Error from C:94.
- Otherwise run `isHostCodexHome(expected)`.
- If that result is true, throw the wire error identified by `wireEngine.account.folder.codexHostExecution`.
- Otherwise return `expected`.

The equality is string equality, not canonical-path equality. In `importSources`, the supplied `account.home` is already the result of the same `accountHome` calculation; the mismatch branch remains testable through the direct helper.
Evidence: E:46; C:92–97.

#### Canonical-path algorithm

For `canonicalPath(path)`:

1. Set `current = resolve(path)` and `missing = []`.
2. Attempt `realpathSync(current)`.
3. On success, return that real path if no suffix is missing; otherwise return `join(real, ...missing)`.
4. On failure with code other than `ENOENT`, attempt `statSync(current)`.
5. A successful stat returns `dev:${String(dev)}:ino:${String(ino)}`, optionally followed by `/` and `missing.join('/')`.
6. `ENOENT` skips that stat attempt.
7. If the stat attempt fails, or realpath failed with `ENOENT`, calculate `parent = dirname(current)`.
8. If `parent === current`, return `resolve(path)`.
9. Otherwise prepend `basename(current)` to `missing`, set `current = parent`, and repeat.

Permission errors and symlink loops are therefore not automatically grounds for skipping an account. They enter the identity/ancestor fallback.
Evidence: C:49–70.

#### Host-home comparison

Evaluate, in order:

```text
target = canonicalPath(home)
host   = canonicalPath(hostCodexHome())
dot    = canonicalPath(join(homedir(), ".codex"))
```

All three canonical values are computed before the equality test.

If `target === host || target === dot`, return true. Otherwise evaluate:

```text
sameFile(home, hostCodexHome())
    || sameFile(home, join(homedir(), ".codex"))
```

The second `sameFile` is short-circuited when the first succeeds. `hostCodexHome()` is called again for this stage; do not assume that earlier observation may be reused.
Evidence: C:83–88.

`sameFile` stats the left path, then the right path, follows links, and returns equality of both `dev` and `ino`. Any stat failure makes that call false. No `lstat`, owner, mode, directory-kind, or authentication check is added.
Evidence: C:73–80; contrast C:99–104.

#### Boundary decision

Pruv owns:

- ID validation;
- lexical path operations;
- expected-home comparison;
- ancestor traversal;
- interpretation of `ENOENT`;
- whether and when to request stat;
- identity-string assembly;
- canonical and device/inode equality;
- short-circuiting;
- account rejection and skip-on-failure.

The host observes:

- ambient `CODEX_HOME`;
- homedir and cwd;
- requested realpaths;
- requested stat results or failures.

A stat completion contains the numeric `dev` and `ino` values and their lossless JavaScript string renderings. Producing those renderings is boundary conversion; the host does not compare them or decide account eligibility.

No auth/token read is permitted.

### 3.6 `findSource`

Search the supplied array from left to right using strict equality of `entry.source` and the requested string.

- Return the first matching entry.
- Return `undefined` when absent.
- Empty strings are legitimate lookup values.
- Do not trim, parse, validate, case-fold, or deduplicate.
- Do not interpret the source ID as a path.
- Do not generate an exception for absence.

The boundary returns the matching index, or an explicit absent result, so the host can preserve the original entry identity.
Evidence: E:56–59.

### 3.7 Dispatch

Both list and read choose the same route:

| `source.engine` | Selected implementation |
|---|---|
| `claude` | Claude |
| `codex` | Codex |
| `opencode` | OpenCode |
| `qwen` | Qwen |
| Anything else | Grok |

The fallback is Grok, including unknown or empty engine strings in compatibility probes. It is not an engine-validation branch and does not use `engine ?? 'codex'`. That default belongs only to terminal bound-thread extraction.

Pass `source`, `folder`, `thread` where applicable, and the original options through without changing them. There is exactly one selected engine call. Synchronous throws and asynchronous rejections enter the same common catch.
Evidence: E:73–93.

The wrapper itself does not validate folders, cap list length, sort rows, validate thread IDs, or prepare binaries.
Evidence: E:73–93; R:80–89,113–115.

### 3.8 List-title clipping

After a successful list call, apply the following independently to each item:

```text
title.length > 200
    ? { ...item, title: title.slice(0, 200) }
    : item
```

Evidence: E:61–62,80–82.

Required details:

- Length and slicing count UTF-16 code units.
- Length 200 is unchanged.
- Length 201 is cut to exactly the first 200 code units.
- There is no whitespace folding, trim, ellipsis, Unicode normalization, byte clipping, or grapheme handling.
- A short item remains the same object.
- A clipped item is a shallow copy preserving its other enumerable own properties.
- The result array is new, in input order.
- A supplementary character occupying units 199 and 200 in one-based numbering is retained in full.
- A supplementary character beginning at unit 200 is split: its high surrogate is retained as the final code unit and its low surrogate is discarded.
- An existing lone surrogate is retained if it lies in the retained prefix.

For example:

```text
"a" × 198 + "😀"       → unchanged, 200 units
"a" × 199 + "😀"       → "a" × 199 + "\uD83D", 200 units
```

This common 200-unit cut is **exact**. The older engine-local policy that drops a scalar split by an internal title cut does not apply here.
Evidence: E:81; compare `docs/QWEN_IMPORT_SPEC.md:75–79`, `docs/CODEX_IMPORT_SPEC.md:160`, `docs/OPENCODE_IMPORT_SPEC.md:288–290`.

### 3.9 Successful reads

Return the successful engine record unchanged. Do not impose a second collector, title cut, plan cut, turn cap, or field projection. Preserve the record identity at the wrapper boundary.
Evidence: E:86–93; record shape T:80–95.

### 3.10 `rethrow`

Apply the tests in this order:

1. If the value is an actual `EngineImportError` instance, throw the same value.
2. Otherwise, if its optional `name` property equals exactly `'AbortError'`, throw the same value.
3. Otherwise create:

```text
new EngineImportError(
    "read-failed",
    value instanceof Error ? value.message : undefined,
    { cause: value }
)
```

Evidence: E:64–67.

Consequences:

- A plain object with `name: 'AbortError'` passes through.
- An Error with that name passes through regardless of its concrete Error subclass.
- A plain object named `EngineImportError` is not an instance and is wrapped.
- A non-Error object’s `message` is not used as detail.
- Thrown strings, numbers, booleans, null, and undefined are wrapped without diagnostic detail.
- The original value remains the cause, including undefined.
- An Error with an empty message produces message `read-failed`, not `read-failed: `.
- A nonempty Error message produces `read-failed: ${message}`.
- `TimeoutError`, lowercase `aborterror`, and other near matches are not AbortError.
- An `EngineImportError` still wins the first branch even if its name has been changed.

The supported import error codes are:

```text
not-found
codex-unsupported
grok-exists
grok-load-failed
timeout
read-failed
engine-missing
```

The constructor sets `name = 'EngineImportError'`, preserves `code`, and uses truthiness of `detail` to select its message form.
Evidence: T:35–46.

This layer does not convert AbortError to timeout. The request handler performs that separate localization.
Evidence: R:54–68.

### 3.11 `sessionThreads`

Generate candidates in the following sequence:

1. If `externalThread` is truthy, emit its `{engine, thread}`.
2. Emit each `externalThread.forks ?? []` value in array order, with the origin’s engine.
3. Read `terminal?.cliSession`; if truthy, emit it with engine `session.engine ?? 'codex'`.
4. Visit `Object.entries(session.engineThreads ?? {})` in ECMAScript own-enumerable string-key order. For each entry whose `bound?.threadId` is truthy, emit:
   - engine: `slot.split(':', 1)[0]`;
   - thread: `bound.threadId`.

Evidence: B:15–27.

Required edge cases:

- Origin and fork strings are not filtered for emptiness.
- Duplicate origins, forks, terminal IDs, and slot IDs remain duplicates in the generator output.
- Missing/null forks produce no fork candidates.
- Empty terminal IDs are omitted.
- Missing/null terminal engine defaults to Codex; an empty engine string remains empty.
- Slot `codex` gives engine `codex`.
- Slot `codex:a:b` gives engine `codex`.
- Slot `:a` and slot `''` give engine `''`.
- Unknown engine names are retained.
- Empty/missing/null slot thread IDs are omitted.
- No terminal `started` check is performed.
- Integer-index object keys enumerate before other string keys, in ascending numeric order; other string keys retain insertion order. Symbols and inherited properties are not entries.

Evidence: B:17–25; SC:598–603,1068–1073.

### 3.12 `excludedFromBound`

The exact expression is:

```text
session.temporary === true || isTemporary(session.id)
```

Literal `true` excludes immediately and suppresses the callback. Missing, false, null, numeric `1`, and string `"true"` do not satisfy the first test; they require the callback.

The callback is evaluated once per eligible session occurrence, not once per distinct ID. A callback failure escapes; it is not converted to `read-failed`.
Evidence: B:30–31,37–38.

### 3.13 `boundSessionIndex`

Iterate sessions in supplied iterable order. Exclude temporary sessions before enumerating their threads. For each remaining candidate, construct:

```text
key = engine + ":" + thread
```

Insert the session ID only when the map does not already contain that key.

Required behavior:

- First eligible writer wins.
- An empty session ID is still a stored value and still blocks later writers.
- Duplicate candidates do not move an existing key.
- Map iteration order is first insertion order.
- The key contains no source ID or account ID.
- Colons are not escaped. `{engine: 'a:b', thread: 'c'}` and `{engine: 'a', thread: 'b:c'}` collide.
- Do not split the resulting key to reconstruct a pair.
- No sorting or deduplication occurs before insertion.

Evidence: B:35–44.

The caller’s separate title Map is last-write-wins by session ID. Its title substitution and fallback remain outside this slice.
Evidence: R:133–141.

## 4. Pruv interface

This section defines the new interface; it does not describe already implemented entry files.

### 4.1 Representation

> **Amendment (implementation design).** `bench/l3/entry/DESIGN.md` §2–§3 fixes a concrete flat layout with the same value domain: a section header, one UTF-16 pool, fixed 3-word value slots, an explicit absent-property tag, and fixed-width tables. Every distinction below stays representable. Where the two differ in layout only, DESIGN.md is the wire contract; this section remains the semantic contract.

Use a versioned flat `[i64]` packet. Strings use UTF-16 code units rather than UTF-8 so the new common layer can represent the exact title-cut result and exact source/bound keys.

Each encoded value is:

```text
[tag, payload_word_count, ...payload]
```

| Tag | Meaning | Payload |
|---:|---|---|
| 0 | undefined | empty |
| 1 | null | empty |
| 2 | false | empty |
| 3 | true | empty |
| 4 | protocol integer | one signed i64 |
| 5 | JS Number | low and high unsigned 32-bit words of IEEE-754 binary64 |
| 6 | string | UTF-16 units, each 0…65535 |
| 7 | array/tuple | element count followed by encoded values |
| 8 | plain object | pair count followed by encoded string-key/value pairs |
| 9 | opaque host reference | one positive integer ID |

A missing object property differs from an own property whose value is undefined. Empty strings differ from absent values. Object keys must be unique; array lengths must match their payloads.

Opaque references preserve identity of options, successful records, original list rows, and thrown values. The core may retain or return them but may not dereference them except through specified observation requests.

A packet is:

```text
[1, kind, operation_id, sequence, payload_word_count, ...encoded_payload]
```

Kinds:

```text
0 input
1 await
2 completion
3 done
```

Sequence numbers start at 1 for the first emitted request and increase by one. An operation has at most one outstanding common-layer request. This does not reduce concurrency inside an existing engine implementation.

Malformed packets produce a boundary error; they must never become a successful import result.

### 4.2 Public functions

```text
entry_begin(input: [i64]) -> [i64] ! alloc
entry_step(previous_await: [i64], completion: [i64]) -> [i64] ! alloc
```

Supported operation names and arguments:

| Operation | Arguments | Successful value |
|---|---|---|
| `import_sources` | `{dataDir, depsEnv}` | ordered source objects |
| `find_source` | `{sources, source}` | matching integer index or undefined |
| `list_external` | `{source, folder, optionsRef}` | list reconstruction recipe |
| `read_external` | `{source, folder, thread, optionsRef}` | original record reference |
| `account_home` | `{dataDir, engine, id}` | path |
| `codex_account_folder` | `{dataDir, id, home}` | path |
| `session_threads` | `{session}` | ordered `{engine, thread}` array |
| `excluded_from_bound` | `{session}` | boolean |
| `bound_session_index` | `{sessions}` | ordered `[key, sessionId]` pairs |

`session` contains only the fields read by B:15–31. `sessions` is an ordered finite snapshot. The host may project away unread fields, but must not omit, normalize, or prefilter any field read by these functions.

For `engineThreads`, the object codec preserves own property values and enumeration order. Numeric-key ordering is verified at the codec boundary; it must not become lexicographic ordering.

The operation ID identifies a host invocation. It does not carry domain meaning.

### 4.3 Await and completion payloads

An await payload is:

```text
[state, request]
```

`state` is the core-owned continuation, encoded as a value. The host returns the entire previous await packet unchanged to `entry_step`. Its internal layout is not a host contract.

A request is:

```text
[request_name, arguments]
```

A completion payload is one of:

```text
["ok", value]
["throw", error_observation]
```

The completion must echo the outstanding operation ID and sequence. Wrong operation IDs, stale sequences, duplicate completions, and stepping a terminal packet are boundary failures.

| Request | Arguments | Successful completion |
|---|---|---|
| `accounts` | engine string | ordered `{id,name}` array |
| `ambient_env` | `"CODEX_HOME"` | string or undefined |
| `homedir` | empty tuple | string |
| `cwd` | empty tuple | string |
| `realpath` | path | resolved path string |
| `stat` | path | `{dev, ino, devText, inoText}` |
| `is_temporary` | session ID | boolean |
| `invoke_engine` | `{route, operation, source, folder, thread?, optionsRef}` | engine success described below |

The core requests observations at the reference evaluation points. It must not eagerly query accounts or temporary status, and must not cache repeated stat, environment, or predicate observations in a way that changes the reference behavior.

The host performs no canonicalization, account skipping, title clipping, or bound-key construction before returning a completion.

### 4.4 Engine success packets

A successful list completion is:

```text
{
  rows: [
    { rowRef, title },
    ...
  ]
}
```

The host retains the complete original rows. The core’s final list recipe is an ordered array of:

```text
["keep", rowRef]
["copy_title", rowRef, newTitle]
```

The host creates a new result array, reuses a row for `keep`, and applies a shallow enumerable-own-property copy with the replacement title for `copy_title`. This preserves all metadata without inventing a common projection.

A successful read completion is:

```text
{ recordRef }
```

The core returns that reference unchanged.

The underlying row/record types remain T:66–95. The common layer must not drop `entrypoint`, `origin`, `maybeOpen`, `lastPlan`, optional timestamps, or any additional enumerable metadata merely because it does not inspect them.

### 4.5 Error observations and terminal errors

An error observation is:

```text
{
  valueRef,
  isEngineImportError,
  isError,
  name,
  message,
  code
}
```

`name`, `message`, and `code` preserve absent values. The host supplies the results of runtime type/property observations. It does not perform the `rethrow` branch selection.

The two instance flags refer to the actual oracle/runtime class relationships, not string-name tests. For ordinary Error objects, `message` is a string. Non-Error values retain an opaque reference even when they are primitives.

Terminal payloads are:

```text
["ok", value]

["throw_existing", valueRef]

["throw_import", code, detailOrUndefined, causeRef]

["throw_account_wire", wireKey, diagnosticText]

["throw_plain_error", diagnosticText]

["boundary_error", boundaryCode]
```

`throw_import` constructs the Error using T:39–45. The host retains `causeRef` exactly, including a reference to undefined.

Boundary codes are:

```text
InvalidPacket
InputTooLarge
InternalInvariant
```

They are not `EngineImportErrorCode` values. `InternalInvariant` is always an implementation failure.

Account-helper errors are swallowed only when the core is executing the corresponding account body in `import_sources`; the same direct helper call exposes its error.

## 5. Existing engine forwarding

| Engine | Existing public surface | Integration contract |
|---|---|---|
| Claude | `head_packet`, `tail_packet`, `window_packet` | Retain existing file/list/read orchestration and packet decoding |
| Qwen | Same three packet roots plus `valid_thread`, `project_name`, `title_bytes` | Retain existing host orchestration |
| Codex | `codex_list`, `codex_read` | Forward to direct effectful functions with existing process dependencies |
| Grok | `grok_begin`, `grok_step` | Drive the existing protocol to completion |
| OpenCode | `opencode_begin`, `opencode_step` | Drive the existing protocol to completion |

Evidence:

- `bench/l3/claude/root.pruv:1–10,25–40`;
- `bench/l3/qwen/root.pruv:15–53`;
- `bench/l3/qwen/host.rs:65–99`;
- `bench/l3/codex/root.pruv:25–31,97–106`;
- `bench/l3/grok/root.pruv:273–325`;
- `bench/l3/opencode/root.pruv:333–375`.

The new common host is a forwarding/observation adapter. The existing engine hosts are not all pure OS adapters: Claude explicitly retains path and exception work, and Qwen retains directory filtering, sorting, folder comparison, list limiting, and activity estimation. Their existing trust boundaries remain visible.

The forwarding adapter may reshape packets and retain references. It must not:

- choose a different engine;
- try a second engine after failure;
- prepare or authenticate an account;
- trim a title;
- remove list rows;
- alter successful records;
- translate an engine error before the common classifier sees it;
- claim unsupported existing-host behavior as implemented.

The accepted scope does not require package consolidation or ABI unification.

## 6. Host contract and oracle

### 6.1 Host responsibilities

The host must:

1. Encode inputs losslessly.
2. Supply observations only when requested.
3. Preserve callback array order and session order.
4. Report distinct filesystem outcomes, especially `ENOENT` versus other errors.
5. Invoke the requested engine exactly once with the original arguments.
6. Retain object/value references needed for identity and causes.
7. Reconstruct the prescribed list or error mechanically.
8. Reject malformed packets explicitly.
9. Never read auth/token material.

Filesystem observations are trusted facts. Their interpretation belongs to Pruv. Existing engine-internal decisions remain trusted at the boundaries described in section 5.

### 6.2 Oracle execution

The oracle executes the pinned functions, not reimplementations of them.

For common-core cases:

- Replace engine imports with deterministic fake functions that record route and arguments and return scripted rows, records, or thrown values.
- Supply scripted account callbacks and temporary predicates.
- Intercept only environmental and filesystem observations needed by C.
- Execute the original A/C/E/B function bodies.
- Do not stub `accountHome`, `codexAccountFolder`, `isHostCodexHome`, `canonicalPath`, `sameFile`, source selection, dispatch, clipping, or bound indexing with expected answers.
- Use inert wire-error support if needed to load account helpers; preserve the wire key and Error category.
- Never invoke account preparation or auth helpers.

The private `rethrow` helper is exercised through the exported list/read wrappers.

Real-engine integration probes use local deterministic fixtures or existing fake-child machinery, not live accounts or network services. Failure of an existing host to support an operation is a reported integration limit, not a reason to replace that engine with an all-purpose stub in the integration gate.

### 6.3 Comparison policies

| Policy | Classification and rule | Justification |
|---|---|---|
| P0 | Exact common semantics | E:34–93 and B:15–44 are the ported contract |
| P1 | Exact UTF-16 strings | E:81 can produce lone surrogates; UTF-8 replacement would change the result |
| P2 | Exact branch/order/observation trace | C:49–88, E:38–48, and B:30–41 contain observable short circuits and ordering |
| P3 | Exact synthetic error behavior | E:64–66 and T:39–45 specify class, code, detail, cause, and pass-through |
| P4 | Allowed representation normalization only | Numeric opaque IDs and fixture-root prefixes are transport/test artifacts |
| P5 | Allowed classified native diagnostic difference | Native OS/runtime prose may vary; operation, errno, classification, cause relationship, and resulting behavior must still agree |
| P6 | Allowed explicit resource-bound difference | The new finite packet boundary may reject oversized input that JS could hold; expected result is exactly `InputTooLarge` |
| P7 | Existing engine deviations remain separately classified | Existing pieces are retained; their documented parser/text/time policies are not new common-core allowances |

P4 normalizes opaque IDs by first occurrence and preserves alias relationships. Root normalization may replace only the assigned fixture root, not separators, case, suffixes, source IDs, or arbitrary string substrings.

P5 never permits a synthetic error message difference. Synthetic messages, including empty strings and surrogate-containing strings, compare exactly. Account wire keys and the deterministic pinned account-error text compare exactly.

P6 applies only to a recorded limit violation. It never permits truncation, partial success, account skipping, or `InternalInvariant`.

For P7:

- The 2,000 common-core Q cases inject identical engine results on both sides; therefore **no engine policy can excuse a common-core mismatch**.
- Forwarding integration fixtures use inputs in the existing engines’ common exact domain.
- Any additional engine-policy demonstration must name the engine, existing policy number, triggering input, and precise affected field. It is reported separately.
- An engine-local 120-unit title deviation or plan-text deviation cannot justify changing the new common 200-unit cut.

Existing policy evidence: `docs/QWEN_IMPORT_SPEC.md:73–81`; `docs/CODEX_IMPORT_SPEC.md:144–165`; `docs/GROK_IMPORT_SPEC.md:225–306`; `docs/OPENCODE_IMPORT_SPEC.md:283–319`.

Compare:

- ordered source arrays and every present field;
- lookup index/absence;
- selected route and exact forwarded arguments;
- list order, row identity recipe, and UTF-16 titles;
- unchanged read reference;
- error branch, code, message, and cause identity;
- thread sequence;
- predicate call sequence;
- ordered bound-map entries;
- canonicalization and stat request sequence.

Do not sort outputs before comparison. A mismatch is not a classified difference merely because its final JSON looks similar.

## 7. Fixed fixtures: 150 named cases

Each group contains ten individually named cases. These are normative fixture requirements, not claims that fixture files already exist.

Default fixtures use stable homedir `/home/test`, cwd `/work`, data directory `/data`, valid IDs `acc_0123456789abcdef` and `acc_fedcba9876543210`, and distinct sentinel object references.

### 7.1 Source environment basics — S01–S10

Evidence: E:34–53; C:41–42.

| ID and name | Required observation |
|---|---|
| S01 `homes-default-order` | No accounts/overrides: five homes in Claude, Codex, Grok, OpenCode, Qwen order |
| S02 `claude-trimmed-override` | Surrounding ECMAScript whitespace removed |
| S03 `claude-empty-fallback` | Empty override selects `.claude` |
| S04 `claude-whitespace-fallback` | Whitespace-only override selects `.claude` |
| S05 `claude-relative-retained` | Relative nonempty override remains relative |
| S06 `grok-trimmed-override` | Trimmed Grok home used |
| S07 `grok-empty-fallback` | Empty Grok home selects `.grok` |
| S08 `codex-ambient-wins` | Ambient value used despite conflicting `deps.env.CODEX_HOME` |
| S09 `codex-ambient-blank` | Blank ambient value falls back even with nonblank deps value |
| S10 `deps-home-not-homedir` | `deps.env.HOME` does not control default homes |

### 7.2 Additional environment cases — E01–E10

Evidence: O:39–41; Q:16–22; E:52.

| ID and name | Required observation |
|---|---|
| E01 `xdg-absolute-join` | Absolute XDG root receives `opencode` suffix |
| E02 `xdg-relative-fallback` | Relative XDG root ignored |
| E03 `xdg-blank-fallback` | Empty/trimmed-empty root uses homedir fallback |
| E04 `xdg-root-and-dot-normalization` | Join behavior preserved for `/` and dot segments |
| E05 `qwen-runtime-precedence` | Runtime override wins over global override |
| E06 `qwen-runtime-blank-global` | Blank runtime uses trimmed global |
| E07 `qwen-both-blank-default` | Both blank use `.qwen` |
| E08 `qwen-relative-verbatim` | Nonempty relative runtime is not resolved |
| E09 `trim-unicode-membership` | FEFF/NBSP trimmed; U+0085/U+200B retained |
| E10 `qwen-eager-home-observation` | Homedir observed even with runtime override |

### 7.3 Accounts — A01–A10

Evidence: A:10–13,54–57; E:38–48.

| ID and name | Required observation |
|---|---|
| A01 `account-valid-lower-hex` | Valid ID joined correctly |
| A02 `account-uppercase-rejected` | Uppercase hexadecimal letter rejected |
| A03 `account-length-boundaries` | 15/17 hex digits rejected; 16 accepted |
| A04 `account-newline-and-path-rejected` | Trailing newline, slash, and traversal forms rejected |
| A05 `account-name-verbatim` | Empty and padded names preserved |
| A06 `account-callback-order` | Claude callback precedes Codex callback |
| A07 `account-array-order-duplicates` | Array order and duplicate IDs preserved |
| A08 `account-bad-middle-skipped` | Invalid middle account omitted; later account retained |
| A09 `account-callback-throw-escapes` | Callback failure aborts operation; no partial output |
| A10 `account-no-eight-cap` | Nine valid accounts all retained |

### 7.4 Canonical path behavior — C01–C10

Evidence: C:49–70.

| ID and name | Required observation |
|---|---|
| C01 `canonical-existing-realpath` | Successful realpath returned directly |
| C02 `canonical-one-missing-leaf` | Parent realpath plus leaf |
| C03 `canonical-multiple-missing-components` | Missing suffix order preserved |
| C04 `canonical-enoent-skips-stat` | No stat at ENOENT location |
| C05 `canonical-eacces-stat-identity` | EACCES followed by stat produces device/inode identity |
| C06 `canonical-eloop-stat-identity` | ELOOP uses same non-ENOENT branch |
| C07 `canonical-stat-failure-climbs` | Failed fallback stat continues upward |
| C08 `canonical-root-exhaustion` | Root exhaustion returns resolved original path |
| C09 `canonical-relative-cwd` | Relative input resolved against observed cwd |
| C10 `canonical-identity-suffix` | Missing suffix appended to identity string in original order |

### 7.5 Codex host-home protection — H01–H10

Evidence: C:73–97.

| ID and name | Required observation |
|---|---|
| H01 `codex-home-exact-mismatch` | Supplied home mismatch fails before filesystem requests |
| H02 `codex-canonical-ambient-alias` | Canonical equality with ambient home rejects |
| H03 `codex-canonical-dot-alias` | Equality with default `.codex` rejects despite ambient override |
| H04 `codex-canonical-all-three` | Dot canonicalization still occurs when target equals host |
| H05 `codex-samefile-ambient` | Different canonical strings but equal dev/ino reject |
| H06 `codex-samefile-dot` | Second same-file comparison rejects |
| H07 `codex-samefile-short-circuit` | Successful first same-file call suppresses second |
| H08 `codex-samefile-stat-failure` | Stat failure returns false and permits later comparison |
| H09 `codex-distinct-accepted` | Distinct canonical and file identities accepted |
| H10 `codex-account-failure-continues` | Rejected account skipped; later account and final homes retained |

### 7.6 Source lookup — F01–F10

Evidence: E:56–59.

| ID and name | Required observation |
|---|---|
| F01 `find-empty-array` | Undefined |
| F02 `find-first-entry` | Index 0 |
| F03 `find-last-entry` | Last matching position |
| F04 `find-missing` | Undefined, no error |
| F05 `find-duplicate-first-wins` | First duplicate selected |
| F06 `find-case-sensitive` | Case variant does not match |
| F07 `find-no-trim` | Padded ID differs |
| F08 `find-empty-id` | Empty ID can match |
| F09 `find-colons-verbatim` | Multi-colon ID compared as one string |
| F10 `find-surrogate-identity` | Lone surrogate differs from U+FFFD |

### 7.7 Dispatch — D01–D10

Evidence: E:73–93.

Each fixture checks both list and read.

| ID and name | Required observation |
|---|---|
| D01 `dispatch-claude` | Claude selected |
| D02 `dispatch-codex` | Codex selected |
| D03 `dispatch-opencode` | OpenCode selected |
| D04 `dispatch-qwen` | Qwen selected |
| D05 `dispatch-grok` | Grok selected |
| D06 `dispatch-unknown-falls-grok` | Unknown engine selects Grok |
| D07 `dispatch-empty-falls-grok` | Empty engine selects Grok |
| D08 `dispatch-arguments-identical` | Source, folder, thread, and options unchanged |
| D09 `dispatch-exactly-once` | No fallback retry after rejection |
| D10 `dispatch-read-record-identity` | Successful read reference unchanged |

### 7.8 Titles — T01–T10

Evidence: E:61–62,80–82.

| ID and name | Required observation |
|---|---|
| T01 `title-empty` | Empty title and row identity retained |
| T02 `title-199` | Unchanged |
| T03 `title-200` | Unchanged and same row |
| T04 `title-201` | First 200 units and copied row |
| T05 `title-many-kilobytes` | Exactly first 200 units |
| T06 `title-pair-fits` | 198 ASCII plus supplementary character retained |
| T07 `title-pair-split` | 199 ASCII plus supplementary character retains high surrogate |
| T08 `title-lone-surrogates` | Existing retained surrogates preserved |
| T09 `title-whitespace-not-cleaned` | Newlines and padding retained |
| T10 `title-metadata-and-order` | All other enumerable fields and list order preserved |

### 7.9 Errors — R01–R10

Evidence: E:64–67; T:35–46.

| ID and name | Required observation |
|---|---|
| R01 `rethrow-import-all-codes` | Every declared import code passes through by identity |
| R02 `rethrow-import-renamed` | Actual import instance still passes through after name change |
| R03 `rethrow-abort-error` | Error named AbortError passes through |
| R04 `rethrow-abort-object` | Plain object named AbortError passes through |
| R05 `rethrow-fake-import-name` | Plain object named EngineImportError wraps |
| R06 `rethrow-error-detail` | Nonempty Error message copied exactly |
| R07 `rethrow-empty-error-detail` | Result message is `read-failed` |
| R08 `rethrow-null-undefined` | Both wrap without detail; distinct causes retained |
| R09 `rethrow-primitive-values` | String/number/boolean wrap without stringification |
| R10 `rethrow-name-near-misses` | TimeoutError and lowercase aborterror wrap |

### 7.10 Origin and terminal threads — U01–U10

Evidence: B:15–23.

| ID and name | Required observation |
|---|---|
| U01 `threads-empty-session` | No candidates |
| U02 `threads-origin-only` | Origin pair emitted |
| U03 `threads-forks-order` | Origin then forks in order |
| U04 `threads-nullish-forks` | Missing/null forks add nothing |
| U05 `threads-empty-origin-fork` | Empty origin/fork strings retained |
| U06 `threads-duplicate-forks` | Duplicates retained |
| U07 `threads-terminal-default` | Missing engine uses Codex |
| U08 `threads-terminal-null-engine` | Null engine uses Codex |
| U09 `threads-terminal-empty-engine` | Empty engine retained |
| U10 `threads-terminal-truthiness` | Empty cliSession omitted; whitespace included; started ignored |

### 7.11 Engine slots — V01–V10

Evidence: B:24–25.

| ID and name | Required observation |
|---|---|
| V01 `slot-no-colon` | Whole slot is engine |
| V02 `slot-one-colon` | Prefix before first colon |
| V03 `slot-many-colons` | Only first prefix used |
| V04 `slot-leading-colon` | Empty engine |
| V05 `slot-empty-name` | Empty engine |
| V06 `slot-unknown-engine` | Unknown prefix retained |
| V07 `slot-empty-thread` | Empty thread omitted |
| V08 `slot-nullish-bound` | Missing/null bound omitted |
| V09 `slot-object-entry-order` | Integer keys numeric-first; remaining keys insertion-ordered |
| V10 `slot-after-origin-terminal` | Slots follow all origin/fork/terminal candidates |

### 7.12 Temporary exclusion — X01–X10

Evidence: B:30–38.

| ID and name | Required observation |
|---|---|
| X01 `temporary-literal-true` | Excluded without callback |
| X02 `temporary-missing-false-callback` | Callback false retains |
| X03 `temporary-missing-true-callback` | Callback true excludes |
| X04 `temporary-explicit-false` | Callback still invoked |
| X05 `temporary-null` | Callback still invoked |
| X06 `temporary-number-one` | Numeric 1 is not literal true |
| X07 `temporary-string-true` | String `"true"` is not literal true |
| X08 `temporary-callback-throws` | Original failure escapes |
| X09 `temporary-duplicate-id-calls` | One call per non-literal-true occurrence |
| X10 `temporary-excludes-before-threads` | Excluded session contributes no candidates |

### 7.13 Bound index — B01–B10

Evidence: B:35–44; R:133–138 for B10’s caller comparison.

| ID and name | Required observation |
|---|---|
| B01 `bound-empty` | Empty ordered map |
| B02 `bound-one` | One exact key/value |
| B03 `bound-duplicate-first-wins` | Earlier eligible session retained |
| B04 `bound-temporary-first-skipped` | Later eligible session wins when earlier one excluded |
| B05 `bound-within-session-duplicates` | One entry; no reordering |
| B06 `bound-colon-collision` | Unescaped composite keys collide |
| B07 `bound-account-source-ignored` | Same engine/thread collides across source/account |
| B08 `bound-empty-id-still-present` | Empty stored ID blocks overwrite |
| B09 `bound-insertion-order` | First insertion order preserved |
| B10 `bound-vs-title-map-winner` | Bound map first-wins; separate caller title Map last-wins |

### 7.14 Packet boundary — P01–P10

These test the new interface in section 4, rather than asserting additional TS behavior.

| ID and name | Required observation |
|---|---|
| P01 `packet-empty-vs-undefined` | Distinct values survive round trip |
| P02 `packet-null-vs-undefined` | Distinct values and causes survive |
| P03 `packet-surrogate-roundtrip` | All UTF-16 units preserved |
| P04 `packet-object-extra-fields` | Uninspected row fields survive reconstruction |
| P05 `packet-number-bits` | Finite values and negative zero preserved |
| P06 `packet-wrong-version-length` | InvalidPacket |
| P07 `packet-wrong-operation-sequence` | InvalidPacket |
| P08 `packet-duplicate-completion` | InvalidPacket |
| P09 `packet-terminal-reentry` | InvalidPacket |
| P10 `packet-size-boundary` | Exact limit accepted; one word over gives InputTooLarge |

### 7.15 Forwarding integration — I01–I10

Evidence: public interfaces cited in section 5.

| ID and name | Required observation |
|---|---|
| I01 `integration-claude-list` | Existing adapter result receives only common title processing |
| I02 `integration-claude-read` | Existing record forwarded unchanged |
| I03 `integration-qwen-list` | Existing list order/metadata preserved |
| I04 `integration-qwen-read` | Existing record forwarded unchanged |
| I05 `integration-codex-list` | Correct direct function and arguments |
| I06 `integration-codex-read` | Correct direct function and unchanged record |
| I07 `integration-grok-list` | Existing begin/step driver used |
| I08 `integration-grok-read` | Existing read result and errors forwarded |
| I09 `integration-opencode-list` | Existing begin/step driver used |
| I10 `integration-opencode-read` | Existing read result and errors forwarded |

No integration fixture may prepare a real account, read tokens, or use live external services.

## 8. Q generator

### 8.1 Seed and count

Run exactly **2,000 generated cases**, in addition to the 150 fixed fixtures.

Seed: **20261118**.

Use unsigned 32-bit xorshift:

```text
x ^= x << 13
x ^= x >>> 17
x ^= x << 5
```

Mask to 32 bits after each update. Initialize `x = 20261118`.

For a uniform choice from `0…n-1`, reject random words at or above `floor(2^32/n) * n`, then use remainder modulo `n`.

Construct the exact family multiset below, then Fisher–Yates shuffle it with this PRNG. Generate each case from the same continuing PRNG state. This gives exact family counts rather than approximate probabilities.

| Family | Weight | Cases |
|---|---:|---:|
| G1 source environments and account arrays | 20% | 400 |
| G2 canonical paths and Codex aliases | 15% | 300 |
| G3 source lookup and dispatch | 10% | 200 |
| G4 list titles and result preservation | 15% | 300 |
| G5 error classification | 15% | 300 |
| G6 session thread extraction | 10% | 200 |
| G7 temporary exclusion and bound collisions | 10% | 200 |
| G8 packet boundaries and continuation discipline | 5% | 100 |
| **Total** | **100%** | **2,000** |

### 8.2 Family generation

**G1 — Environments and accounts**

Vary each relevant environment slot among absent, empty, whitespace-only, padded absolute, padded relative, root, dot-segment path, and non-ASCII text. Sample whitespace from the exact trim set plus U+0085/U+200B controls.

Vary ambient Codex environment independently from `deps.env`. Generate 0–12 accounts per engine, with valid/invalid/duplicate IDs, arbitrary names, and callback failures. Preserve generated array order.

**G2 — Canonical paths**

Generate 0–5 missing suffix components. Script realpath outcomes from success, ENOENT, EACCES, and ELOOP; script fallback stat success/failure and dev/ino equality independently.

Generate canonical aliases to ambient/default homes, same-file aliases with unequal canonical strings, root exhaustion, and distinct paths. Include observation sequences in which repeated calls return different values, so forbidden caching is detectable.

**G3 — Lookup and dispatch**

Generate arrays of 0–32 sources, with duplicate IDs at different positions. Vary query equality, case, padding, empty strings, colons, and surrogate units.

Choose the five named routes plus unknown/empty engines. Alternate list and read. Use distinct argument/reference sentinels and count invocations.

**G4 — Titles**

Generate 0–64 rows. Choose title lengths around 0, 1, 198, 199, 200, 201, 202, and a long-tail range up to 16,384 units.

Mix ASCII, BMP text, combining marks, supplementary pairs, lone surrogates, and whitespace. Bias half of nonempty cases toward a code-unit boundary at 199/200. Add optional and extra metadata fields. Check row-copy recipes as well as values.

The common wrapper does not impose a 50-row cap on injected results.

**G5 — Errors**

Choose actual import errors, ordinary Errors, AbortError-named Errors, AbortError-named plain objects, misleading names, and primitive thrown values.

Vary messages among absent where applicable, empty, whitespace, ordinary text, and surrogate-containing strings. Alternate synchronous throw and rejected completion. Exercise both list and read.

**G6 — Thread extraction**

Generate optional origins, 0–12 forks, optional terminals, and 0–12 slots. Include empty strings, repeated strings, unknown engines, nullish optional fields, multiple colons, and integer-index slot keys.

Check the complete ordered candidate sequence before any map insertion.

**G7 — Temporary and bound behavior**

Generate 0–32 sessions. Vary literal flags and scripted predicate results. Include repeated IDs and repeated engine/thread pairs across sessions, deliberate colon collisions, empty session IDs, and excluded earlier writers.

Generate predicate failures in a minority of cases. Compare the exact callback prefix before failure.

**G8 — Packet discipline**

Generate valid packet round trips and one controlled corruption per invalid case: version, declared length, tag, string unit range, operation ID, sequence, duplicate completion, or terminal reuse.

Resource-limit cases are separate explicit boundary probes and must produce the declared boundary result. They are not interpreted as TS mismatches.

### 8.3 Execution and reporting

Run the common entry on:

```text
py/rust
py/llvm
rs/rust
rs/llvm
```

All four results and normalized traces must agree. Compare each with the pinned oracle under section 6.

Persist the seed, case number, family, generated input, observation script, oracle output, Pruv output, and first differing field/request. Shrinking may remove rows, sessions, accounts, suffix components, and string units, but must preserve the triggering mismatch.

Report separately:

- exact matches;
- permitted P5 native-prose differences;
- explicit P6 resource-limit cases;
- existing-engine policy observations;
- excluded-domain observations;
- unexplained failures.

The required result is zero unexplained failures. A crash, watchdog kill, missing engine result, or `InternalInvariant` is not a passing timeout or permitted difference.

## 9. Mutants

Build each mutant separately on `rs/rust`. Each must be killed by its named fixture or rejected by verification. A fixture that merely crashes for an unrelated reason is not a valid kill.

| ID | Mutant | Required killing fixture and difference |
|---|---|---|
| M01 | `ascii-only-trim` | E09: FEFF/NBSP handling differs |
| M02 | `codex-use-deps-env` | S08: wrong Codex home selected |
| M03 | `accept-relative-xdg` | E02: relative XDG incorrectly used |
| M04 | `account-sort-or-deduplicate` | A07: source order/count differs |
| M05 | `account-failure-aborts-all` | A08: later valid account/final homes lost |
| M06 | `enoent-also-stats` | C04: forbidden extra stat request |
| M07 | `codex-ignore-default-dot-home` | H03: protected default home incorrectly accepted |
| M08 | `find-last-match` | F05: wrong duplicate index |
| M09 | `unknown-engine-default-codex` | D06: wrong engine route |
| M10 | `title-codepoint-cut` | T07: wrong length or missing high surrogate |
| M11 | `abort-requires-error-instance` | R04: AbortError plain object incorrectly wrapped |
| M12 | `stringify-non-error-detail` | R09: invented diagnostic detail |
| M13 | `terminal-engine-or-default` | U09: empty engine incorrectly becomes Codex |
| M14 | `slot-last-colon-prefix` | V03: wrong engine extracted |
| M15 | `temporary-truthiness-or-eager-callback` | X06 and X01: wrong exclusion or forbidden callback |
| M16 | `bound-last-write-wins` | B03: later session incorrectly overwrites earlier |

All 16 are required. Mutation testing does not authorize edits to the pinned references.

## 10. Limits and acceptance

### 10.1 Common-layer limits

The new packet transport accepts at most **16,777,216 i64 words per packet**, including its header. Value nesting is limited to **32** levels.

These are implementation bounds, not Lair import limits. Check sizes with overflow-safe arithmetic before allocating or indexing. Exceeding a bound returns `InputTooLarge`; it must not return a truncated result.

No additional account-count, source-count, session-count, fork-count, or list-count cap is introduced beyond the packet bound.

The existing engine limits remain owned by their engine implementations:

| Limit | Pinned value |
|---|---:|
| File window | 8 MiB |
| Line | 1 MiB |
| Turns | 200 |
| Reply | 32 KiB |
| Total imported text | 2 MiB |
| List | 50 |
| Head/tail edge | 64 KiB |
| Parallel work | 4 |
| RPC budget | 20,000 ms |

Evidence: T:13–33.

The common list wrapper’s 200-unit title limit is separate from these limits. It must not apply the engine list cap a second time.
Evidence: E:61–81.

### 10.2 Known limits

- Windows path and process behavior is not claimed.
- OS paths must be representable by the existing adapters. Pure string operations still preserve arbitrary UTF-16 units.
- Live filesystem races are not proved away. Scripted observation sequences test the specified decision algorithm.
- Session iteration uses a finite ordered snapshot; arbitrary side-effectful iterators are excluded.
- Real external cancellation remains limited by existing engine pieces. The common classifier nevertheless handles an observed AbortError exactly.
- The existing Claude/Qwen hosts retain policy outside verified Pruv.
- The new entry does not prove engine transcript parsing, filesystem truth, Node built-ins, or host object identity management.
- Existing-engine policy deviations remain separate; they cannot hide common-entry errors.

### 10.3 Acceptance criteria

Implementation acceptance requires:

1. The specified common operations and packet boundary.
2. Verification of reachable contracts on all four compiler/backend pairs.
3. All 150 fixed fixtures.
4. The 2,000-case seeded Q run.
5. All 16 mutants killed or verification-rejected.
6. Five-engine forwarding probes without changing existing engine semantics.
7. No auth/token access, account preparation, or real external service dependency.
8. Zero unexplained result or observation-trace differences.

Proof targets are packet validity, safe indexing, bounded loops, valid continuation matching, and preservation of the declared structural invariants. TS semantic equivalence is established by the specified differential tests, not claimed as a complete mechanized theorem.

Only L3-8 and affected forwarding checks are required. Do not run unrelated full-repository tests or rebuild existing engine internals merely to satisfy this specification.

## 11. Verification status of this document

Confirmed during specification work:

- The relevant pinned entry, bound, account, Codex-home, caller, type, and directory-helper code was read.
- The five engine public interfaces were inspected.
- The entry/bound/account/Codex-home/Qwen hashes matched the manifest.
- The older Qwen reference was byte-identical to the new pinned Qwen copy.
- Read-only Node probes confirmed the account-ID trailing-line-terminator rejection, UTF-16 split-surrogate example, and slot-splitting examples.
- The first-write bound-index behavior and separate last-write title-map behavior were confirmed directly.

Not performed:

- Entry implementation or file creation.
- Compilation, proof execution, fixture execution, Q execution, mutation execution, or real-engine integration execution.

Those are future implementation acceptance steps, not missing source evidence for this design. No files outside Pruv, authentication material, or live Lair state were inspected.
