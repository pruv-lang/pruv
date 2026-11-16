# OpenCode conversation import (L3-7)

Status: design, L3-7. Drafted by a design review (Codex) on 2026-11-17 and settled by the lead; open questions are
answered in the last section. No implementation exists yet.

## Scope

Port Lair's OpenCode importer to Pruv. The same Pruv program must verify and give identical results on all four
compiler/backend pairs (py/rust, py/llvm, rs/rust, rs/llvm).

| Reference surface | L3-7 scope |
|---|---|
| `opencodeDataDir` | Data-directory resolution, testable on its own |
| `folderKey` | Non-Windows comparison behavior |
| `readOpencodeRows` | Unfiltered discovery utility, including schema checks and row conversion |
| `listOpencode` | Folder-specific SQL selection, row validation, counts and activity hints |
| `readOpencode` | Database admission, official export, JSON conversion and result |
| `opencodePlan`, `fit` | Last recorded todo plan |
| `TurnCollector`, `isoOf`, `oneLine`, `validThread`, `plainFileIn` | Shared behavior, subject only to the numbered policies |

Account preparation, UI integration, publication, Windows behavior, external cancellation and detached process-group
management are outside this benchmark.

### Reference identity

The pin is `bench/l3/grok/reference/` (Lair commit `744d14af`, already used by L3-6).

| File | SHA-256 |
|---|---|
| `opencode.ts` | `985a4a9d5e79a97278e0ab7050b673977dde1230a968b9a448fe0f6d624804a0` |
| `tail.ts` | `750399578ca53bcf6ccf0b2b03863de08d0f16a3580c60f40d9ef8c216c3c14e` |
| `plan.ts` | `e432a9988c50b8f6500cb01db5c31816ad86cf30f64a9ee2c7d56a67afd4bda5` |

On 2026-11-17 the three pinned files were byte-equal (`cmp`) to the current Lair checkout
(`src/backend/engine-import/`, last change to them at Lair `1b239300`). L3-7 therefore ports current Lair.

## Decisions

### D1 — SQLite through a thin host adapter

A Rust host adapter opens SQLite read-only and runs parameterized statements. Pruv does not parse the SQLite file format,
and no importer-specific language builtin is added.

Reasons:

1. Pruv has no SQLite access.
2. SQLite owns pages, overflow pages, WAL visibility, locking, affinity, collation, ordering and query execution.
   Reimplementing them would make SQLite compatibility the main project.
3. A language builtin enlarges the trusted runtime and needs both compilers and both backends before the importer runs.
4. A narrow host adapter fits the existing benchmark arrangement (MCP command lookup, Grok host) and keeps the
   importer's decisions in Pruv.

The adapter does **not** expose a finished `list_opencode(folder)`.

| Responsibility | Owner |
|---|---|
| Resolve data directory and DB path | Pruv |
| Decide plain-file admission from filesystem facts | Pruv |
| Open/close SQLite and execute bound statements | Host primitive |
| Build schema queries and inspect returned column names | Pruv |
| Require `SESSION_REQUIRED` and `message.session_id` | Pruv |
| Decide the optional archived-column predicate | Pruv |
| Build root/message/folder/id predicates | Pruv |
| Choose `ORDER BY`, direction and `LIMIT 50` | Pruv |
| Execute SQL comparison, ordering and limit | SQLite |
| Validate returned IDs/directories and clean titles | Pruv |
| Request message counts and validate count results | Pruv |
| Compute timestamps and `maybeOpen` | Pruv |
| Map failures and build final results | Pruv |

Pruv owns the sorting rule and its placement before row validation; SQLite executes the order Pruv asks for. Sorting
converted rows in Pruv would change the source: SQL sorts raw values before `Number(updated)` and before invalid rows are
dropped.

### D2 — Adapter contract

Primitives equivalent to `db_open_readonly(path)`, `db_query(connection, sql, typed_parameters)`, `db_close(connection)`.

