# Grok conversation import

Status: implemented, L3-6 (design 2026-11-16; results in RESULTS.md section 136). This is the implementation contract.

## Goal

Port Lair commit `744d14af`, pinned verbatim in `bench/l3/grok/reference/` (hashes/callers in its README).
Pruv owns parsing, admission, validation, ordering, budgets, collection, allow-list/error/race decisions; Rust
supplies primitives. Both compilers and both backends must agree. Missing capabilities belong to G, not Rust
importer policy. Only numbered policies permit differences; do not repair surprising TS behavior.

## Scope

There are five operation exports and a sixth utility export; account for all six:

| Export | Decision and reason |
|---|---|
| `listGrok` | In: discovery for the selected source and folder, including active-session hints. |
| `readGrok` | In: `engine-import-open` first calls `readExternal`, which dispatches here. |
| `copyGrokSession` | In: the open caller then copies the session and invokes verification. |
| `grokLoadCheck` | In: the verification callback must really initialize and load the copy through ACP. |
| `removeGrokCopy` | Out: used by `migrate/work-undo.ts`, not the open flow. Its source-home exclusion and undo authorization belong to a later migration slice. |
| `readSmallJson` | In as the shared summary-reading utility, tested directly; its separate migration scanner caller is out. |

Preserve the open caller's sequence: read, copy with `home => grokLoadCheck(...)`, publish/bind, recursive removal
of the returned copy on publication race/failure. This is caller `rm`, not undo. Database/account/UI/source
discovery/deployment integration is out. Never read/copy authentication files. Grok never invokes `plan.ts`:
`lastPlan` stays absent even for plan-like tool calls.

## Division of work and G dependencies

Use a Pruv state machine emitting primitive requests and consuming raw results: bytes, metadata, entry kinds,
descriptor identity and OS errors. Rust dispatches requests; Pruv constructs paths, compares identities, chooses
offsets and cleanup, interprets errors and cancellation events. Unlike Qwen, the host never parses JSON, filters,
sorts or makes import decisions. Descriptor transfers use bounded chunks, with no copy-file-size cap.

`pruvc/foreign.py` confirms File has only `file_open_read`, truncating `file_create`, `file_read_all`,
`file_write_str/bytes`, `file_close` and observers (`file_size` means buffered bytes, not metadata).
Despite `G10_SPEC.md`'s stale status heading, these exist; they cannot replace checked window reads or wx creation.

| Missing primitive | Provider for the benchmark / named replacement |
|---|---|
| `readdir(path)` with names, entry kinds and native enumeration order | Thin Rust host now; **G10-FS-DIR** later. Return every entry, including unknown kinds and links. |
| `lstat(path)` with kind, size, dev/ino and errno (no final-link following) | Host now; G10-FS-DIR. Pruv implements plainDir/plainFileIn and both existence tests. |
| `realpath(path)` and errors | Host now; G10-FS-DIR. Pruv performs separator-aware containment. |
| read-only open with `O_NOFOLLOW` when available, descriptor `fstat`, positioned/chunk reads and close | Host now; **G10-FS-HANDLE**, alongside G10-FS-DIR; Pruv compares expected dev/ino before reading. |
| `mkdir(path, recursive, mode)` | Host now; G10-FS-DIR, modes supplied by Pruv (0700). |
| `rename(from,to)` | Host now; G10-FS-DIR, ordinary OS rename, not an invented atomic no-replace operation. |
| recursive force remove | Host now; G10-FS-DIR; report errors even when Pruv will ignore them. |
| exclusive create (`wx`), mode 0600, chunk write and close | Host now; G10-FS-HANDLE; no implicit overwrite, chmod or permission repair. |
| file/directory creation modes | Host now; the two G10 slices above. Respect umask and leave existing parent modes alone; general chmod is not required. |

Recommendation: use the thin host now, replace it with G10-FS-DIR/HANDLE later. G10 first reduces temporary trust
but enumeration alone leaves no-follow/identity/streaming unsolved; the table is the minimal combined slice.
This boundary does not claim current Pruv has these primitives. Random six-byte entropy is also host-provided,
to become **G10-ENTROPY**; Pruv hex-encodes it and constructs the temp name.

ACP reuses Codex's `rpc.pruv` approach and unchanged `transport`. G10 proc supplies args/cwd; async supplies pipe
events/timers. Net is unused except the Loop close effect. Per-child env/detached groups and external cancellation
are missing from G10 today (`proc_spawn(program, args, cwd)` has no env); see the scheduling decisions below.

`access::number` exposes raw tokens/exact i64, not JS binary64 conversion. TS keeps `num_chat_messages` for any
finite nonnegative JS number (fractions, exponents, rounded integers, negative underflow to -0); see P9.

