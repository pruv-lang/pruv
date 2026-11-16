# L3-7 OpenCode import: implementation design

This implements `docs/OPENCODE_IMPORT_SPEC.md`; that specification wins. The reference is
`../grok/reference/{opencode,tail,plan,stubs}.ts`, pinned at Lair `744d14af`. Do not edit or
copy the pin. This document specifies future implementation, not completed verification.

Use Grok's borrowed-array, delta-step arrangement. Pruv owns paths, admission, SQL text and
parameters, row conversion, export control, JSON conversion, plans and results. Rust owns
OS/SQLite calls, opaque handles, byte storage and packet rendering. There is no host
`list_opencode`, `run_export`, title cleaner, row filter or plan selector. One request is
outstanding at a time; this importer needs neither Grok's four listing slots nor its RPC.

## 1. Files, interfaces and shared packages

Paths below are relative to `bench/l3/opencode/`. Sizes are approximate source lines, not caps.

| File | Responsibility / interface | Lines |
|---|---|---:|
| `pruv.json` | G4b root `l3_opencode_root`, version `0.1.0`, root `root.pruv`; dependencies below | 25 |
| `root.pruv` | Public `opencode_begin` / `opencode_step`; validate packets, match completions, dispatch phases | 250 |
| `packet.pruv` | Config/control/request/completion/result constructors and checked shapes; bank patches | 500 |
| `util.pruv` | Checked pool appends, JSON string access, one-pass child-index collection, bounded decimal/IEEE helpers | 220 |
| `path.pruv` | `data_dir`, `db_path`, `folder_key`, POSIX `join`/`dirname`, separator-aware `contained` | 230 |
| `admit.pruv` | `advance`: thread check, DB parent realpath/lstat/file realpath, read admission | 200 |
| `sql.pruv` | `advance`: schema queries, required-name sets, selection and count statement lifecycle | 280 |
| `rows.pruv` | `decode`: SQL row validation, `updated_number`, `clean_title`, `db_iso` | 260 |
| `list.pruv` | `advance`: count iteration, close, single wall clock observation, list/rows result assembly | 160 |
| `export.pruv` | `advance`: temp file, spawn, deadline/TERM/KILL/give-up, error precedence, cleanup | 350 |
| `entry.pruv` | `text_of`, `feed`: one validated message, attachment gate, collector calls | 230 |
| `convert.pruv` | `decode`: parse entire document, validate every message, metadata, collector, final packet | 230 |
| `plan.pruv` | `select`, `fit`, local UTF-16-unit prefix; direct plan and export-plan selection | 200 |
| `host.rs` | Primitive dispatcher, handle tables, delta-bank driver, renderer, test schedule/trace | 1,200 |
| `host/Cargo.toml`, `host/Cargo.lock`, `host/src/lib.rs` | Thin `opencode_host` library including `host.rs`; exact locked SQLite build | 40 + lock |
| `harness.rs`, `str_adapter.rs` | Link generated `sol`; existing byte/str ABI adapter pattern only | 150 |
| `common.py` | Synthetic trees/DB images/sidecars, process execution, trace parsing and snapshots | 350 |
| `test_impl.py` | Fixture expansion, verified four-pair build, selective tests and expected assertions | 400 |
| `fixtures/_defaults.json`, `fixtures/F01_paths.json` … `F16_utilities.json` | Recipes and named cases in section 7 | 16 groups |
| `fake_exporter.py` | Executable scenario-driven fake; sibling scenario/log, exact CLI and real fd checks | 180 |
| `reference.mjs` | Node version check, pinned exports, error envelope, serial per-case restoration | 160 |
| `reference-hooks.mjs`, `reference-primitives.mjs` | Scoped loading, real SQLite/FS/spawn wrappers, identical schedule language | 600 |
| `oracle.py` | Exact comparison and only OpenCode P1–P13 policy handling | 350 |
| `q.py`, `q_mutants.py` | Seeded 2,000 cases; M01–M16 isolated source mutations | 400 + 200 |
| `README.md`, `Q_README.md`, `IMPL_NOTES.md` | Commands, actual results, ABI implementation notes; no claimed unrun results | 200 total |
| `work/**` | Generated builds, private case trees, logs and reports; disposable | generated |

No edits to shared packages, compilers, other pieces, or pinned TS are planned. Root declares
the local modules and these direct dependencies, each as
`"NAME": {"path":"../pkgs/NAME","version":"0.1"}`:
`json, parse, jstr, access, text, time, names, bytes, turns`. `lex` is transitive through
`parse`. Do not use `bench/l3/lib`, JSONL framing, `args` or `transport` for this protocol.

The following APIs were checked in `bench/l3/pkgs/<module>/<module>.pruv`. These are the
shared functions this design relies on; other function names in this document are local
interfaces to implement, not presumed package APIs.

| Module | Existing functions / types used | Constraint |
|---|---|---|
| `json` | `JsonDocument`, `JsonParse`, `check_tape`, `limit_input_bytes`, `limit_records`, `limit_depth`, `err_input_too_large` | 16,777,216 bytes, 1,048,576 six-word records, depth 128 |
| `parse` | `parse(input: [u8]) -> json::JsonParse` | Whole document; distinguish Failed from Parsed |
| `access` | `root`, `kind`, `field`, `raw_span`, `array_length`, `array_at`, `string_text`, `string_is`, `boolean`, `number` | Missing id is not a value; key lookup preserves UTF-16 identity and last-key semantics |
| `jstr` | `string_units`, `string_utf8`, `adapter_probe` | Decode JSON values with P2; adapter is byte/str representation conversion only |
| `text` | `trim`, `trim_span`, `is_blank`, `same`, `one_line`, `clip_bytes` | ECMAScript whitespace; one_line counts UTF-16 units, clip_bytes counts UTF-8 |
| `time` | `iso_of`, `integer_time`, `format_iso`, `time_none`, `time_min_ms`, `time_max_ms` | Export P4 domain; absent status 1, defect -4; present timestamp has 24 bytes |
| `names` | `valid_thread` | ASCII first alphanumeric, remainder alphanumeric/underscore/hyphen, 1–128 |
| `bytes` | `copy_span(inout dst, at, src, lo, hi)` | Bounds proved before copy |
| `turns` | `new_state`, `new_slots`, `new_buffer`, `grow`, `begin`, `answer`, `tool`, `finish`; `Collected` | Four state words, 1,600 slot words, grow before begin; finish with base 0 |

`turns::skip_user` exists but must NOT be used for an empty/synthetic-only user here: it
would mark the next prompt clipped. Keep a separate OpenCode attachment boolean instead.
`text::one_line` must NOT implement plan slicing: plan text preserves internal whitespace.