- Results keep column names, row order and SQL value kinds (NULL, INTEGER, REAL, TEXT, BLOB); values are not turned into
  strings.
- Rows may be batched but keep statement order. An error during an `all()`-equivalent invalidates the whole operation;
  partial rows are never published.
- Integer reads match the Node binding: an INTEGER outside the safe range is a read error (Node's default conversion
  throws), not a silently narrowed value.
- One connection serves the session query and the later counts, as in TS. No transaction, immutable mode, copied
  database, Rust-side folder prefilter or schema repair.
- **Build (lead decision):** the host crate adds `rusqlite` with the `bundled` feature, pinned to the release whose
  `libsqlite3-sys` bundles the SQLite closest to Node 26.4.0's (3.53.4 on this machine). The test run records both
  `sqlite_version()` values. Differences in query order or binding errors are failures unless separately explained.
  L3 pieces do not run in CI, so the extra crate is a local dependency only.

### D3 — Export stdout is a real file

Child stdout is redirected straight into an exclusively created file. A pipe copied to disk is not allowed: the
reference records that OpenCode 1.19.5 truncated piped export at 196,608 bytes while returning exit code zero.

The G10 process ops take program, arguments, cwd and environment but no stdout file, so a narrow host primitive spawns
with an inherited output descriptor; observation and signaling are host primitives too. Pruv owns argument building,
spawn timing, temporary-path order, timeout and termination states, exit/error classification, export-size decisions,
cleanup decisions and all export parsing. The host owns the OS operations, not a composite `run_opencode_export`.
`transport` is precedent for deadline handling; its JSONL receive path is not the export protocol.

### D4 — Environment and temporary directory

The host supplies the user home, parent environment and clock observations; Pruv reads `XDG_DATA_HOME` and
`OPENCODE_DB`. For export, an omitted `options.env` means the parent environment; a supplied environment is the complete
child environment. `tempRoot` must already exist and is not created. Temporary directories use the `opencode-` prefix;
the output file uses exclusive creation and mode `0600`. No account discovery, authentication reads or shell.

### D5 — Reuse shared packages without broad changes

Reuse json, jstr, access, text, time, names, bytes and turns through G4b manifests. OpenCode path resolution, SQL
building, row conversion, export state, message mapping and plan selection stay in `bench/l3/opencode/`. One
`turns::tool` call per matching tool part; no shared bulk-count API for this piece alone. No JSONL window or line limits
apply to an export document.

### D6 — Keep the parser limits

Reuse the shared parser's 16,777,216-byte source limit, depth 128 and tape 1,048,576 records. This is the explicit P5
deviation for exports above 16 MiB and up to 32 MiB. Raising the parser limit is a separate decision with effects on
other consumers.

### D7 — Verification boundary

Pruv proofs cover reachable contracts, state transitions, loop bounds, collector use and packet/reference validity.
Filesystem facts, SQLite execution, child-process facts, clock observations and host serialization are trusted. TS
equivalence is tested, not proved.

## Preserved behaviors

### Data directory and database path

`opencodeDataDir(env)`: ECMAScript-trim `XDG_DATA_HOME`; if nonempty and absolute, Node-style join with `opencode`;
else `<homedir>/.local/share/opencode`. List/read use `source.home` and do not recompute it.

Database path: trim `OPENCODE_DB`; exactly `:memory:` means no database; missing or empty means
`join(home, 'opencode.db')`; absolute is used directly; relative is joined against `home`. POSIX `path.join`
normalization applies; `../other/opencode.db` is not rejected for leaving `home`.

Database admission:

1. `realpath(dirname(path))`; failure means no database.
2. `lstat(path)` must be a regular file, not a symlink.
3. `realpath(path)`.
4. Separator-aware containment below the resolved **database parent** (not below `home`).

A similarly named sibling fails containment; a symlinked parent may pass; hard links are not rejected. There is no
`openRead` with the inspected inode, so no descriptor-identity protection is claimed.

