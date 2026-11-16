# Codex conversation import

Status: done

## Goal

Port Lair's Codex conversation import (commit 744d14af, `src/backend/engine-import/codex.ts`) to Pruv without
language changes. Unlike Claude and Qwen, Codex import parses no files. It spawns `codex app-server --stdio` for the
request and speaks newline-delimited JSON (Codex shape: no `jsonrpc` field). Pruv owns the whole conversation:
spawn, framing, request ids, response matching, error mapping, paging, list filtering, turn classification,
acquisition budgets, the turn collector, last plan and the result packet. The Rust host only supplies binary,
extra arguments, cwd, folder, thread and timeout, and renders the packet as JSON.

Sources live in `bench/l3/codex`. Modules that a second piece now needs move into `bench/l3/pkgs` as G4b packages
(no copies, no `--lib`); qwen and mcp switch to those packages in the same batch.

## Scope

| Reference | Implementation |
|---|---|
| withRpcChild (non-win32, jsonrpc false) | rpc.pruv on the `transport` package (G10 event loop) |
| withCodex: initialize, initialized | rpc.pruv::open |
| listCodex | list.pruv |
| readCodex, readTurn, itemText | read.pruv, item.pruv |
| codexPlan, fit (plan.ts) | plan.pruv |
| TurnCollector | `turns` package (moved from qwen) |
| isoOf, oneLine, validThread, folderKey | `time`, `text`, `names` packages; folder.pruv |
| makeCodexRunDeps | host (binary, args) — account preparation is out of scope |

Out of scope: Windows (`process.platform === 'win32'` branches: paging with folder filter, case-insensitive
folderKey, taskkill), `detached` process groups, AbortSignal, account home preparation and environment building.
The child inherits the runner's environment.

## Protocol (what TS does, and Pruv must do)

- Spawn `binary app-server --stdio ...extra` with cwd from the host. Spawn failure → `engine-missing`.
- Request ids start at 1 and increase by one per request. Codex sends at most 42 requests (initialize, thread/list
  or thread/read, up to 40 thread/turns/list), so ids are small multi-digit decimals.
- `initialize` params: `{"clientInfo":{"name":"lair","title":"Lair","version":"0.0.1"},"capabilities":{"experimentalApi":true,"requestAttestation":false}}`,
  then notification `{"method":"initialized","params":{}}`. An initialize error response → `read-failed`.
- Incoming line handling: split stdout on LF, trim ECMAScript whitespace, skip blank lines and lines that do not
  parse to a JSON object. If `method` is a string: when `id` is present, reply
  `{"id":<same id>,"error":{"code":-32601,"message":"not handled"}}`; otherwise ignore (notification). Otherwise
  the line is a response only when `id` is a number equal to the outstanding request id; others are ignored.
  If `error` is an object the response is an error (`message` string sliced to 300 UTF-16 units, else
  `rejected`; `code` kept); otherwise `result` (possibly absent) is the value.
- Failures while waiting: overall deadline → `timeout`; child stdout EOF/exit → `read-failed`; unterminated line
  over the cap → `read-failed`. The first failure wins. After the work returns or fails, close stdin and stop the
  child (G10 close; P7).
- stderr is drained and ignored.

### listCodex (non-win32)

One `thread/list` request: `{"cwd":[folder],"limit":50,"sortKey":"updated_at","useStateDbOnly":true,"sourceKinds":["cli","vscode","exec","appServer"]}`.
No paging on non-win32 (the loop breaks after the first page). An error response → `codex-unsupported`.
Page = `result.data` if an array, else `result.threads` if an array, else empty. Take the first 50 raw entries, then
for each: skip non-objects and entries whose `id` is not a string passing validThread
(`^[A-Za-z0-9][A-Za-z0-9_-]{0,127}$`). Row fields:

- thread: id
- title: `oneLine(trim(name) || trim(preview) || "", 120)` where name/preview count only when strings
- updatedAt: `isoOf(updatedAt)`, omitted when absent
- path: string `path`, omitted when absent or empty
- origin: only when `originator` is a string: `lair` if it starts with `lair`, else `other`

### readCodex

1. `validThread(thread)` fails → `not-found` without spawning.
2. `thread/read {"threadId":thread,"includeTurns":false}`. A transport failure while waiting propagates (timeout,
   read-failed). Any error response → `not-found`. meta = `result.thread` if an object, else `{}`.
3. meta.cwd must be a string and `folderKey(cwd) == folderKey(folder)`, where folderKey strips trailing `/` but keeps
   an all-slash string unchanged. Comparison is exact UTF-16 unit equality. Otherwise → `not-found`.
4. Up to 40 pages of `thread/turns/list {"threadId":thread,"limit":20,"sortDirection":"desc","itemsView":"summary"}`
   plus `"cursor":c` when a cursor is held. Error responses: code `-32601` → `codex-unsupported`; message matching
   `/not found|no rollout|unknown thread/i` → `not-found`; otherwise `read-failed`.