All stateful modules expose the same `advance` arguments as `opencode_step` below and
return a step packet. `rows::decode(queryWords, queryBytes)` returns validated seven-word
rows plus their text pool as an internal struct. `convert::decode(bytes, fallbackTitle)`
returns a read result packet. `plan::select(doc, arrayId, inputKind)` returns an internal
plan value (presence, time, item rows, text, more); inputKind 0 means export messages,
1 means direct `opencodePlan` input `{parts, at?}[]`. `entry::feed` takes document/message
id and caller-owned collector arrays via internal `inout`; it returns attachment state
and the last eligible model pair. No public `inout` or buffer-containing struct ABI.

## 2. Public step ABI and storage

```pruv
pub fn opencode_begin(cfg: [i64], cfg_text: [u8]) -> [i64] ! alloc
pub fn opencode_step(
  cfg: [i64], cfg_text: [u8], ctl: [i64],
  query_words: [i64], query_bytes: [u8], rows: [i64], text: [u8],
  document: [u8], saved_result: [i64],
  event: [i64], completion: [u8]
) -> [i64] ! alloc
```

Modes: 0 list, 1 read, 2 rows (`readOpencodeRows`), 3 dataDir, 4 folderKey, 5 plan.
The host supplies raw inputs, never precomputed DB paths or selected rows. For plan mode,
the input document is the JSON array accepted by pinned `opencodePlan`, with `at` already
a string or absent; it is not a second conversation converter. Utility modes perform no I/O.

Config has 32 header words, then four words per environment pair. All refs use `cfg_text`.

| Words | Meaning |
|---|---|
| 0–5 | version 1, mode, export timeout ms (20,000), TERM grace ms (1,500), KILL grace ms (1,000), env mode (0 inherited, 1 full replacement) |
| 6,8,10,12,14,16 | start of two-word refs: source.home, folder, thread, binary, tempRoot, OS homedir |
| 18 | environment pair count N |
| 19–20 | direct plan JSON ref (absent except mode 5) |
| 21–31 | reserved zero |
| 32+4i | key start, key length, value start, value length |

Exactly `32+4N` words. The supplied table is the effective environment (a snapshot of the
parent for mode 0, only supplied keys for mode 1). Keys are unique, no NUL or `=`; values
are strings with no NUL. Missing is absence from the table; empty stays empty. Pruv looks
up XDG_DATA_HOME/OPENCODE_DB itself. For child mode 0 the host inherits the same parent;
for mode 1 it clears the environment and installs exactly the requested table. Test timer
overrides are config only, recorded on both sides, positive and restored per case.

Banks are opaque and independently capped at `B=2^31` elements: 0 query words, 1 query
bytes, 2 converted rows, 3 persistent text, 4 export/direct-plan bytes, 5 saved final-result
words. Allocation/length arithmetic checks against B before addition/multiplication.
Overflow is explicit `CoreError/InputTooLarge`, not a new count limit or partial success.

`ctl` has 64 words. Initial handles/refs are absent; reserved words stay zero.

| Words | Meaning |
|---|---|
| 0–7 | version, mode, phase, next request id, pending id (0 none), pending opcode, expected event (0 empty/1 completion/2 terminal), continuation phase |
| 8–11 | DB handle, selection statement handle, count statement handle, output file handle (-1 absent) |
| 12–15 | child handle, timed-out flag, settled flag, observed child-ended flag |
| 16–19 | monotonic start, deadline, last monotonic observation, termination stage (0 waiting/1 TERM/2 KILL) |
| 20–23 | next validated row, validated row count, archive-column flag, return mode after DB work |
| 24,26,28,30 | two-word refs in bank 3: DB path, resolved parent, temp dir, export path |
| 32,34,36 | fallback title, first selected directory, first selected thread refs in bank 3 |
| 38–39 | saved-result-present, export stat size |
| 40–63 | reserved zero |

Phases: 100–199 admission; 200–299 schema/select; 300–399 counts/list/rows and DB close;
400–499 export; 500 conversion/plan; 600 dataDir/folderKey; 999 terminal. The owner changes
phase before a handoff. Read DB close finishes before folder/binary checks and export.
On DB error, save its result and go through DB close; close failure is ignored.

Step packet, all i64 (byte regions are 0–255):

```
[0x4f435331, 1, action, 64, patchCount, requestCount, literalWords, literalBytes, resultWords, 0, 0, 0,
 ctl[64], patches[8*patchCount], requests[12*requestCount], wordPool, bytePool, result]
```

Length is `76+8*patchCount+12*requestCount+literalWords+literalBytes+resultWords`.
Action 0 awaits exactly one request; 1 is done with a final result and no pending request;
2 continues after patches with no request and a changed phase or monotonically advanced
cursor. Root must reject wrong version/shape, duplicate completion, stale id and terminal
reentry. Request ids increase; guard exhaustion rather than wrapping.

Patch row: `[kind, bank, dst, count, source, src, 0, 0]`. Kind 0 resizes to `count` (dst 0,
source 0), keeping prefix and zeroing new space; kind 1 copies `count` elements. Sources:
1 event words, 2 completion bytes, 3 literal words, 4 literal bytes. Units are bank elements.
Apply in order before issuing the next request; retain event storage until all patches finish.
This avoids copying whole database/export banks through the small control block.

## 3. Primitive wire contract

Request row: `[id, opcode, stage, flags, handle, a, b, c, argStart, argWords, 0, 0]`.
Arguments are words in the step wordPool; their string/blob refs point into its bytePool.
Unused scalars are zero, unused handle -1. No strings encoded as JSON fragments.

Completion is event header plus payload words, and a separate byte array:

```
[1, id, opcode, status, errorClass, nativeCode, payloadWords, byteCount,
 codeStart, codeLen, messageStart, messageLen, 0, 0, 0, 0, payload...]
```

Status 0 success / 1 error. ErrorClass 0 none, 1 OS, 2 SQLite, 3 Node-binding-range,
4 host-capacity, 5 invalid-protocol. Code/message refs use completion bytes. Native code is
positive errno or SQLite extended code; code is e.g. `ENOENT`, `SQLITE_CORRUPT`,
`ERR_OUT_OF_RANGE`, `InputTooLarge`. Do not compare diagnostic prose. A failed call has
no successful payload/rows. Protocol bugs are harness failures, never an allowed policy.
The empty continuation event is a 16-word version-1 header with id/opcode/payload/bytes 0
and absent code/message refs. It is legal only after begin/continue.

Typed SQL value is exactly four words `[kind,a,b,c]`:

| Kind | Encoding |
|---|---|
| 0 NULL | `0,0,0` |
| 1 INTEGER | signed i64 in a; b=c=0 |
| 2 REAL | unsigned high32 and low32 IEEE-754 binary64 bits in a,b; c=0 |
| 3 TEXT | UTF-8 start,length in a,b; c=0 |
| 4 BLOB | raw byte start,length in a,b; c=0 |

INTEGER results outside ±9,007,199,254,740,991 fail the whole query with binding-range,
matching Node's default Number reads. Keep REAL bit patterns, TEXT and BLOB distinct.
SQLite TEXT decoding follows the Node binding's UTF-8 replacement behavior; BLOB is never
decoded or stringified. Do not turn a finite fractional REAL into an INTEGER in the host.