Rejected or absent database: `[]` for list and unfiltered rows, `not-found` for read. An admitted database that fails to
open or query: `read-failed`. Close errors are ignored.

### Schema and SQL selection

Required `session` columns: `id, parent_id, directory, title, time_updated`; `message` must have `session_id`. Column
names come from `SELECT name FROM pragma_table_info(?)`; only string names count. A missing column is `read-failed`,
detail `columns`.

```sql
SELECT s.id AS id, s.directory AS directory, s.title AS title, s.time_updated AS updated
FROM session s
WHERE s.parent_id IS NULL
  /* if time_archived exists: AND s.time_archived IS NULL */
  AND EXISTS (SELECT 1 FROM message m WHERE m.session_id = s.id)
```

- ID selection adds `AND s.id = ?`.
- Folder selection adds `AND s.directory IN (?, ?)` and `ORDER BY s.time_updated DESC LIMIT ?` with 50.
- ID and unfiltered selection add no order or limit.
- Parameters are bound, never interpolated.

Non-Windows folder parameters: `base` = folder with trailing `/` removed unless that empties it; parameters are `base`
and `base + '/'`. Listing `/work` admits `/work` and `/work/` but not `/work//`; a direct read may accept `/work//`
through `folderKey`. SQLite's returned order is kept, ties included; no ID tiebreaker.

### Row conversion and listing

After SQL order and limit:

1. Skip unless `id` is a string matching `^[A-Za-z0-9][A-Za-z0-9_-]{0,127}$`.
2. Skip unless `directory` is a nonempty string.
3. `updated` via `Number(...)`; nonfinite becomes zero (P9 narrows the domain).
4. Clean string titles; other title values act as absent.

`cleanTitle`: empty for missing/empty or a raw string starting exactly with `New session - ` or `Child session - `
(tested **before** trimming); otherwise collapse ECMAScript whitespace, trim, slice to 120 UTF-16 units (P2).

Invalid rows use up places in the limit; no backfill from row 51.

For each validated row, on the same connection: `SELECT COUNT(*) AS n FROM message WHERE session_id = ?`.

After database work and close, sample `Date.now()` once. Output: `thread`; `title` (empty string included);
`updatedAt` = `isoOf(updatedMs)` only when `updatedMs > 0` and conversion succeeds; `messages` only when finite and
nonnegative; `maybeOpen: true` only when `updatedMs > 0 && now - updatedMs < 120000`. Never `maybeOpen: false`, path,
model or origin. Future timestamps count as open; exactly 120,000 ms old does not; the test uses raw `updatedMs`.

### Read admission

1. Invalid thread → `not-found`, before database work.
2. Select by ID with the same schema/root/archive/message rules.
3. Take the first validated row.
4. Require equal non-Windows `folderKey(row.directory)` and `folderKey(folder)`.
5. Missing row or unequal folder → `not-found`.
6. Missing/empty binary or `tempRoot` → `engine-missing`.
7. Only then export.

`folderKey` removes a trailing `/` unless that empties the string; no dot resolution, case folding or backslash
replacement. Export metadata does not recheck thread or folder.

### Export lifecycle

1. `mkdtemp(join(tempRoot, 'opencode-'))`.
2. Open `<dir>/export.json` with `wx`, mode `0600`.
3. Spawn the binary with exactly `--pure export <thread>`.
4. cwd = `source.home`, no shell, stdin/stderr ignored, the open file as stdout.
5. Wait for the child.
6. Close the output handle in `finally`.
7. Stat the output.
8. Size **greater than** 33,554,432 bytes: do not read it.
9. Otherwise read the whole file.
10. Recursive forced removal of the temporary directory; removal errors are ignored.

No trailing newline is required. The timeout is 20,000 ms, starting in `waitExport` after spawn; it does not cover
database work, temp creation or JSON conversion. On timeout: record failure, TERM, KILL 1,500 ms later if needed, give
up 1,000 ms after that. A close after a recorded timeout still returns `timeout`.