5. For each object in `result.data` (array, else empty), classify items (non-objects dropped):
   - `userMessage`: itemText; prompts with nonblank text are kept and joined by `\n\n`.
   - `agentMessage`: phase `commentary` gives no text; nonblank itemText is a reply.
   - `plan`: string nonblank `text` sets planText; the last one in the turn wins. Not a tool.
   - `reasoning`: ignored. Any other type (including absent) counts one tool.
   - itemText: `text` if a string; else `content` array parts' string `text` fields, empty ones dropped, joined
     by `\n\n`; else empty.
   - createdAt = isoOf(startedAt), completedAt = isoOf(completedAt).
6. Last plan: the first turn met (newest-first) with planText sets lastPlan = codexPlan(planText,
   completedAt ?? createdAt), and later turns never replace it, even when codexPlan returns none. This happens
   before the empty-turn skip and before the budget check.
7. A turn with empty prompt and no replies is skipped. Otherwise size = bytes(prompt) +
   min(sum of reply bytes without separators, 32768). If 200 turns are already held or held bytes + size >
   2097152, set more and stop. Else hold it.
8. Paging: stop when full; stop when nextCursor is not a nonempty string or the page's data is empty; else take the
   cursor. Reaching the 40th page with a further cursor sets more.
9. Feed held turns oldest-first into the collector: begin(prompt, createdAt); answer(reply, completedAt) for each
   reply; tool(count). omittedBefore = more || collector.trimmed.
10. model = meta.model if a string, else meta.model.id if a string; kept only when trim is nonempty and the
    untrimmed length is ≤ 200 UTF-16 units; the trimmed value is used. title = oneLine(trim(meta.name), 120) when
    the trimmed name is nonempty.

codexPlan: split on `\r?\n`; per line trim, remove one leading `-`, `*` or `\d+.` followed by ECMAScript whitespace,
trim again; drop empty lines; slice each to 200 UTF-16 units; keep at most 30, `more` = dropped count (omitted when
zero); none when no item remains. Items have no status.

Held turns are stored with their reply already folded exactly as TurnCollector.answer would fold it (per-turn
state is independent), so storage stays bounded: prompts are covered by the 2 MiB budget and folded replies by
32768 bytes each. The result must equal feeding the original texts in order. The `turns` package gains
`tool_n` (add a count) and whatever minimal entry point replays a folded reply including its clipped flag;
existing functions keep their behavior for qwen.

## Boundary representation

Root functions (all `! async, net, proc, alloc`), with `argv` as the mcp NUL-terminated argument blob of the
extra arguments (Pruv prepends `app-server`, `--stdio`):

- `codex_list(binary: str, argv: str, cwd: str, folder: str, timeout_ms: i64) -> [i64]`
- `codex_read(binary: str, argv: str, cwd: str, folder: str, thread: str, timeout_ms: i64) -> [i64]`

Success: `[0,16,row_words,byte_len,...meta16,...rows,...text_bytes]`; failure: `[code,stage,detail_len,...detail]`.

| Meta | Meaning |
|---|---|
| 0 | Schema version 1 |
| 1 | Mode 0 list, 1 read |
| 2 | omittedBefore 0/1 (list 0) |
| 3 | Row count |
| 4–5 | Read title ref |
| 6–7 | Read model ref |
| 8–9 | lastPlan at ref |
| 10 | Row width (list 9, read 10) |
| 11 | lastPlan item count, −1 when lastPlan is absent |
| 12 | lastPlan more (0 when absent) |
| 13–15 | Reserved 0 |

List row (9): thread ref, title ref, updatedAt ref, path ref, origin (−1 absent, 0 other, 1 lair).
Read row (10): prompt ref, reply ref, createdAt ref, completedAt ref, tools, clipped — the qwen turn layout.
Plan item rows (2 words: text ref) follow the main rows; row_words = count·width + 2·items.
Refs are (start,len) into the text bytes (UTF-8); (−1,0) is absent.

Failure codes: 1 InputTooLarge, 2 InternalInvariant, 3 not-found, 4 codex-unsupported, 5 timeout, 6 read-failed,
7 engine-missing. Stages: 0 pre-spawn, 1 spawn, 2 initialize, 3 thread/list, 4 thread/read, 5 thread/turns/list,
6 result. Detail is the error message bytes when a response carried one (diagnostic only, not compared).

The host renders list results as the TS item array, read results as the TS ExternalRecord, and failures as
`{"error":"<code name>"}`. The reference driver renders TS results the same way (EngineImportError → its code).

## Policies (differences from TS)

- P1–P5 as in QWEN_IMPORT_SPEC (strict UTF-8 lines are skipped; lone surrogates become U+FFFD in values; byte
  clipping; isoOf integer/canonical-ISO policy; bounded pure core). Folder and key comparisons use exact UTF-8
  bytes (UTF-16 unit equality for valid text); a `cwd` holding a lone surrogate never matches, since its decoded
  U+FFFD must not stand in for the surrogate (TS compares the units and does not match either).