| Opcode | Request fields / argument words | Success payload |
|---|---|---|
| 1 realpath | args `[pathRef]` | `[resolvedPathRef]` |
| 2 lstat | args `[pathRef]`, no following final symlink | `[kind,size]` |
| 3 db_open_readonly | args `[pathRef]` | `[connection]` |
| 4 db_query | flags 0 prepare, 1 all, 2 get; see below | prepare `[statement,C,columnRefs...]`; execute `[C,R,columnRefs...,values...]` |
| 5 db_close | handle connection | empty |
| 6 mkdtemp | args `[prefixRef]` (full path ending `opencode-`) | `[directoryRef]` |
| 7 open | args `[pathRef]`; flags 1 = write/create/exclusive; a=384 (`0600`) | `[fileHandle]` |
| 8 spawn_stdout_fd | handle output file; args below | `[childHandle]` |
| 9 observe | handle child; a absolute monotonic deadline ms | observation below |
| 10 kill | handle child; a=15 TERM or 9 KILL; flags=1 POSIX isolated group target | `[sent]` (0 already ended, 1 attempted successfully) |
| 11 close | handle output file | empty |
| 12 stat | args `[pathRef]`, follows symlinks like Node stat | `[kind,size]` |
| 13 read | args `[pathRef]`, full readFile-equivalent to EOF, raw bytes | `[dataRef]` |
| 14 rm | args `[pathRef]`, flags=3 recursive+force | empty |
| 15 clock_now | flags 0 monotonic / 1 wall-clock epoch ms | `[milliseconds]` |

`[pathRef]` denotes two words, not one. File kind: 0 unknown, 1 regular, 2 directory,
3 symlink, 4 FIFO, 5 socket, 6 char, 7 block, 8 other. Sizes are nonnegative i64 facts;
overflow is a host-capacity error. No inode-identity check is added to DB admission.
Read is a generic file operation, not a parser: do not stop at stat size or 16/32 MiB.
It can fail explicitly at the bank bound; otherwise read to EOF and let Pruv decide.

For db_query prepare, handle is connection and args are `[sqlRef]`. Return an opaque
statement and column names, even when no rows exist. For all/get, handle is statement and
args are `[P, value0[4], ..., valueP-1[4]]`. Execute binds fresh values, preserving row order;
all materializes the whole result atomically, get reads at most one row. Column refs are
two words each; values are row-major, so execute payload has `2+2C+4*C*R` words. Finalize
or reset after execution; close drops all statements. Repeated get uses the same logical
statement. Rust can use `Connection::prepare_cached` plus a statement-id/SQL registry;
keep cached statements available across calls and do not retain self-borrowing Rust structs.
Prepare-only must really prepare: a count prepare can fail even with zero validated rows.

Open uses SQLite read-only flags only (no create, immutable URI, copied DB, transaction,
schema repair or automatic retry policy). Use `rusqlite` with `bundled`; the host owner
must select and lock the released `rusqlite`/`libsqlite3-sys` pair closest to SQLite 3.53.4,
record the exact crate versions and both `SELECT sqlite_version()` values in build metadata.
Do not invent a release number from this document. Version probes use a separate metadata
connection before cases and never appear as importer statements. Version differences do
not authorize sorting or binding differences. Use the locked crate in all four runners.

Spawn arguments are `[binaryRef,cwdRef,argc,envMode,envCount,shell,stdinMode,stderrMode,
detached,windowsHide, argvRefs...,envPairs...]`, where the first two refs occupy four
words, argv refs two each and env pairs four each. Required scalars: argc=3, shell=0,
stdinMode=stderrMode=0 (ignore), detached=1 and windowsHide=1 to match the pinned POSIX
call. argv is exactly `--pure`, `export`, thread; no scenario arguments or shell command.
The child inherits/duplicates the already-open file as fd 1; no pipe or copying thread.
`FileHandle.fd` in the TS wrapper must be the real numeric descriptor, not a logical id.
Opcode-8 flags=0 selects this fd mode. The host also accepts flags=1 as a generic pipe
mode for M10 only, draining that pipe to a discard sink. Baseline Pruv always emits 0.
Thus M10 is a normal executable wrong-stdio mutation caught by the fake's fd check and
trace, not an invalid-protocol/build error falsely counted as a kill.

Observe success payload is `[eventKind,codePresent,exitCode,signalNumber,monoMs,exited]`:
eventKind 0 deadline reached, 1 child close, 2 child error. Error events carry the raw error
in the header's error fields but have status 0 (the observation itself succeeded). Close
without code has codePresent=0; success requires codePresent=1 and exitCode=0. Preserve
error versus close order. `exited` reflects exitCode/signalCode availability, so a deadline
can precede close after an exit; skip signaling if already ended but still await close.
The host queues observations from spawn onward, including an error before the first observe.
An actual synchronous spawn throw is opcode-8 error. Node's asynchronous missing-program
error is opcode-8 success followed by eventKind 2; Rust must adapt its synchronous OS spawn
error to that event form where needed for the matching fixture, without changing its class.

The host wait returns the first queued child event or the deadline, not a final timeout
decision. Deadline completions include a clock observation; spurious early wakeups may
reissue observe with the same deadline, never restart the budget. Signal errors are raw
completions; Pruv ignores them, as TS does. Use isolated POSIX children matching the pin's
detached flag and negative-pid signal target; this is not a promise to manage descendants.
Fixtures have only the direct child. No external cancellation or Windows behavior is tested.

### Export transition table

| State / event | Pruv action and next state |
|---|---|
| admitted read | mkdtemp, then open `join(dir,'export.json')` wx 0600, then spawn |
| spawn success | clock_now(monotonic); save start; deadline=start+20,000; observe(deadline) |
| waiting + close | zero => success, other => read-failed; settle |
| any unsettled wait + child error | engine-missing, even if timeout was recorded; settle |
| waiting + deadline | record timeout once; TERM unless ended; clock observation after send; set deadline=now+1,500; observe |
| TERM wait + close | settle with stored timeout, even code zero |
| TERM wait + deadline | KILL unless ended; clock observation after send; deadline=now+1,000; observe |
| KILL wait + close | settle with stored timeout |
| KILL wait + deadline | settle with stored timeout (give-up) |
| settled | cancel further wait registration; close output once |
| output close failure | replace pending success/wait error with raw close error |
| close success + pending failure | skip stat/read; cleanup |
| close success + wait success | stat; if size >33,554,432 create oversized result without read; else read |
| stat/read failure | save raw OS error; cleanup |
| cleanup | rm recursive+force if mkdtemp succeeded; ignore its failure; then convert bytes or return saved result |