Synchronous spawn exceptions and child `error` events → `engine-missing` (the `error` handler finishes directly and does
not consult a stored timeout). Without a timeout, exit code zero succeeds; any other close is `read-failed`.

Other file errors stay raw OS errors: `mkdtemp` failure happens before the cleanup `try`; output close failure can
override a pending wait failure; stat/read failure is not automatically an `EngineImportError`; removal failure never
replaces the result.

Oversized export returns exactly `{"turns":[],"omittedBefore":true}` plus only the nonempty database fallback title.

### Export JSON and message mapping

Malformed JSON → `read-failed`, detail `json`. Nonobject root, nonarray `messages`, or any message without object `info`
and array `parts` → `read-failed`, detail `shape`. Every message is validated, including unknown roles and messages
before the kept suffix; a late malformed message fails the whole read. Messages are processed in array order.

`textOf(parts, skipMarked)`: object parts with `type === 'text'` and string `text` whose trim is nonempty; the original
text is kept; with `skipMarked`, reject only `synthetic === true` or `ignored === true`; join with `\n\n`.

`open` starts false.

- `role === 'user'`: text with `skipMarked=true`; `open` = text nonblank; if open, start a turn at
  `isoOf(info.time.created)`. A user without human text only closes attachment; no `skip_user`, next turn not clipped.
- `role === 'assistant'`:
  1. `summary === true` → skip entirely.
  2. Plan time = `isoOf(completed) ?? isoOf(created)`.
  3. If `providerID` and `modelID` are both nonempty strings, model = `providerID + '/' + modelID` (no trim, no length
     limit).
  4. Its parts count for plan selection even when no turn is open.
  5. If not open, stop here for the collector.
  6. Otherwise answer with `textOf(parts, false)` at **only** `isoOf(completed)`.
  7. Count every object part with `type === 'tool'`.
- Unknown roles change nothing.

Final title: `cleanTitle(string root.info.title) || databaseFallbackTitle`. Empty title/model and absent plan are omitted.

### Collector

The shared collector: prompt and reply clipped separately to 32,768 UTF-8 bytes; two LF between nonempty answer pieces;
completion time updated before rejecting empty or saturated text; a partial append saturates and sets `clipped`; a later
append to an exactly full reply does not newly set `clipped`; newest contiguous suffix within 200 turns and 2,097,152
prompt-plus-reply bytes, returned oldest first; `omittedBefore` means collector trimming only. Metadata and plan
processing continue after eviction.

### `opencodePlan`

Only nonsummary assistants count. Search messages newest to oldest, parts last to first. Candidate: object part,
`type === 'tool'`, `tool === 'todowrite'`, object `state` whose status is not exactly `error`, object `state.input` with
array `todos`.

For the first candidate: keep object todos with string `content`; status kept only for `pending`, `in_progress`,
`completed`; trim and drop blanks; slice to 200 UTF-16 units (P8); at most 30; `more` = remaining nonblank valid
entries, omitted when zero; time = the assistant's plan time. An empty or all-blank latest candidate returns no plan and
hides older ones; a nonarray `todos` is skipped and allows an older candidate.

## Policies

Each policy is **equal** or **Deviation**. Only the stated trigger and consequence are allowed.

- **P1 — Deviation: parser rejection domain.** Trigger: invalid UTF-8 in export bytes (minimal: byte `FF` inside a JSON
  string), nesting over 128, or tape over 1,048,576 records. Pruv returns `read-failed`; TS may replacement-decode or
  accept. No partial conversation; `InternalInvariant` is never a permitted result.
- **P2 — Deviation: surrogates and title slices.** Trigger: a JSON value with `"\ud800"`, or a 120-unit title cut that
  splits an astral scalar (119 ASCII + `😀`). Pruv uses U+FFFD for lone surrogates and drops the split scalar. Key
  matching keeps UTF-16 identity.