### Scheduling decisions (main agent, 2026-11-16)

These settle the designer's open questions and bind the implementation.

- **D1 — filesystem primitives: thin host now.** Same precedent as L3-4 (Qwen listing in the host): the table
  above is implemented in `host.rs` as primitive requests only. **G10-FS-DIR** and **G10-FS-HANDLE** (and
  **G10-ENTROPY**) are recorded as G-track follow-ups that later replace the host; they are not L3-6 prerequisites.
- **D2 — per-child environment: G10 slice first (G10-PROC-ENV).** GROK_HOME is what makes the load check check the
  copy, so it cannot be dropped, faked with an `env`/shell launcher or set by mutating the parent. Before the L3-6
  load-check code, both compilers and both backends gain a spawn form taking environment overrides
  (`proc_spawn_env(program: str, args: ProcessArgs, cwd: str, env_names: ProcessArgs, env_values: ProcessArgs)`,
  reusing the existing `ProcessArgs` builder because `[str]` does not exist): inherit the parent env, then set each
  pair in order (a repeated name: last wins); no unset or full replacement. Unequal counts, an empty name, or a name
  containing `=` or NUL give the same failed `Process` as a failed `proc_spawn` (must-consume unchanged, `proc_ok`
  false); NUL in a value is rejected the same way. Spawning stays in the importer's `rpc.pruv` (as in
  `bench/l3/codex/rpc.pruv`); `transport` is not changed. This slice gets its own spec section, tests and mutants in
  the usual G10 shape, not a Rust-host workaround.
  **L3-6 environment domain:** TS builds the child env as `{...deps.env, GROK_HOME: home}` and passes that object as
  the whole environment. L3-6 covers `deps.env` equal to the parent process environment (what the open caller
  passes in practice) plus the GROK_HOME override; the reference runner is given `deps.env = process.env` of the same
  parent. A `deps.env` that differs from the parent (removing or replacing variables) is out of scope, not a policy.
- **D3 — detached process group and AbortSignal: out, as in L3-5.** `docs/CODEX_IMPORT_SPEC.md` excluded `detached`
  groups and AbortSignal; L3-6 does the same. The AbortError paths (copy loop/transfer abort, pre-aborted signal,
  abort during load check) and group kill are listed as known limits, not claimed equal, and are not Q cases.
  **G10-PROC-GROUP** and **G10-ASYNC-CANCEL** are recorded as G-track follow-ups. This replaces the "no P5
  cancellation waiver" wording: there is no waiver, the paths are simply not ported.
- **D4 — message counts: P9, not a G prerequisite.** A JS-exact binary64 parse-and-print facility is recorded as the
  G-track follow-up **G-JSON-NUM**; L3-6 uses P9.
- **D5 — deadline-aware wait/write: adopted.** G10-ASYNC-DEADLINE added `ev_proc_wait_for` and `ev_proc_write_for`;
  the shared `transport` package uses both (transport adoption, RESULTS §138). P7 keeps only the close-time limits.

## Preserved behaviors

### Paths, admission and summary