Mkdtemp failure is raw and has no cleanup target. Open failure still removes the directory.
Spawn exception becomes engine-missing but still closes the file. JSON parsing is after rm,
as in `runExport` returning before `readOpencode` parses. Size checked after successful wait
only; growth after stat is read to EOF and is not a second oversized decision. Oversized
result is `{turns:[],omittedBefore:true}` and only a nonempty DB fallback title.

Deadlines use monotonic clocks, list freshness uses wall time. No fixed poll-count timeout.
Grok/Codex use shared transport timer observations and bounded waits (`ev_proc_wait_for`);
that supports this observation-driven approach, but its JSONL transport and teardown are
not the export protocol and must not be reused here. On give-up the host may retain a
reaper for its own child; it must not delay/replace the returned timeout or issue unrequested
signals. Watchdog cleanup is a failed observation, outside the passing trace.

## 4. Pruv decisions and module handoffs

### Paths, admission and SQL

`path` implements Node POSIX semantics locally: join ignores empty segments, normalizes
`.`/`..` and repeated `/`, preserves required trailing separator behavior, and returns `.`
for an empty join. dirname must reproduce POSIX root/double-leading-slash cases. It is not
Rust PathBuf joining (a later absolute component must not silently replace earlier parts).
Absolute OPENCODE_DB is used as given; only relative/default paths are joined. Trim env
with `text::trim`. Exactly `:memory:` is absence. Relative parent traversal is allowed.
dataDir uses OS homedir only for its fallback; list/read/rows use supplied source.home.
folderKey removes all trailing `/` unless doing so empties the input; no normalization,
backslash conversion or case fold. Empty and all-slash inputs retain their exact value.

Read validates thread before any filesystem work. Admission issues realpath(dirname(DB)),
lstat(DB), and only for a regular non-symlink, realpath(DB). Compare resolved file against
resolved parent plus a separator (do not add a second separator for `/`). Every failure
here means absence: list/rows empty, read not-found. Open the original DB path, not its
resolved substitute. A symlinked parent/hard link may pass; a sibling prefix must not.

The exact query templates (single spaces, no newline) are:

```
SELECT name FROM pragma_table_info(?)
SELECT s.id AS id, s.directory AS directory, s.title AS title, s.time_updated AS updated FROM session s WHERE s.parent_id IS NULL
SELECT COUNT(*) AS n FROM message WHERE session_id = ?
```

Prepare schema SQL and all(TEXT `session`). Require string names `id,parent_id,directory,
title,time_updated`. If any are missing, fail read-failed/detail `columns` immediately:
do NOT issue the message-schema query (TS short-circuits). Otherwise prepare schema SQL
again and all(TEXT `message`); require string `session_id`. Nonstring names do not count.
Remember whether session has `time_archived`.

Append to the SELECT prefix, in this order:

1. ` AND s.time_archived IS NULL` only if that column exists.
2. ` AND EXISTS (SELECT 1 FROM message m WHERE m.session_id = s.id)`.
3. Read: ` AND s.id = ?`, TEXT thread. List: ` AND s.directory IN (?, ?)`, TEXT base,
   TEXT base+`/`, where base=folderKey(folder). Rows: no filter.
4. List only: ` ORDER BY s.time_updated DESC LIMIT ?`, parameter JS Number 50.

For trace/binding equivalence, JS Number parameters use REAL (binary64 bits for 50.0),
not INTEGER: Node binds Number as double and bigint as integer. This does not change
SQLite LIMIT behavior. Log parameter kinds before SQLite affinity; never interpolate.

Execute all, then `rows::decode` validates id/directory and cleans title, preserving order.
No converted-value sorting, backfill, tiebreaker, early ID LIMIT 1, or unfiltered limit.
Even read selects/validates all rows before taking the first validated row. List prepares
COUNT once even for zero validated rows, then get(TEXT id) once per validated row on the
same connection. Failed all/get invalidates the operation. Close is attempted after work
or error and its error ignored. Unfiltered rows never count and never sample the list clock.
No transaction: a scheduled external DB mutation between selection and counts is observable.

`rows::updated_number` implements P9 in Pruv: safe INTEGER; finite integral safe REAL
(decode sign/exponent/mantissa from the two limbs, reject nonzero fractional bits); trimmed
TEXT `[+-]?[0-9]+` with checked accumulation; NULL/blank TEXT => zero; anything else =>
zero. SQL ordering is already finished. Titles only use TEXT; prefix tests happen before
`text::one_line(title,120)`. Retain empty titles in rows/list.

For real COUNT(*) the result is a safe nonnegative INTEGER, or binding-range error. For
test-injected result rows, Pruv checks presence/kind/range; do not let the host manufacture
an accepted count. Keep zero present. Only matched contract-preserving count replacements
are passing equality fixtures; arbitrary forged count types are adapter-negative tests.

List samples wall Date.now once after DB close, including absent DB => empty list. A DB
error returns before that observation. maybeOpen is only true when raw updatedMs>0 and
now-updatedMs<120000, including future timestamps. isoOf uses seconds below 1e11, even
though the field is called updatedMs; freshness still uses the raw value. `db_iso` uses
`time::integer_time`/`format_iso` within their domain, but must locally format positive
extended years through JS TimeClip's 8,640,000,000,000,000 ms (signed six-digit year prefix)
when needed. Shared `time` stops at year 9999; silently dropping larger valid DB dates is
NOT P4/P9. Beyond TimeClip omit updatedAt. This extension belongs in rows, not the package.

Read closes DB, takes the first validated row, compares folderKey, then checks binary and
tempRoot truthiness (no trim). Wrong folder/missing row => not-found; missing/empty binary
or tempRoot => engine-missing. Export metadata never rechecks thread or folder.

### JSON, entries, plan and collector

Read bytes >16,777,216 after a non-oversized stat => CoreError/InputTooLarge (P5). Otherwise
parse once. Invalid UTF-8/depth/tape/JSON failures => read-failed at parse, detail `json`;
nonobject root, nonarray messages or ANY malformed message => read-failed/detail `shape`.
Never publish partial turns. Validate each message as object with object info and array
parts before role dispatch, including unknown/summary/old messages. Unused huge/exponent
number tokens remain unexamined (P10). No line splitting, tail window or line cap.

`entry::text_of` visits object parts with type `text` and string, nonblank text, keeps the
original string, joins with two LF. For users reject only boolean true synthetic/ignored.
Build the joined text before one collector call so separator/clipping semantics match.
The parser source/tape bounds bound this allocation; do not impose a smaller text cap.

Keep `open=false` initially. User sets open from human text; nonblank calls begin with
iso_of(created), blank only clears open. Unknown role changes nothing. Summary assistant
(`summary===true`) contributes nothing. Other assistants update model if both IDs are
nonempty strings (no trim/length limit), regardless of open. While open, answer with joined
text even if empty, and only completed timestamp; then call turns::tool once per object
tool part, including error/todowrite tools. No fallback from completed to created for reply.