- **P3 — Deviation: genuine U+FFFD at a byte cut.** Trigger: 32,765 ASCII bytes, U+FFFD, another character, clipped at
  32,768. Pruv keeps the genuine U+FFFD; TS removes it. Saturation is unchanged.
- **P4 — Deviation: export timestamp domain.** `time` is reused unchanged: safe lexical integer JSON numbers, trimmed
  unsigned digit strings, exact Gregorian `YYYY-MM-DDTHH:mm:ss.sssZ`, years 0000–9999; 100,000,000,000 splits seconds
  from milliseconds. Triggers: `1.5`, `1e3`, `"1.5"`, signed numeric strings, offset/local dates → absent. Created and
  completed times may disappear; plan time may fall back from completed to created; collector time has no fallback.
- **P5 — Deviation: bounded core.** Trigger: export size 16,777,217..33,554,432 bytes. Pruv returns `InputTooLarge`; TS
  parses. 33,554,433 and above take the reference oversized branch and are equal. The host/core bank limit is `2^31`
  bytes/words and fails explicitly. No new session-count limit.
- **P6 — equal: SQL order, selection and limit.** Fixtures: 51 sessions with an invalid newest ID; `/work`, `/work/`,
  `/work//`; equal update values; mixed storage classes. No resort or invented tiebreaker.
- **P7 — equal: direct-child export lifecycle.** Fixtures: spawn failure, nonzero exit, hang, TERM exit, TERM ignored
  until KILL. 20 s timeout, 1.5 s/1 s termination, error precedence and cleanup order. Detached groups, descendants and
  AbortSignal are excluded; a watchdog kill is a failed observation, never a passing timeout.
- **P8 — Deviation: plan text slice.** Trigger: 199 ASCII units then an astral scalar at the 200-unit limit. Pruv drops
  the split scalar; TS keeps a lone high surrogate.
- **P9 — Deviation: database numeric coercion.** `updated` accepts safe signed INTEGER, finite integral REAL within
  `±(2^53−1)`, ECMAScript-trimmed TEXT `[+-]?[0-9]+` within that range, NULL and blank TEXT as zero; anything else is
  zero. Triggers: REAL `1.5`; TEXT `"1e3"`, `"1.5"`, `"0x10"`; finite REAL beyond the safe range; BLOBs that Node
  coerces to a nonzero number. Can change `updatedMs`, `updatedAt`, `maybeOpen`; never SQL order or the selected 50
  rows. Safe `COUNT(*)` is equal; an unsafe-INTEGER binding error stays `read-failed`.
- **P10 — equal: no JSON-number pass-through.** Trigger: unused export fields `1e400`, `1.0`, `9007199254740993` are
  ignored on both sides.
- **P11 — equal: attachment gate and metadata independence.** Trigger: valid user, synthetic-only user, assistant with
  text/model/todos, another valid user. The middle assistant adds nothing to the old turn but may set model and plan; the
  later user is not clipped. A summary assistant contributes nothing.
- **P12 — equal: latest-plan clearing.** Trigger: older nonempty `todowrite`, newer one with `todos: []` → no plan. A
  newer error-state call or nonarray `todos` is skipped instead.
- **P13 — equal: oversized export and post-export errors.** Trigger: exit 0 with output just over 32 MiB versus nonzero
  exit with the same file. The first returns the empty omitted record, the second `read-failed`. Raw close/stat/read
  failures and ignored removal failures are kept.

## Fixtures