- `validThread` is exactly `^[A-Za-z0-9][A-Za-z0-9_-]{0,127}$`. Only copy checks `source.engine === 'grok'`.
- Folder directory = `encodeURIComponent(folder)`: UTF-8 percent bytes with uppercase hex; leave only
  `A-Z a-z 0-9 - _ . ! ~ * ' ( )` unescaped. Reject the resulting empty string, `.` and `..` as `not-found`.
  Do not encode a normalized folderKey instead. TS throws URIError on a lone-surrogate folder (P2's input boundary).
- `realHome` catches every realpath error as absent. `plainChain` checks `home/sessions`, the encoded folder and,
  when supplied, the thread, in that order with short-circuiting. `plainDir` means lstat succeeded, directory,
  not symlink. Home itself may be a symlink; do not add a new rejection of it.
- `plainFileIn` first requires a non-symlink regular lstat file, then realpath beginning with
  `homeReal + separator` (or homeReal if already separator-terminated). Errors become absent. A similarly named
  sibling is outside. `openRead` uses O_NOFOLLOW when available, then fstat dev/ino equality with that lstat;
  mismatch/stat failure closes the handle and throws `read-failed`. Hard links inside the path are not rejected.
- `readSmallJson` rejects size **greater than** 65536, opens with identity checking, reads UTF-8 to EOF, parses an
  object root and always closes. Any error returns absent. The size check is before open: later growth is not a new
  64 KiB truncation rule. `readSummary` additionally requires object `info`, string id exactly equal to thread,
  string cwd, and equal folderKey(cwd)/folderKey(folder). Non-Windows folderKey strips trailing `/` unless that
  empties the string; all-slash strings stay unchanged. It neither resolves dots nor case-folds.
- TS isoOf accepts finite numbers (seconds below 1e11, otherwise milliseconds); trimmed numeric strings matching
  `/^\d+(\.\d+)?$/` use that same threshold, other nonblank strings use Date.parse. Invalid dates are absent;
  valid ones use toISOString. P4 narrows that domain explicitly.

### listGrok

1. Missing real home or failed chain returns `[]`; readdir failure also becomes `[]`. Encoded-folder rejection
   propagates when reached (a missing home short-circuits it). Admit only directory entries with valid threads;
   recheck plainDir and matching summary for each. `mapLimit` has four workers and returns input order, not finish
   order. Each worker checks abort before examining its directory; an empty candidate list does not poll abort.
2. Title = `oneLine(trim(generated_title) || trim(session_summary) || '',120)`, string fields only.
   oneLine collapses ECMAScript whitespace runs to one ASCII space, trims and slices 120 UTF-16 units (P2).
   Path is the joined directory path. updatedAt is `isoOf(summary.updated_at)` if defined. Messages is present
   only for a JS number that is finite and >= 0; zero and fractions are valid, strings and infinity are not.
3. Sort all admitted items with exactly `Date.parse(b.updatedAt ?? '') - Date.parse(a.updatedAt ?? '') || 0`.
   Missing/invalid dates compare equal to **every** item, not "oldest". Equal dates retain enumeration order.
   This comparator is not a total order: use the pinned Node 26 sort comparison schedule for mixed missing dates,
   not an arbitrary stable sorting algorithm. Include inputs on both sides of the short-array sort threshold.
4. Read `home/active_sessions.json` with the same plain-file/identity/65536-byte checks, but accept an array root.
   Each entry is a string, else string `id ?? sessionId ?? session_id` in that precedence; an empty string id
   prevents fallback and is then ignored. Nonempty ids are put in a set without validThread filtering. Any error
   yields the empty set. Take the first 50 sorted items, adding `maybeOpen:true` only for a matching id. Never emit
   false, infer activity from mtime or suppress an active session.

### readGrok, framing and collector

1. Invalid thread, absent home/chain/summary/history plain file => `not-found`. Model is trimmed string
   `summary.current_model_id`, omitted if empty, with no Codex 200-unit limit. Title uses the list fallback,
   omitted if empty; list always has a title. No metadata is taken from history model/title fields.
2. Open history with identity checking; fstat its current size; read `[max(0,size-8388608),size)` from that handle,
   using 256 KiB source chunks as TS. If start > 0, discard through the first LF even at a line boundary, with no
   dropped-user notification. Only LF-terminated ordinary lines are parsed; CR counts toward the 1048576-byte cap.
   A subsequent line over that cap, including an oversized unterminated final line, emits one drop at that position.
   An ordinary unterminated final line is ignored. Invalid/nonobject JSON is skipped; duplicate keys are last-wins.
3. The dropped-prefix test, on at most the first 8192 bytes decoded like Buffer UTF-8, is exactly
   `/^\s*\{\s*"type"\s*:\s*"user"/` AND NOT `/"synthetic_reason"\s*:\s*"/`.
   It is raw matching, not JSON: type must be the first field, no closing-field delimiter is required, escaped
   keys do not match, and only a string-looking synthetic marker suppresses the drop. Numeric/null synthetic
   values do not. A matching drop calls skipUser before the next line; a synthetic marker beyond 8192 is unseen.
4. Entry time = isoOf(`timestamp ?? created_at`); a present invalid timestamp prevents fallback. `contentText`
   returns a string unchanged, else takes only array elements' string `.text` values, drops empty strings and
   joins with two LF characters. Whitespace-only parts survive; no type/thought filtering.
5. A user is skipped if `synthetic_reason` is neither missing nor null, including false, 0 and empty string.
   Let trimmed = text.trim(). If it starts `<user_query>`, ends `</user_query>` and is at least the combined tag
   length, use the inner substring, removing exactly one leading `\r?\n` and one trailing `\r?\n`.
   Outer whitespace is discarded; inner spaces/additional newlines and lone CR are kept. This is not XML parsing.
   Otherwise, skip text starting `<user_info>` after trim only when prompt_index is missing/null. Zero/false/empty
   prompt_index prevent that skip. Query unwrapping takes precedence. Begin only if the final text.trim() is
   nonempty, passing the untrimmed final text and at to the collector.
6. An assistant calls answer(contentText,at), even for empty text, then tool(tool_calls.length) if it is an array.
   Every array slot counts, regardless of its contents. Other entry types do nothing. Answers/tools without a
   current user are ignored. Do not filter commentary, synthetic assistants or reasoning content within text.
7. TurnCollector clips prompts/replies to 32768 UTF-8 bytes separately. begin consumes lostBefore and sets clipped
   for that flag or a long prompt. skipUser detaches the draft and marks the next begun draft clipped. answer
   updates completedAt before the empty/saturated-text check; noninitial reply text gets `\n\n`, counted in its
   budget. Partial append saturates replyBytes and sets clipped; a later append at an already-full budget does
   not newly set clipped. tool changes only count. Finish keeps the newest contiguous suffix of <=200 turns and
   <=2097152 prompt+reply bytes, oldest-first in output; it stops, rather than skips over, an over-budget turn.
8. `omittedBefore = window.omittedBefore || collector.trimmed`, not "any ignored line" or merely clipped.
   The history try/catch preserves EngineImportError and AbortError; ENOENT becomes `not-found`, other errors
   `read-failed`. The preliminary summary read swallows errors, so its identity/read failure becomes `not-found`.

### copyGrokSession and errors

- Reject wrong engine, invalid thread or empty targetHome as `not-found`; construct source/parent/target, validate
  source home and chain. Copy does **not** call readSummary; direct copy may succeed with absent/invalid summary.
- Initial lstat(target): any existing kind, including dangling symlink, => `grok-exists`; only ENOENT means absent.
  Other errors propagate raw, outside the copy try/catch. Generate `<thread>.lair-import-<12 lowercase hex digits>`
  using randomBytes(6), before entering that try; entropy failure also propagates raw, with no cleanup attempt.
- mkdir parent recursively mode 0700, then temp nonrecursively mode 0700. Iterate every source readdir name,
  checking optional abort **before each**. COPY_ALLOW is exactly:
  `summary.json`, `chat_history.jsonl`, `events.jsonl`, `updates.jsonl`, `prompt_context.json`,
  `resources_state.json`, `rewind_points.jsonl`, `signals.json`, `usage.json`, `system_prompt.txt`,
  `tool_definitions.json`, `announcement_state.json`, `title_refresh_idx`.
  Nonallowed or non-plainFileIn names each add one skipped; do not descend. Allowed files stream through openRead
  into exclusive-create mode-0600 destinations. No JSON validation, summary-size limit or content rewrite on copy.
  Pipeline cancellation aborts an in-flight transfer; an empty source has no loop abort poll.
- Second lstat(target), immediately before rename: success => `grok-exists`; **all** errors mean absent here.
  Rename temp to target. Copy/rename failure attempts recursive force temp removal, ignores cleanup errors,
  preserves EngineImportError/AbortError, maps everything else to `read-failed`.
- After rename, call verify(targetHome) exactly once. Rejection attempts recursive force target removal and ignores
  cleanup errors; AbortError stays AbortError, every other rejection (including timeout/engine-missing) becomes
  `grok-load-failed`. Success returns `{path:target,skipped}`. No extra abort check between rename and verify.
  Failure does not remove newly made parent directories. Cleanup is not a guarantee that deletion succeeded.
- Preserve raw errors at boundaries outside the catches; do not flatten all failures into the four import codes.
  Test packets distinguish EngineImportError code, AbortError name, RpcResponseError and raw OS/URI errors.

### grokLoadCheck / withRpcChild

Spawn binary with exactly `agent --no-leader stdio`, cwd=folder, env=`{...deps.env,GROK_HOME:home}`, shell=false,
piped stdin/stdout/stderr and detached=true on non-Windows. A pre-aborted signal fails before spawn. Request ids
start at 1; each outgoing request and server-request rejection includes `"jsonrpc":"2.0"` and ends in LF:

1. `initialize {"protocolVersion":1,"clientCapabilities":{"fs":{"readTextFile":false,"writeTextFile":false},"terminal":false},"clientInfo":{"name":"lair","title":"Lair","version":"0.1.0"}}`.
2. Require object result.agentCapabilities with `loadSession === true`; otherwise `grok-load-failed` (`no loadSession`).
3. `session/load {"sessionId":thread,"cwd":folder,"mcpServers":[]}`. Any successful response result, including
   missing/null, completes verification. There is **no** Codex `initialized` notification and no thread validation here.

Split stdout on LF, ECMAScript-trim, ignore blank/invalid/nonobject lines. A string method takes precedence:
with any defined id (including null), reply with that same id and `error:{code:-32601,message:"not handled"}`;
otherwise ignore notification. Other lines match numeric outstanding ids only. An object error rejects
as RpcResponseError (string message sliced to 300 UTF-16 units, else `rejected`, raw code preserved); other error
types do not reject. Direct loadCheck propagates that response error, unlike copy's verify wrapper.
Drain stderr; ignore stdin error events; stdout errors/child close => `read-failed`, spawn/error event =>
`engine-missing`, abort => AbortError, single overall 20000 ms timer => `timeout`. First terminal failure wins.
TS parses complete lines before checking the leftover buffer's 8388608 UTF-16-unit cap (P6).
Finally unsubscribe/clear timer, close stdin, SIGTERM the detached group, SIGKILL after 1500 ms, wait at most
5000 ms for close; cleanup does not replace the operation result. P7 specifies the current G10 lifecycle difference.

## Policies (differences from TS)

Every allowed difference is labeled below. No P5 blanket exemption for filesystem decisions or error mapping. D3 paths are not ported, not exempted.

- **P1 — Deviation:** invalid UTF-8 or JSON exceeding depth 128/tape 1048576 records is rejected by the shared
  parser. Trigger: e.g. raw byte FF inside an otherwise valid JSON string, or 129 nested containers. History/RPC
  lines are ignored; summary becomes absent and active-list becomes empty. Raw dropped-prefix scanning still
  uses replacement decoding. InternalInvariant is an implementation failure, not malformed user input.
- **P2 — Deviation:** lone surrogate JSON values become U+FFFD; keys keep UTF-16 identity. Trigger: `"\ud800"`
  as a value. A surrogate-containing cwd cannot match a literal U+FFFD folder. oneLine's cut omits a split scalar
  (119 ASCII units followed by an astral scalar at the 120-unit boundary). Folder inputs are valid UTF-8;
  JS-only lone-surrogate arguments are outside that representable boundary, not silently encoded as U+FFFD.
- **P3 — Deviation:** byte clipping keeps genuine trailing U+FFFD. Trigger: 32765 ASCII bytes + U+FFFD + `x`
  clipped at 32768, where TS removes that genuine replacement character. Budget saturation remains unchanged.
- **P4 — Deviation:** reuse `time` unchanged: safe lexical integer numbers, trimmed unsigned digit strings,
  exact Gregorian-valid `YYYY-MM-DDTHH:mm:ss.sssZ`, years 0000–9999. Signed numeric comparison <100000000000
  selects seconds, else milliseconds. Fraction/exponent numbers (`1.5`, `1e3`), signed/decimal numeric strings,
  offset/local dates, overflow calendars or out-of-range dates become absent. This can change sort order and
  completion times. Message counts are governed by P9, not P4.
- **P5 — Deviation:** bounded core packets, ordinary unpolluted JS runtime, non-Windows paths and UTF-8 OS names.
  Oversized core input (e.g. a supplied history window of 8388609 bytes) => InputTooLarge, not truncated success.
  A small-checked JSON file racing above the parser's 16777216-byte source cap => InputTooLarge, an explicit
  exception to small-json error swallowing. No extra session-count or copy-byte cap.
  **Listing concurrency (not a deviation):** listing keeps four candidates in flight, as `mapLimit` does. Each of
  up to `IMPORT_LIMITS.parallel` (4) worker slots has at most one outstanding primitive request, tagged with its
  slot. The host runs outstanding requests concurrently and hands back **one completion at a time, as soon as it
  finishes** (no waiting for the others). Pruv processes that completion and immediately issues the same slot's next
  request: the candidate's next step, or, when the candidate is done, its first step for the next unclaimed name in
  input order. Results are collected by input index. Serial execution and batch-wide barriers are not allowed.
  Tests assert from the primitive trace (a) four delayed requests outstanding together and (b) with candidates A–E
  where only A's and E's first step is delayed, E is started before A completes. Overlapping live mutations/failure races
  during listing are outside the tested equivalence domain and must be reported, not passed as exact matches.
- **P6 — Deviation:** reuse Codex framing: byte cap 8388608 on every RPC line, rather than TS's UTF-16 leftover
  cap after each chunk. Trigger: a >8 MiB complete line delivered with LF in one chunk, or non-ASCII leftover
  text exceeding the byte cap first. Reply id must be the exact decimal token `1`/`2`, not `1.0`/`1e0`.
  **Chunk ordering (not a deviation):** TS handles every line of one stdout `data` event before the next request is
  sent. Pruv does the same: after a response settles a request, the remaining complete lines of the current read
  chunk (`transport` state `st[2] < st[4]`) are processed against the outstanding set **before** the next request is
  written, so a premature `{"id":2,...}` in the same chunk as the initialize reply is discarded and cannot complete
  `session/load`. Exactness is claimed when the fake delivers both lines in one write of at most 512 bytes
  (PIPE_BUF, atomic on the platforms in P5), which both Node and the G10 reader receive as one read. Fixture
  required. Larger or split deliveries are P12.
  A 1000000-step per-request pump exhaustion => `read-failed`; pin that count in tests. Final non-LF RPC fragments
  are ignored on both sides. Compare outgoing JSON semantically, not property order or escape spelling.
- **P7 — Deviation:** the shared transport's G10 close replaces TS's timed TERM/KILL/give-up sequence. Trigger:
  a child requiring a grace period, keeping pipes open, or not exiting after EOF. After EOF `transport::receive`
  waits for the exit in 10 ms `ev_proc_wait_for` slices and polls the overall timer between them, so a child that
  closes stdout and stays alive ends as `timeout` as in TS. `transport::send` writes with `ev_proc_write_for` and the
  overall budget as its deadline. Write latency is a stated limit, not part of P7: the loop has no clock, so each write gets the whole budget; the first write that hits it half-closes stdin and later writes are skipped, but a child that reads slowly can delay timer observation by up to one budget per write. A fired timer is still seen by the next poll, so only latency differs, not the result (a new request after a deadline half-close first takes the expired overall timer and ends in timeout). G follow-up G10-TIMER-LEFT (remaining time of a timer slot) would bound writes by the remaining overall time. Close
  (`ev_remove`) can still wait on a descendant that inherited the pipes; classify these separately (limits below). This does not waive GROK_HOME (D2).
  Detached groups and AbortError are not ported (D3), so they are known limits, not P7 cases.
- **P8 — plan cut:** Codex's Deviation (split high surrogate at a 200-unit plan slice) has no triggering Grok input:
  Grok emits no lastPlan. Do not add plan extraction or claim a plan-omission deviation.
- **P9 — Deviation:** `num_chat_messages` is kept only when its JSON token is a nonnegative decimal integer without
  sign, fraction or exponent whose value is at most 9007199254740991 (2^53-1), rendered as that integer. Any other
  number (fraction such as `1.5`, exponent such as `1e3`, `-0`, larger integers) makes `messages` absent on the
  Pruv side where TS keeps a JS number. Non-number values are absent on both sides. Trigger: `1.5`, `1e3`, `-0`,
  `9007199254740993`. Grok writes integer counts, so the deviation is outside its real output.
- **P10 — Deviation (number pass-through):** JSON numbers that L3-6 returns or echoes keep their raw token on the
  Pruv side: `readSmallJson` result values, the server-request id echoed in the `-32601` reply, and the raw
  `error.code` of an RpcResponseError. TS converts through binary64 and re-serializes. The harness compares numbers
  by binary64 value (Python `float(token)` against the TS number), so `9007199254740993` vs `9007199254740992` and
  `1.0` vs `1` compare equal. The remaining difference is a token that overflows binary64: TS sends/returns `null`
  (JSON.stringify of Infinity), Pruv the token. Trigger: `{"x":1e400}` small JSON, server request id `1e400`, error
  code `-1e400`. G follow-up: G-JSON-NUM.
- **P11 — child stdin write failure: equal (was a deviation before transport adoption).** TS ignores stdin write
  errors and error events and keeps reading stdout. `transport::send` now uses `ev_proc_write_for`, which half-closes
  stdin on a broken pipe without failing the slot, so Pruv keeps reading stdout too. Replies to server requests ignore a closed stdin (TS catches the throw); a new
  request (`initialize`, `session/load`) on a closed stdin fails read-failed at its stage, as TS's `write` throws
  `read-failed 'closed'` when `!child.stdin.writable`. Fixture `acp-p11-stdin-closed-then-late-result` (the fake
  closes its stdin, sends a server request, then a valid `session/load` result) compares equal; the oracle no longer
  has a P11 allowance. The G follow-up G10-PROC-HALFCLOSE is covered by G10-ASYNC-DEADLINE.
- **P12 — Deviation (read boundaries):** a Node `data` event holds up to 65536 bytes; the G10 process reader reads
  8192-byte pieces into an unbounded queue and `ev_proc_read` returns what has arrived. When a response settles a
  request, Pruv processes only the lines already read; lines read later are processed after the next request is sent.
  So a premature reply to a not-yet-sent request that TS would see in the same `data` event (and discard) can be
  matched by Pruv after it sends that request. Trigger: initialize reply, then padding past 8192 bytes, then
  `{"id":2,"result":null}` in one write, with `session/load` never answered: TS times out, Pruv may succeed. A real
  ACP agent answers only requests it has received, so this needs a misbehaving agent. G follow-up: none needed for
  the importer; the difference is recorded, and Q classifies it only for that trigger shape.

Q must demonstrate each P1–P6 and P9–P12 output difference with the minimal triggering input and a narrowly normalized TS
rerun or explicit expected policy result; P7 requires a lifecycle trace and an explicit bounded outer watchdog.
Anything outside these triggers is an unexplained failure. A watchdog termination is not successful verification.

## Layout and boundary representation

All new importer files live under `bench/l3/grok/`:

| Files | Responsibility |
|---|---|
| `root.pruv`, `pruv.json`, `packet.pruv` | Public list/read/copy/load and small-json entries; flat result/primitive packets. |
| `path.pruv`, `summary.pruv`, `list.pruv` | Encoding, folderKey, admission/identity/containment, active ids, ordering and cap. |
| `read.pruv`, `entry.pruv`, `framing.pruv` | History window, Grok raw watcher/content/tag rules and collector calls. |
| `copy.pruv` | Allow-list, skipped count, publication/verify/rollback state machine, caller cleanup request. |
| `rpc.pruv` | ACP requests, matching, capability decision, transport/error state; adapt Codex's approach, not its initialize message. |
| `host.rs` | Thin fs driver, argument/signal input and checked JSON rendering; no import decisions. |
| `reference.mjs`, `test_impl.py`, `fixtures/`, `q_server.py`, `q.py`, `q_mutants.py`, `work/` | Oracle runner, tree/scenario recipes, fake ACP, differential/mutants and disposable outputs. |

Reuse `bytes`, `text`, `time`, `names`, `turns`, `args`, `transport` and packaged `json/jstr/lex/lines/parse/access/tail`
unchanged. Use G4b manifests (`0.1` requirements/current `0.1.0`), direct dependencies only; do not mount legacy
`bench/l3/lib` copies. Count tools with one unchanged `turns::tool` per array slot. Grok framing stays local.
Any shared extraction/edit requires all consumers' runners (Codex, Qwen, MCP, JSON, supervisor as applicable).
No package edit is authorized by this document task.

Use the Codex/Qwen flat-packet convention: versioned metadata, fixed-width rows and `(start,len)` refs into UTF-8
bytes; `(-1,0)` means absent. Read rows keep the ten-word turn layout. List rows carry thread/title/path/time refs,
optional numeric message representation and maybeOpen; copy carries path/skipped; load success carries no record.
Primitive packets carry request id/opcode, path/data refs, errno/kind/size/identity (distinct ENOENT/EACCES/EEXIST).
Errors preserve stage/class/code; diagnostic text is not compared. Host validates packet shape/ranges only.
Both compilers verify reachable contracts, loop/collector bounds and packet ranges; OS facts remain trusted,
and TS equivalence/race safety are tested, not proved.

## Tests

`test_impl.py` builds verified py/rust, py/llvm, rs/rust, rs/llvm runners. Harness-built temporary trees and fake
Grok ACP replace personal homes/credentials/network. Compare JSON, requests, primitive traces and resulting tree
bytes/kinds/modes with pinned TS on Node 26.4.0 using its hooks. Reference hashes stay unchanged.

Fixed fixtures include:

- `paths`: encoded slashes/spaces/Unicode/percent, empty/dot/dotdot, invalid threads, all-slash/trailing-slash cwd;
  `identity`: linked home accepted, each linked chain component rejected, summary/history links, outside sibling,
  hard link, FIFO/directory, and ancestor/file replacement between lstat, realpath, open and fstat.
- `summaries`: 65536/65537 bytes, malformed/array/null roots, wrong id/cwd, missing history, direct small-json
  failure and summary growth; `listing`: >50 valid with newer rejected entries, blank fallback titles, 120-unit
  cuts, fractional/exponent/overflow/underflow messages, mixed missing/equal/valid dates at 10 and >64 entries.
- `active`: strings, id/sessionId/session_id objects, empty id suppressing fallback, duplicates, wrong root,
  oversize and unreadable file. `content`: string/array/empty/whitespace parts and tool_calls with junk elements.
- `tags`: outer whitespace, CRLF/LF/lone CR, repeated inner newlines, incomplete/repeated tags, wrapped user_info,
  prompt_index missing/null/0/false, synthetic_reason missing/null/false/0/empty. `metadata`: model/title summary
  priority, timestamp nullish fallback versus invalid-primary, ignored plan-like tool calls.
- `dropped`: >1 MiB ordinary and synthetic raw prefixes, type not first, escaped keys, marker beyond 8192,
  oversized final fragment, assistant between drop and next user. `budgets`: 8 MiB cut on/off LF, 1 MiB line
  boundary including CR, 200/201 turns, 2 MiB suffix, saturated/empty answers, UTF-8 cuts and genuine U+FFFD.
- `copy-allow`: all 13 names plus auth/lock/unknown/nested/link entries, byte equality and skipped count;
  `copy-exists`: file/dir/dangling-link target; `copy-raced`: target appears before second check;
  `copy-faults`: initial EACCES versus second EACCES, source read/destination wx/rename failure, temp collision,
  cleanup failure; `verify-failed`: RPC/timeout/general error and target removal (abort cases are D3 known limits);
  `open-raced`: successful copy then caller rollback, source unchanged. Observe modes with a fixed test umask.
- `acp`: exact argv/cwd and the GROK_HOME env override (D2), true versus missing/false/string capability,
  initialize/load errors, absent result, notifications/server requests (including null id), mismatched ids,
  malformed/split/oversized/final lines, stderr flood, spawn failure, early exit and timeout.
  The fake checks request order and records an unexpected `initialized`. It drains/replies deterministically;
  lifecycle-only cases exercise blocked write, EOF without exit and descendants holding pipes under a watchdog.

`q.py`: 2000 seeded scenarios (20261116) plus fixed cases, TS versus all four pairs. Freeze source bytes; replay
identical readdir order through test-only primitive schedules, never sort outputs. Check tied/NaN sort against
pinned Node. mapLimit completion delays cannot change result indices. Inject races at primitive barriers, not sleeps.
Replay randomBytes' six bytes or normalize only the temp suffix; separately assert format/placement/collision.
No production seeded entropy. Avoid reply/deadline races; probe the actual 20 s setting once (grok.ts reads `IMPORT_LIMITS.rpcMs` directly, so the reference runner
sets and restores that field per case), use equal shorter
deadlines for injected timeout tests. Never normalize order, copied bytes, error classes or cleanup actions.

### Q mutants

Build each single mutation separately, initially on rs/rust. A named fixture must distinguish its output/trace
from the baseline, or the verifier must reject it for the changed contract. Record which occurred; all 16 must die.

| Mutant | Intended killing fixture and observation |
|---|---|
| M01 `encode-folder-as-path` | paths: `/a b` must use `%2Fa%20b`, not nested directories. |
| M02 `follow-chain-link` | identity: linked sessions/thread must not yield a row or record. |
| M03 `ignore-open-identity` | identity: replace a checked history before open; must fail, never import replacement bytes. |
| M04 `trust-summary-id-cwd` | summaries: wrong id and wrong cwd are excluded/not-found. |
| M05 `cap-before-sort` | listing: 51 valid sessions, newest last in enumeration, must survive top 50. |
| M06 `missing-date-is-oldest` | listing: `[old,missing,new]` and >64 mixed dates must retain Node's comparison behavior. |
| M07 `active-id-truthy-fallback` | active: `{id:'',sessionId:'T'}` must not mark T maybeOpen. |
| M08 `trim-query-interior` | tags: wrapped CRLF plus interior spaces/newlines must keep the exact inner text. |
| M09 `synthetic-truthiness` | tags: synthetic_reason false/0/empty must be skipped, not admitted. |
| M10 `drop-without-detach` | dropped: intervening assistant cannot attach to old prompt; next prompt is clipped. |
| M11 `tool-count-one` | content: tool_calls with three arbitrary entries must add three. |
| M12 `omit-budget-flag` | budgets: 201 turns or >2 MiB must set omittedBefore after suffix loss. |
| M13 `allow-all-files` | copy-allow: auth/lock/unknown stay uncopied and contribute to skipped. |
| M14 `omit-second-exists` | copy-raced: injected empty target dir must yield grok-exists, not rename success. |
| M15 `keep-copy-after-verify-error` | verify-failed: target must be removed and the error must be grok-load-failed. |
| M16 `ignore-load-capability` | acp: loadSession false/string must fail without sending session/load. |

## Limits and completion

- After transport adoption the wait after EOF is bounded by the overall timer (10 ms slices) and a write by its own
  overall budget (a stated latency limit, not P7; G10-TIMER-LEFT). A new request after a write deadline closed stdin
  first takes the expired overall timer (`transport::expired`), so the result is timeout as in TS. `ev_remove`/close can still block while reaping/joining pipes that a descendant
  inherited; that is the remaining P7 limit (G10 §2.5), with no hidden importer workaround.
- Windows folderKey slash conversion/lowercasing, filesystem names/identities and taskkill are outside this
  benchmark's P5 platform domain. A Windows build alone does not establish Windows import parity.
- Source metadata checks are not a snapshot: contents can change in place without changing inode. There is a
  TOCTOU window after the second target lstat and before rename; ordinary POSIX rename can replace an empty
  directory created in that window. Preserve that limitation. Target ancestors are not plainChain-validated in
  TS; do not silently harden them. A colliding temp directory may be removed by the existing failure cleanup.
- Full L3-6 acceptance requires the fixed/four-pair/Q/mutant gates and the G prerequisites named above; a pure
  parser or successful happy-path fake handshake alone is not completion. This design runs none of those gates.
  Scheduling is settled by D1-D5: G10-PROC-ENV lands first; the other G items are follow-ups. None authorizes
  moving decisions into Rust.