`convert` owns collector arrays and grows before begin when
`65584*(begun%200)+65584 > len(buffer)`. `entry::feed` may return a grow-needed status only
before changing any state, or the owner can grow before invoking its user branch. Carry
the full package ring/flag/ordinal contracts, not only buffer bounds. Begin count is bounded
by message count; tools by part count. Call finish(base=0), then append title/model/plan
bytes and relocate their refs; do not prepend metadata to a large collector base.
omittedBefore is exactly collector trimming (except the explicit oversized branch).

After all messages validate, plan scans messages newest first, parts last first. In export
mode it accepts only nonsummary assistants; plan time is completed ISO or created ISO.
Direct plan mode takes each entry's supplied optional at. Candidate: object tool part,
tool `todowrite`, object state, status not exactly `error`, object input, todos array.
The first candidate ends the search even if no items survive. Nonarray todos/error skips.
Fit keeps object entries with string content, ECMAScript trims, drops blanks, preserves
internal whitespace, takes a scalar-safe prefix of 200 UTF-16 units (P8), accepts status
pending/in_progress/completed only. Count ALL surviving entries, store first 30 and set
more=count-30 only if positive. Empty fit means absent plan, not an empty plan or fallback.
Final title is cleanTitle(export info.title) or DB fallback; omit empty title/model.

## 5. Final result packet and trace

All modes use the same result header, then ordinary rows, plan items and UTF-8 bytes (one
byte per i64). Refs always address the final byte pool, never banks or request literals.
`(-1,0)` alone means absent; `(0,0)` is a valid present empty string in an empty pool.

| Words | Meaning |
|---|---|
| 0–5 | magic `0x4f435231`, version 1, status (0 ok/1 error), mode, row count N, row width W |
| 6–7 | plan item count K, text byte count T |
| 8–9 | omittedBefore (0/1), planPresent (0/1) |
| 10,12,14,16 | two-word refs: title, model, scalar value, plan at |
| 18 | plan more (0 means omitted) |
| 19–22 | error stage, result error class, code, native code |
| 23,25,27 | refs: raw error code, error name, detail |
| 29–39 | reserved zero |
| 40 onward | N rows of width W, K plan items of width 3, T bytes |

Length `40+N*W+3*K+T`. Result error classes: 0 none, 1 EngineImportError, 2 RawOsError,
3 CoreError. Codes: 0 none, 1 not-found, 2 read-failed, 3 engine-missing, 4 timeout,
5 InputTooLarge, 6 InternalInvariant. Errors have no rows/plan/value/metadata; error refs
may contain the small detail `columns`, `json`, `shape`, or a diagnostic. Native/raw cause
facts survive DB-error wrapping, but output class is EngineImportError/read-failed.
InternalInvariant is a defect and never a P1–P13 success. Renderer faults are HarnessError
outside this packet, not conversation errors.

Stages (stable numeric order): 0 input, 1 db-parent, 2 db-lstat, 3 db-realpath, 4 db-open,
5 schema-session, 6 schema-message, 7 select, 8 count-prepare, 9 count, 10 db-close,
11 read-admission, 12 list-clock, 13 temp-create, 14 output-open, 15 spawn, 16 wait,
17 term, 18 kill, 19 give-up, 20 output-close, 21 output-stat, 22 output-read,
23 cleanup, 24 parse, 25 shape, 26 convert, 27 plan, 28 result. A stored error retains
its origin stage across cleanup; timeout always has wait stage, not cleanup/give-up.

| Mode | Ordinary row / scalar rendering |
|---|---|
| list (W=10) | `[threadRef,titleRef,updatedAtRef,messagesPresent,messages,maybeOpen,0]`; each Ref=2 words. title present even empty; omit absent updatedAt/messages and false maybeOpen |
| read (W=10) | `[promptRef,replyRef,createdAtRef,completedAtRef,tools,clipped]`; emit prompt/reply/tools, optional times, clipped only if true |
| rows (W=7) | `[idRef,directoryRef,titleRef,updatedMs]`; all four fields present |
| dataDir / folderKey (W=0,N=0) | present scalar value ref, including empty folderKey |
| plan (W=0,N=0) | planPresent 0 => absent result; 1 => last-plan object using plan area |
| error | W=N=K=0; only mode/error fields |

Plan item `[textStart,textLen,status]`: status 0 absent, 1 pending, 2 in_progress,
3 completed. PlanPresent 0 requires K=more=0 and absent at; present requires 1<=K<=30.
Read exposes it as optional `lastPlan`; direct plan exposes the optional value plus an
explicit `present` bit in the harness envelope. Host validates all refs and enum domains
and escapes strings; it makes no omission decisions beyond the packet's presence flags.

Trace is JSONL `{"trace":{...}}` on stderr, result envelope on stdout. Each record has
`v:1, seq, event, id, op, stage, handle, args, result, error`. event is issue, complete,
barrier-before-call or barrier-after-result-before-delivery. Missing fields are null.
Arguments/results use named fields but exactly the wire values. SQL typed JSON values are
`{kind:"null"}`, `{kind:"integer",decimal:"..."}`, `{kind:"real",bits:"16 hex digits"}`,
`{kind:"text",text:"..."}`, `{kind:"blob",hex:"..."}`. Trace column names AND ordered rows.
Large read/export bytes use length+SHA-256 and a raw artifact; SQL/text payloads may use
the same artifact convention, without losing type or ordering information.

Compare SQL prepare/all/get sequence, SQL text, typed params/results, connection/statement
lifetime, count placement, admission calls, temp prefix/parent, wx/mode, child argv/cwd/
effective env/stdio, close/stat/read/rm order, error stage/class/code, and final trees.
Normalize only fixture roots, opaque handles/request ids and random temp suffixes. Assert
prefix/location before suffix normalization. Do not normalize slash differences in inputs,
sort rows, erase failed calls or move cleanup across a failure.
Clock values use per-run monotonic origins and semantic observations (wait-start, TERM,
KILL, give-up); compare requested budgets and event order, not scheduler latency. Frozen
list wall time is exact. A host poll implementation must coalesce only its internal waits
into the single observe request, never discard an error or timeout boundary. This is trace
representation, not a policy permitting early/late timeout decisions.

## 6. TS instrumentation and equal fault injection

Use Node 26.4.0, record V8 and SQLite. `reference-hooks.mjs` resolves imports into
`../grok/reference/`, redirects ONLY pinned opencode/tail filesystem imports, opencode's
`node:sqlite`/`node:os` imports, and stubs.ts's `node:child_process` import to local wrappers.
Shared-engine-import and unrelated engine stubs resolve to the pin. The shim's own builtin
imports stay real. Transpile plan.ts only using explicitly supplied `GROK_TS_TRANSPILER`
(its parameter property needs more than strip-types); no user's external checkout default.