| Group | Required cases |
|---|---|
| F1 paths | Missing/blank/absolute/relative XDG; default, absolute, relative, parent-relative and `:memory:` DB; trimming; all-slash paths |
| F2 DB admission | Missing DB, final symlink, symlinked parent, hard link, directory/FIFO, containment sibling, lstat/realpath errors |
| F3 SQLite/schema | Corrupt DB, open/query errors, each missing column, absent/present archive column, NULL vs zero archive, read-only WAL visibility |
| F4 selection | Child sessions, no-message sessions, invalid IDs, empty/nonstring directories, 49/50/51 rows, bad row using a limit place, slash asymmetry, ties/raw-type sorting |
| F5 metadata | Default-title prefixes before trim, 120-unit cuts, counts, P9 values, zero/negative/future times, ages 119999/120000/120001 |
| F6 read admission | Invalid thread, absent row, wrong folder, trailing slashes, missing binary/tempRoot, admission failure preventing spawn |
| F7 spawn/file | Exact argv/cwd/env; stdin/stderr ignored; stdout is a regular file; spaces in paths; output over 196608 bytes; no shell |
| F8 lifecycle | Spawn exception/error, exit zero/nonzero/signal, hang, TERM/KILL, output close failure, stat/read failure, cleanup failure, missing tempRoot |
| F9 size/parser | 16 MiB and 32 MiB boundaries, invalid UTF-8, depth/tape limits, malformed JSON, no trailing LF, nonobject root |
| F10 shape | Missing/nonarray messages; null/array message; bad info; bad parts; late malformed message after many valid turns |
| F11 text/attachment | Text/nontext parts, blank text, boolean marks, truthy nonboolean marks, synthetic-only user, unknown roles, summary assistants, leading orphan assistant |
| F12 model/title/time | Last nonempty model pair, whitespace IDs, export-title fallback, created/completed, P4 timestamps |
| F13 tools | Several tool parts, error tools, todowrite tools, malformed parts, tools while closed |
| F14 plans | Reverse order precedence, empty clearing, malformed fallback, error skip, noncompleted states, statuses, 30/31 items, 200-unit cut |
| F15 collector | 200/201 turns, 2 MiB boundary, prompt/reply clipping, exact saturation then text, empty answer updating time, U+FFFD cuts |
| F16 utilities | Direct data-dir/folder-key calls and unfiltered `readOpencodeRows` (no limit, no ordering) |

Fixtures use synthetic databases, fake executables and private temporary roots; never real conversations or
credentials.

## Q plan

**Harness.** `bench/l3/opencode/` holds the Pruv modules, a thin `host.rs` crate, fixtures, `test_impl.py`, `q.py`,
`oracle.py` and a scripted fake exporter, following the Grok harness pattern but not its Grok policies. The TS runner
executes the pinned exports on Node 26.4.0 with real `node:sqlite`, the pinned non-Windows `spawnEngine` stub for real
fake-child runs, and hooks only for module loading, controlled observations and fault injection; `sessionRows`,
`convert` and `opencodePlan` are never replaced. An OpenCode counterpart of `bench/l3/grok/reference.mjs` dispatches the
operations; the TypeScript transpiler location is passed in explicitly (as `GROK_TS_TRANSPILER`).

**Boundary.** Versioned flat packets with fixed metadata and `(start,len)` UTF-8 references; `(-1,0)` is absent. They
carry the mode, list rows or the ten-word turn rows, optional title/model, `omittedBefore`, plan presence/time/items/
`more`, unfiltered row fields including `updatedMs`, and primitive requests/completions (operation, handle, typed
values, error class). Absent and empty, false and omitted stay distinct. The host checks ranges and renders output; it
does not clean titles, filter rows, select plans or convert exports.

**Differential run.**

1. Build verified runners for all four pairs.
2. Run the fixtures against pinned TS and each pair.
3. Run 2,000 generated cases (seed `20261117`) plus fixed policy and boundary probes.
4. All four Pruv results and normalized traces agree.
5. Each pair matches TS under the numbered policies.

Generated cases vary schemas, SQLite value types, row orders, optional fields, malformed structures, text marks, model
pairs, time tokens, plans and child outcomes. Each isolated run gets the same database bytes and sidecars. The list
clock is frozen at the `Date.now()` observation; timeout progress is not. Most child-timeout probes use equal shorter
budgets; one fixed probe uses the real 20 s constant (any test-only override is recorded and restored).