- P6 framing: the line cap is 8388608 bytes of an unterminated line (TS: 8 Mi UTF-16 units of leftover buffer after
  a chunk). Response ids match only as JSON numbers whose decimal token equals the outstanding id exactly
  (`1`, not `1.0` or `1e0`). A final unterminated fragment at EOF is ignored (TS: never parsed). The pump loop has a
  fixed iteration bound; reaching it fails as `read-failed`.
- P7 lifecycle: G10 child close replaces SIGTERM / 1.5 s SIGKILL / 5 s give-up timers. The result is decided
  before shutdown and shutdown never changes it. Known limits (G10, shared `transport` package, also mcp):
  `ev_proc_wait` after EOF can block past the timeout when the child closes stdout but keeps running, and a
  blocking `ev_proc_write` ignores the timeout when the child stops reading. TS bounds both with its timers.
  G track: a deadline-aware wait (or a nonblocking exit poll) and a nonblocking or deadline-aware write.
- P8 plan text: the 200-unit slice drops a trailing high surrogate split by the cut, as one_line does.
- P3 and P4 consequences: a genuine U+FFFD left at the end of a byte clip is kept (TS strips it). Rule 6 takes
  `completedAt ?? createdAt` after isoOf, so when P4 drops the plan turn's `completedAt` (for example a fractional
  number) the plan's `at` is that turn's `createdAt`, or absent.
- Request JSON is compared semantically (parsed), not byte for byte. The cursor is re-sent as the raw JSON token
  received.

## Packages (moves in this batch)

| New package | From | Depends on |
|---|---|---|
| bytes | qwen/boundary.pruv `copy_span` | — |
| text | qwen/text.pruv | — |
| time | qwen/time.pruv | json, jstr, access, text |
| names | qwen/names.pruv | — |
| turns | qwen/turns.pruv (+ `tool_n`, folded-reply entry) | bytes, text |
| args | mcp/args.pruv | — |
| transport | mcp/transport.pruv, line cap as a parameter (mcp 4194304, codex 8388608) | tail |

qwen and mcp keep their outputs; their root manifests add the packages. Codex-local modules: rpc, list, read,
item, plan, folder, packet, root.

## Proof targets

Both compilers verify all reachable contracts: memory safety, loop termination (event pump, pages ≤ 40, ids ≤ 42),
packet shape and reference ranges. TS equivalence is tested, not proved.

## Tests

- `test_impl.py`: four pairs (py/rust, py/llvm, rs/rust, rs/llvm) build verified hosts and run fixed scenarios
  against the scripted fake app-server, comparing with checked-in expected JSON and with the pinned TS.
- `q_server.py`: fake `codex` executable driven by a scenario file (path in an argument). It checks every request
  (method, params, id sequence, initialized order) and records them; a mismatch is reported in the transcript.
- `q.py`: 2000 seeded scenarios (seed 20261116) plus fixed probes, run on the pinned TS under Node and on all four
  pairs; results and request transcripts must match, except classified P1–P4/P6/P8 cases. The fake's `await`
  action field reads that many client lines after `pre` before it answers, so replies to server requests are
  logged before the client closes the child.
- `q_mutants.py`: wrong id accepted, initialized omitted, -32601 mapping lost, a not-found alternative dropped,
  list admission bypassed, stop after first page, newest-first feeding, commentary admitted, plan/reasoning counted
  as tools, generic tool not counted, budget omission removed, reply bytes not capped, later plan replacing the
  first. Each must be killed.
- After the moves: qwen `test_impl.py`, `q.py`, `q_mutants.py`; mcp `test_impl.py`, `q.py`, `q_mutants.py`;
  json and supervisor runners if they use a moved package; `bench/g4b/measure_cache.py` if paths change.

## Deviations recorded during implementation

- Deviation: the `turns` package is not edited (no `tool_n`, no folded-reply entry). `item.pruv` folds replies with its
  own copy of the `answer` arithmetic; `read.pruv` replays one `turns::answer` per turn (the folded text, plus one
  4-byte padding scalar when the fold was clipped, which the collector cuts off again) and `turns::tool` in a loop.
  See `bench/l3/codex/IMPL_NOTES.md` for the argument.
- Deviation: spawn failures are always `engine-missing` (as TS, whose child `error` event maps to it); no host
  `command_missing` lookup is used, so the root declares no `trusted` function.
- Deviation: an `isoOf` defect (corrupt tape) is an absent timestamp, not an import failure.

## Reference

`bench/l3/codex/reference/` holds verbatim `git show 744d14af:` copies (see its README for hashes). Node runs them
with hooks that stub the unused account/engine modules; the child is the real fake app-server process.

## Risks

- Timing: fixed probes with deadlines use short timeouts on both sides; scripted hangs never race a response.
- The collector replay must not drift from qwen's collector; qwen tests guard the unchanged paths.