DatabaseSync facade opens real Node SQLite readOnly, returns statement facades, and
intercepts prepare/all/get/close. Log prepare even if never executed. Obtain storage kinds
without changing queries: a second materialization is forbidden. Node exposes INTEGER and
REAL both as Number by default; set the wrapped statement to `setReadBigInts(true)` for
the single real execution, record bigint as INTEGER, then reproduce the default safe-range
check/conversion before delivering rows to pinned TS. Number is REAL, string TEXT, null
NULL, Uint8Array BLOB. Use statement column metadata for names even with zero rows.
An out-of-range integer throws the equivalent default binding error and discards all rows.
Verify this shim against native default reads in the narrow SQLite host tests, including
REAL 1.0 and unsafe INTEGER. Do not use `typeof(...)` SQL, extra queries or stringify cells.
Preparation and statement reuse stay visible separately from execution.

The FS shim delegates realpath/lstat/mkdtemp/open/stat/readFile/rm and wraps close; readFile
logs the raw Buffer then returns Buffer.toString('utf8') to TS. A close injection actually
releases the underlying file during harness teardown but reports the scripted close failure;
teardown events are separated from importer trace. `node:os.homedir` returns fixture home.

Spawn still calls pinned spawnEngine stub and real child_process.spawn. Record options and
subscribe to spawn/error/exit/close without changing handler order. Capture direct kill
calls through narrowly scoped process.kill/child.kill wrappers for this known child only;
do not kill unrelated processes. Spawn exceptions versus emitted error events are separate
fault actions. For deterministic timer-order cases, a test child facade may queue scripted
events, but mark these cases `scripted`; real-file stdout and real 20 s/TERM/KILL probes
must use an actual child and cannot be credited by replay.

During a serial case only, wrap Date.now (list freeze) and setTimeout/clearTimeout. Retain
native timers for export. Timer registration captures clock_now and observe(deadline);
firing captures deadline completion; child error/close captures observe completion and
cancels pending timer records. TERM/KILL timer registration records the grace deadline.
Only timers registered from the active pinned waitExport call participate; shim/harness
watchdogs use saved native functions. Restore globals and IMPORT_LIMITS.rpcMs in finally.
This records TS's existing state machine; it does not replace waitExport. Default 20,000 /
1,500 / 1,000 constants remain; fast cases may override rpcMs only. Grace shortening, if
ever needed, belongs only in explicit timer-script tests with both effective values logged.

Schedule format follows Grok's barriers, but not Grok's policy rules:

```json
{
  "rules": [{
    "match": {"op":"db_query", "phase":"get", "sql":"SELECT COUNT(*) AS n FROM message WHERE session_id = ?", "occurrence":1, "barrier":"before-call"},
    "actions": [{"kind":"db-exec", "path":"$SRC/opencode.db", "sql":"INSERT INTO message(session_id) VALUES (?)", "params":[{"kind":"text","text":"A"}]}]
  }, {
    "match": {"op":"close", "path":"$TMP/opencode-@1/export.json", "occurrence":1, "barrier":"after-result-before-delivery"},
    "error": {"class":"OS", "code":"EIO"}
  }]
}
```

Occurrence is counted per (op, phase, logical path or SQL); defaults are occurrence 1,
before-call. Rules run in array order: actions, optional hold, then error/observation
replacement. First error stops; before-call failure skips native call and after rules.
After rules run only after successful native operation. Sync SQLite rules cannot hold
waiting for another importer call; allow immediate DB/file actions, errors or typed-result
replacement only. The same logical SQL/path matcher and barriers run on Rust and TS.

Actions: replace-file, symlink, hardlink, mkdir, unlink, append, truncate, db-exec. Bytes
come from fixture recipes. `return` supplies the opcode's typed payload only in explicitly
scripted adapter/clock/child tests. Real runs are never replaced with preselected rows or
success. Child script events are close/error/deadline with relative times and explicit
ordering; signal actions record TERM/KILL observations. Async holds have named release
events, not sleeps; unreleased holds are HarnessError. Test mutations execute via separate
setup connections and are labeled schedule events, never importer SQL.

Fake exporter checks exact argv, cwd, sentinel env presence/absence, no shell effects,
stdin/stderr ignored, and `fstat(1)` regular file. It writes via fd 1, including >196,608
bytes/no final LF; logs to a sibling file, never stdout. Scenarios: output+exit, nonzero,
self-signal, hang with TERM exit, ignore TERM until KILL. It discovers its sibling scenario
without extra arguments. Cleanup owns only spawned PIDs; no pkill patterns. A watchdog
kill/crash/missing result is always failure, never a passing timeout or mutant kill.

## 7. Fixtures, policies and mutants

Each F file is a JSON array; `_defaults.json` supplies only omitted fields (do not replace
explicit empty strings/objects). Concrete case format:

```json
{
  "name":"F04-limit-51-invalid-newest", "mode":"list", "folder":"/work", "thread":"A",
  "sourceHome":"$SRC", "homedir":"$HOME", "env":{"mode":"full","values":{}},
  "binary":"$BIN/exporter", "tempRoot":"$TMP", "nowMs":1700000000000,
  "rpcMs":20000, "umask":18,
  "tree":[{"path":"$TMP","kind":"dir","mode":448}],
  "db":{
    "path":"$SRC/opencode.db",
    "ddl":["CREATE TABLE session(id,parent_id,directory,title,time_updated)","CREATE TABLE message(session_id)"],
    "inserts":[{"table":"session","columns":["id","parent_id","directory","title","time_updated"],
      "rows":[[{"kind":"text","text":"A"},{"kind":"null"},{"kind":"text","text":"/work"},{"kind":"text","text":"Title"},{"kind":"integer","decimal":"1700000000000"}]]}],
    "generate":{"sessions":51,"messagesPerSession":1,"invalidNewestId":"bad.id"}
  },
  "export":{"bytes":{"utf8":"{\"messages\":[]}"},"exit":0,"term":"exit"},
  "schedule":{"rules":[]}, "kills":["M06"],
  "expected":{"status":"ok","rowCount":49}, "policies":[], "slow":false
}
```

Generators expand before either runner sees the case; explicit rows override generated
rows by ordinal, not by id. Typed inserts use the same kinds as the protocol; decimal
strings avoid JSON's integer loss, REAL uses bits. Use affinity-free columns for storage
class fixtures. Create a template DB with pinned Node SQLite, then distribute identical
closed images to all sides. WAL cases retain a template writer, disable autocheckpoint,
copy DB/-wal/-shm together while quiescent, and keep private sidecar ownership; assert the
WAL-only row is visible. Do not rebuild different physical insertion order per side.

Tree kinds: dir/file/symlink/hardlink/FIFO. Bytes: utf8, hex, repeat/count, concat, blob.
Export can use raw text/hex or a structured recipe with `$raw` tokens, never float
round-trip `1e400`, escaped surrogates or invalid UTF-8. Boundary generators pad a valid
document to exact byte counts. Snapshot DB and temporary tree before/after, including
sidecars, modes and link targets; scheduled mutations are expected, unrelated changes fail.
All roots are private synthetic fixtures; never real conversations, homes or credentials.