**Oracle.** Compare results, order, optional fields, counts, times, plans; SQL statement sequence and typed parameters;
connection lifetime and count placement; child argv/cwd/env and regular-file stdout; temp placement, exclusive
creation, modes, cleanup; error stage/class/code (not prose). Normalize only fixture-root prefixes, opaque handles,
temp suffixes; assert the temp prefix and placement separately. Every Deviation needs a minimal trigger fixture and
either a narrow transformed-TS rerun or an independently stated expected result (P4 reruns change only the unsupported
timestamps; P9 expectations keep the SQL-selected rows; P5 requires exact `InputTooLarge`; P2/P3/P8 apply only at the
shown string boundary). Unexplained failures, policy differences and excluded-domain observations are reported
separately; a watchdog kill is never success.

## Mutants

Built one at a time on rs/rust. A named fixture must tell each from baseline, or verification must reject it. All 16
must be killed.

| Mutant | Killing fixture and observation |
|---|---|
| M01 `memory-means-default-db` | F1: `OPENCODE_DB=:memory:` opens no DB |
| M02 `follow-final-db-link` | F2: final symlink gives absence, not rows |
| M03 `skip-required-column-check` | F3: missing `message.session_id` or a session column fails |
| M04 `admit-child-archived-empty` | F3/F4: child, archived or message-free session not selected |
| M05 `folderkey-for-list-sql` | F4: `/work//` not listed for `/work` |
| M06 `validate-before-limit` | F4: an invalid row in the first 50 is not replaced by row 51 |
| M07 `resort-converted-updated` | F4: mixed raw types keep SQL order |
| M08 `non-strict-open-window` | F5: exactly 120000 ms old is not open |
| M09 `export-before-admission` | F6: wrong folder spawns no child |
| M10 `pipe-export-stdout` | F7: the fake needs a regular-file stdout and writes a large export |
| M11 `size-before-exit-result` | F8/F9: nonzero exit with oversized output stays a failure |
| M12 `skip-malformed-message` | F10: a late malformed message fails the whole record |
| M13 `attach-after-empty-user` | F11: assistant after a synthetic-only user does not attach to the old turn |
| M14 `metadata-only-when-open` | F11/F12: orphan nonsummary assistant still updates model/plan |
| M15 `empty-plan-falls-back` | F14: latest empty todos clears an older plan |
| M16 `omit-collector-trim-flag` | F15: 201 turns or excess bytes set `omittedBefore` |

## Limits

- SQLite and OS adapters are trusted; moving query building and decisions into Pruv does not prove SQLite.
- SQLite tie order is not an application guarantee; engine behavior is pinned and recorded, not hidden by sorting.
- Database validation and opening are not atomic; there is no checked-descriptor identity as in Grok.
- Separate statements are not a snapshot; live mutation between selection and counts is observable.
- Export size is checked after completion; the 32 MiB threshold does not bound disk use while the child runs. Stat and
  read are separate; growth after the stat is not another oversized branch.
- P5 leaves a 16–32 MiB parser-domain gap.
- Windows folder normalization, SQL `NOCASE` and Windows child launch are outside the platform domain.
- External AbortSignal and detached process-group/descendant management are not ported.
- No JSONL tail window, line cap, RPC handshake or response-ID policy applies.
- Shared-package changes need affected-consumer checks (AGENTS rule for `bench/l3/lib`/pkgs).

## Open questions (lead answers)

1. Baseline: the pin equals current Lair (checked 2026-11-17, see Reference identity).
2. P5's 16–32 MiB deviation is accepted for L3-7; raising the parser limit is a separate bundle if ever needed.
3. SQLite build: `rusqlite` bundled, version closest to Node's SQLite 3.53.4, both versions recorded (D2).