The following names are the required generated fixture names (braced alternatives expand
into separate names); they are also the selectors used by local commands.

| Group / file stem | Concrete names and required coverage |
|---|---|
| F1 / `F01_paths` | `F01-xdg-{missing,blank,absolute,relative,unicode-trim,all-slash}`; `F01-db-{default,absolute,relative,parent-relative,memory,trim,all-slash}` |
| F2 / `F02_admission` | `F02-db-{missing,final-symlink,parent-symlink,hardlink,directory,fifo,sibling-prefix}`; `F02-{parent-realpath,lstat,file-realpath}-error` |
| F3 / `F03_schema` | `F03-{corrupt,open-error,query-error,unsafe-integer,wal-visible}`; `F03-missing-{id,parent_id,directory,title,time_updated,message-session_id}`; `F03-archive-{absent,null,zero}`; `F03-count-prepare-empty-error`, `F03-close-error-ignored` |
| F4 / `F04_selection` | `F04-exclude-child-archived-empty`; `F04-{invalid-id,empty-dir,nonstring-dir}`; `F04-limit-{49,50,51,51-invalid-newest}`; `F04-list-slash-asymmetry`, `F04-ties`, `F04-raw-storage-order`, `F04-live-count-mutation` |
| F5 / `F05_metadata` | `F05-title-{raw-prefix,leading-space-prefix,empty,collapse,cut-120,split-astral}`; `F05-count-{zero,many,error}`; `F05-p9-{integer,integral-real,fraction-real,signed-text,exponent-text,decimal-text,hex-text,blank,null,blob,unsafe-real}`; `F05-time-{zero,negative,future,seconds,extended-year}`; `F05-age-{119999,120000,120001}` |
| F6 / `F06_read_admit` | `F06-{invalid-thread,missing-row,wrong-folder,extra-slashes,binary-missing,binary-empty,temp-missing,temp-empty,db-rejected-no-spawn}` |
| F7 / `F07_spawn` | `F07-{argv-cwd,env-inherit,env-full,env-empty,ignored-stdio,spaces-no-shell,stdout-file-large}`; large export contains >196608 meaningful bytes, checked in full |
| F8 / `F08_lifecycle` | `F08-{spawn-throw,child-error,exit-zero,exit-nonzero,exit-signal,hang,term-exit,kill-required,give-up-script,error-after-timeout-script,close-overrides-timeout,stat-error,read-error,rm-error,mkdtemp-error,open-error,missing-temp-root}`; `F08-real-20000ms` slow actual child |
| F9 / `F09_parser` | `F09-bytes-{16777216,16777217,33554432,33554433}`; `F09-nonzero-oversized`; `F09-{invalid-utf8,depth-128,depth-129,tape-limit,tape-over,malformed,no-lf,root-null,root-array,grow-after-stat}` |
| F10 / `F10_shape` | `F10-{messages-missing,messages-nonarray,message-null,message-array,info-missing,info-array,parts-missing,parts-nonarray,late-malformed}`; late error after 201 valid users |
| F11 / `F11_attachment` | `F11-{text-parts,blank-text,marks-boolean,marks-truthy-nonboolean,synthetic-detach,unknown-role,summary,orphan-metadata}` |
| F12 / `F12_metadata` | `F12-model-{last-pair,partial-pair,whitespace,long}`; `F12-title-{fallback,export,prefix}`; `F12-time-{created,completed,invalid-completed-fallback,seconds-ms}`; `F12-p4-{fraction,exponent,signed-string,offset,local}`; `F12-unused-numbers`, `F12-lone-surrogate` |
| F13 / `F13_tools` | `F13-tools-{several,error,todowrite,malformed,closed}` |
| F14 / `F14_plans` | `F14-{message-reverse,part-reverse,empty-clears,blank-clears,nonarray-fallback,error-skip,noncompleted-state,statuses,items-30,items-31,cut-200,split-astral}` |
| F15 / `F15_collector` | `F15-{turns-200,turns-201,bytes-2097152,bytes-over,prompt-clip,reply-clip,exact-full-then-text,empty-answer-time,ufffd-cut}` |
| F16 / `F16_utilities` | `F16-data-dir-direct`, `F16-folder-{empty,slashes,dots,case,backslash}`, `F16-rows-unfiltered-51`, `F16-rows-no-order`, `F16-plan-direct` |

Every Deviation needs its minimal trigger and exact expected effect. `oracle.py` must
not import Grok policy classifiers: their numbering/semantics differ. P1 => read-failed,
no partial data. P2/P3/P8 change only the specified decoded string/cut boundary. P4 reruns
TS with only unsupported export timestamp tokens changed to an invalid date string,
preserving completed-to-created fallback for plan and no fallback for reply. P5 requires
exact InputTooLarge in the 16–32 MiB gap; above 32 MiB is equal only after successful exit.
P9 uses independently computed Pruv numeric expectations on the original SQL-selected
rows; never rerun SQL on normalized updated values. Equal P6/P7/P10/P11/P12/P13 grant
no output/trace relaxation. Report explained deviations, unexplained failures and excluded
domain observations separately; missing measurements cannot pass.

`q.py` uses seed 20261117, 2,000 generated cases plus fixed cases, all four compiler/backend
pairs. Vary schema, raw SQL kinds/insertion order, options, malformed structure, marks,
model/time/plan fields and real child outcomes. Freeze list time only. Most stalled-child
cases use the same shorter rpcMs on both sides; keep the real 20 s fixed probe and real
1.5 s/1 s grace checks. Run timer-sensitive real cases serially to avoid harness overload.

Mutants run one at a time in `work/mutants`, rs/rust first. Add exact `MUT:Mxx` markers at
the intended decision, not regex guesses spanning unrelated code. Baseline must pass the
named case first. A verifier rejection or normal result/trace mismatch kills; compilation
failure, crash, harness error and watchdog kill do not.

| Mutant | Owner / edit | Required killing fixture |
|---|---|---|
| M01 memory-means-default-db | path: use default for `:memory:` | `F01-db-memory` (no db_open) |
| M02 follow-final-db-link | admit: accept final symlink | `F02-db-final-symlink` |
| M03 skip-required-column-check | sql: bypass required-name gate | `F03-missing-message-session_id`, `F03-missing-title` |
| M04 admit-child-archived-empty | sql: drop root/archive/exists predicate | `F04-exclude-child-archived-empty` |
| M05 folderkey-for-list-sql | sql: normalize stored directories for selection | `F04-list-slash-asymmetry` |
| M06 validate-before-limit | sql/list: select beyond 50 and backfill | `F04-limit-51-invalid-newest` |
| M07 resort-converted-updated | rows/list: sort after P9 conversion | `F04-raw-storage-order` |
| M08 non-strict-open-window | list: change `<` to `<=` | `F05-age-120000` |
| M09 export-before-admission | admit/export: allow export on folder mismatch | `F06-wrong-folder` (no spawn) |
| M10 pipe-export-stdout | export: request pipe instead of output fd | `F07-stdout-file-large` (fd-kind/trace failure); mutant-only flag must never occur in baseline |
| M11 size-before-exit-result | export: let oversized success override failed wait | `F09-nonzero-oversized` |
| M12 skip-malformed-message | convert: skip shape error | `F10-late-malformed` |
| M13 attach-after-empty-user | entry: retain open after synthetic-only user | `F11-synthetic-detach` |
| M14 metadata-only-when-open | convert/entry: suppress closed assistant metadata | `F11-orphan-metadata` |
| M15 empty-plan-falls-back | plan: continue after empty fit | `F14-empty-clears` |
| M16 omit-collector-trim-flag | convert: force omittedBefore false | `F15-turns-201`, `F15-bytes-over` |

## 8. Proof obligations and implementation packages

| Module | Obligations to discharge on both compilers |
|---|---|
| packet/root | Length products/additions bounded before indexing; every ref absent or in pool, UTF-8 refs at scalar boundaries; byte values 0–255; mode-specific widths; patch source/destination ranges; matching id/op consumed once; exactly one terminal result; no unchanged continue |
| util/path | Input-derived decreasing cursors; checked decimal/IEEE shifts; normalized path stack bounds; nonempty containment prefix with separator; no folderKey UTF-8 split |
| admit/sql | Phase belongs to owner; handles acquired before use; close-on-success/error, no close before open; schema short circuit; query width/count/ref validation; SQL fragments/param counts agree; admission failure cannot reach spawn |
| rows/list | Iterate exactly returned rows; count index <= validated count; retain raw SQL order; numeric bounds before multiply/time subtraction; required strings and optional refs valid; no session cap except SQL's 50 for list |
| export | Resource presence follows successful acquisition; deadline additions checked; monotonic observations nondecreasing; timeout flag monotone; TERM/KILL at most once and in order; settle once; close before stat; cleanup cannot replace saved error |
| entry/convert | Parser/tape validity, subtree progress and message/part loop bounds; validate all messages; full turns package invariants at every call; grow fits current slot; tool calls bounded by parts; metadata independent of attachment |
| plan | Reverse indices decrease to zero; stop on first candidate even if empty; fit scans all todos, nonblank count bounded by array length; stored <=30; 200 UTF-16-unit scalar-safe prefix; status enum and refs valid |

External liveness/clock honesty, SQLite semantics, OS facts, serialization and TS equivalence
are trusted/tested, not SMT proofs. Each individual step terminates; a child wait's eventual
completion relies on the host honoring its absolute deadline. Do not assert that a loop
counter proves wall-clock termination. Parsed ordinary input must not reach InternalInvariant.

Use the existing `compile_pruv` / `build_llvm_objects` harness flow with the OpenCode G4b
manifest and str adapter. Verified code emission is required for py/rust, py/llvm, rs/rust,
rs/llvm; never `--no-verify`. Also expose a true `test_impl.py --verify-only --pairs ...`
path that invokes compiler `--verify-only` for root and its reachable dependencies, skipping
host linking/execution. Both Python (`python3 -m pruvc`) and Rust (`pruvc-rs`'s built CLI)
must pass; record actual command lines in work logs. Do not copy Grok's misleading
verify-only shortcut that merely skips final linking. `--scope A|B|C|D` creates a temporary
verification entry/manifest in work for only that package's implemented modules; generated
test wrappers call the pure helpers and scripted steps, never supply unproved axioms.

Six work packages, disjoint authored file sets. Generated files use separate work subdirs.
All commands below are the CLI contract to implement in test_impl.py/q scripts, run from
repository root; `GROK_TS_TRANSPILER` must be set for commands using TS.

| Package | Owned files | Dependencies / handoff | Narrow local acceptance command |
|---|---|---|---|
| A ABI/common | `root.pruv`, `packet.pruv`, `util.pruv`, `pruv.json`, `IMPL_NOTES.md` | First: fix layouts and helper signatures here; root dispatch added by A after owners deliver | `python3 bench/l3/opencode/test_impl.py --verify-only --scope A --pairs py/rust,rs/rust`; packet shape/absent-empty probes via `--unit packet` |
| B paths/DB/list | `path.pruv`, `admit.pruv`, `sql.pruv`, `rows.pruv`, `list.pruv` | A + E primitive ABI; finishes read handoff with closed DB and fallback title | `python3 bench/l3/opencode/test_impl.py --pairs rs/rust --only F01 --only F02 --only F03 --only F04 --only F05 --only F06 --only F16-rows` plus `--verify-only --scope B --pairs py/rust,rs/rust` |
| C export state | `export.pruv` | A; consumes B's admitted read, produces raw bytes after cleanup or saved result | `python3 bench/l3/opencode/test_impl.py --unit export --pairs rs/rust` for scripted transitions; then `--only F07 --only F08 --only F09-nonzero-oversized --pairs rs/rust` and `--verify-only --scope C --pairs py/rust,rs/rust` |
| D conversion/plan | `entry.pruv`, `convert.pruv`, `plan.pruv` | A; can use pure byte probes before B/C link; returns section 5 packet | `python3 bench/l3/opencode/test_impl.py --unit convert --pairs rs/rust` for F09–F15 payloads and direct plan; `--verify-only --scope D --pairs py/rust,rs/rust` |
| E host/build/fixture mechanics | `host.rs`, `host/**`, `harness.rs`, `str_adapter.rs`, `common.py`, `test_impl.py`, `fake_exporter.py` | Starts after A wire agreement; scaffold selective commands early for B–D; no fixture expectations owned here | `python3 bench/l3/opencode/test_impl.py --unit host` (SQLite typed roundtrip/native-Node parity, wx/stdout fd, delta patches); `cargo check --locked --manifest-path bench/l3/opencode/host/Cargo.toml` |
| F reference/oracle/Q | `reference*.mjs`, `oracle.py`, `q.py`, `q_mutants.py`, `fixtures/**`, `README.md`, `Q_README.md` | A/E formats; can build TS fixtures while B–D write; full differential waits for integration | `python3 bench/l3/opencode/test_impl.py --reference-only --only F03 --only F07 --only F14`; `python3 bench/l3/opencode/q_mutants.py --check` after marker owners finish |

Order: A publishes ABI; E scaffolds build/unit commands; B, C, D and F proceed against
those interfaces. A links dispatch only after module signatures match. Each owner runs
its narrow commands, not repository suites. Integration then verifies root on both
compilers and builds/runs all four pairs on this piece's fixed fixtures (include `--slow`
for the actual timeout). The Q package alone runs `q.py --cases 2000 --seed 20261117` and
all 16 `q_mutants.py` mutations. Keep per-pair equality and TS policy reports; rerun only
the concrete failure cases after fixes. No shared-package refactor, CI expansion, real
OpenCode install, account access, UI work or deployment is part of these packages.
