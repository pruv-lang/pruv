# L3-8 engine-import common entry: implementation design

This implements `docs/ENGINE_IMPORT_ENTRY_SPEC.md`, and the specification wins on any conflict. The reference is
`reference/backend-{engine-import,engine-import-bound,codex-home,accounts,engine-import-tail}.ts`, plus
`../grok/reference/opencode.ts` (`opencodeDataDir`) and `reference/backend-engine-import-qwen.ts` (`qwenRuntimeDir`),
pinned at Lair `744d14af`. Do not edit the pinned files. This document specifies future implementation, not
completed verification.

**Spec §4 amendment (concrete layout).** Spec §4.1 describes the packet as nested `[tag, count, ...]` values. This
design fixes a concrete flat layout with the *same value domain*. It has the following parts:

- a section header;
- one UTF-16 pool;
- fixed 3-word value slots;
- fixed-width tables.

Every distinction required by §4.1 stays representable. These distinctions are:

- undefined, null, false and true;
- protocol integers and IEEE bits;
- strings as UTF-16 units;
- arrays and object entries;
- opaque references;
- absent own properties.

Nesting is structurally bounded (session → forks/slots is depth 2), so the 32-level depth limit cannot be reached. The
limit is recorded and needs no runtime check.

Ownership is split as follows:

- **Pruv** owns every decision of E:34–93 and B:15–44:
  - trim;
  - the environment fallback;
  - account-id validation;
  - POSIX path algebra;
  - canonicalPath, sameFile and isHostCodexHome;
  - source rows;
  - find;
  - route choice;
  - the title cut;
  - rethrow classification;
  - thread extraction;
  - temporary exclusion;
  - bound keys and first-write.
- **Rust** owns these mechanical parts:
  - scripted observations;
  - opaque reference tables;
  - packet transport;
  - list/error reconstruction;
  - JSON rendering.
- **The host never** trims, canonicalizes, skips accounts, clips titles, or builds keys.

## 1. Files

Paths are relative to `bench/l3/entry/`. Line counts are estimates.

| File | Responsibility | Lines |
|---|---|---:|
| `pruv.json` | Root `l3_entry_root`, version `0.1.0`, root `root.pruv`, **no dependencies** (self-contained; no `pkgs`/`lib` change) | 15 |
| `root.pruv` | `pub fn entry_begin(input: [i64]) -> [i64] ! alloc`, `pub fn entry_step(prev: [i64], completion: [i64]) -> [i64] ! alloc`; validate, dispatch by op, build await/done | 200 |
| `packet.pruv` | Packet/section validation, value-slot (VS) readers with allowed-tag sets, output builder (pool append, sections, VS writers), boundary results | 450 |
| `value.pruv` | Truthiness, strict equality, JS-string stringification of the bound-key domain, safe-integer decimal | 200 |
| `u16.pruv` | Unit-buffer helpers: ECMAScript trim, equality, prefix-before-first-colon, slice, concat, ASCII literal compare | 200 |
| `path.pruv` | Node 26.4.0 posix `normalizeString`, `normalize`, `join`, `resolve` (split into "which cwd calls" + "finish"), `dirname`, `basename`, `isAbsolute` | 300 |
| `accounts.pruv` | `valid_account_id`, `account_home` (wire invalidId) | 80 |
| `canon.pruv` | `canonicalPath` / `sameFile` / `isHostCodexHome` / `codexAccountFolder` as an explicit resumable machine (§4) | 450 |
| `sources.pruv` | `import_sources` phase machine and emitted source rows (§4) | 350 |
| `dispatch.pruv` | `find_source`, route choice, list recipe (title cut), read pass-through, `rethrow` | 220 |
| `bound.pruv` | `session_threads`, `excluded_from_bound`, `bound_session_index` (batched predicate, §3.4) | 350 |
| `state.pruv` | Continuation encode/decode/validate (section 2 of an await packet) | 250 |
| `host.rs` | Case JSON → packets, scripted observation server, ref tables, list/error reconstruction, JSON-object key order, trace, probes, result JSON | 1,300 |
| `host/Cargo.toml`, `host/Cargo.lock`, `host/src/lib.rs` | Thin `entry_host` library including `host.rs`; deps `serde_json` only (registry cache, `--offline --locked`) | 30 + lock |
| `harness.rs`, `str_adapter.rs` | Link generated `sol` to `entry_host::run` (the OpenCode pattern); `str_adapter.rs` only if the generated ABI needs it | 60 |
| `common.py` | Case loading, defaults merge, JSON value codec (§5), result normalization, comparison and classification | 350 |
| `test_impl.py` | Verified four-pair build (py/rust, py/llvm, rs/rust, rs/llvm), fixture run, oracle comparison, `--only GROUP`, `--case ID` | 350 |
| `oracle.py` | Runs `reference.mjs` over case JSON lines and returns normalized oracle results | 120 |
| `reference.mjs`, `reference-hooks.mjs`, `reference-primitives.mjs` | Pinned-function oracle (§6) | 450 |
| `fakes/engine-{claude,codex,grok,opencode,qwen}.ts`, `fakes/wire.ts`, `fakes/stubs.ts` | Oracle-only fake engine modules and inert wire support (§6) | 200 |
| `fixtures/_defaults.json`, `fixtures/{S,E,A,C,H,F,D,T,R,U,V,X,B,P,I}.json` | 155 fixtures (§7 plus S11, C11, C12, H11, V11) | 15 files |
| `integrate.py` | I01–I10 forwarding probes over existing piece drivers (§8) | 250 |
| `q.py`, `q_mutants.py` | 2,000-case Q (seed 20261118) and M01–M16 (§9) | 450 + 250 |
| `README.md`, `Q_README.md`, `IMPL_NOTES.md` | Commands, actual results, Node path-source dump, ABI notes; no claimed unrun results | — |
| `work/**` | Generated builds, logs and reports; disposable, not committed | generated |

Engines, compilers, `pkgs`, `bench/l3/lib`, other pieces and the pinned TS are not edited.

## 2. Packet

### 2.1 Envelope

```text
packet  = [1, kind, op_id, seq, len, payload...]        total words = 5 + len, must equal the array length
payload = [op, nsec, len_0, ..., len_{nsec-1}, data_0, ..., data_{nsec-1}]
```

- **kind:** 0 input, 1 await, 2 completion, 3 done.
- **op_id:** ≥ 1, chosen by the host per invocation.
- **seq:**
  - input: 0;
  - await: 1, 2, …;
  - completion: echoes the await;
  - done: the last await seq, or 0.
- **op:**
  - 1 import_sources
  - 2 find_source
  - 3 list_external
  - 4 read_external
  - 5 account_home
  - 6 codex_account_folder
  - 7 session_threads
  - 8 excluded_from_bound
  - 9 bound_session_index
- **Sections** are contiguous in order:
  - each `len_i` ≥ 0;
  - `2 + nsec + Σ len_i = len`;
  - `nsec` ≥ 2.
- **Section 0** is the UTF-16 pool: every word must be in 0..65535. Unreferenced pool units are allowed; P10 pads with them.
- **Section 1** is the record: VS slots and protocol words, laid out per kind/op/request (§3).
- **Sections ≥ 2:**
  - In an input or completion packet, they are tables.
  - In an await packet:
    - section 2 is the state;
    - section 3 holds the emitted source rows;
    - sections ≥ 4 are request tables.
  - In a done packet:
    - section 2 is empty;
    - section 3 holds the final emitted rows;
    - sections ≥ 4 are result tables.
  - Every table section is referenced by exactly one VS (tag 7/8). An unreferenced table, or a doubly referenced one, is InvalidPacket.
- **Size:**
  - Before any indexing, `total ≤ 16,777,216`, else InputTooLarge.
  - Header arithmetic is overflow-safe: compare each `len_i` with the remaining words before adding.
- **Output size:** an output that would exceed 16,777,216 words is a done packet with InputTooLarge, never truncated.

### 2.2 Value slot (VS), 3 words `[tag, a, b]`

| tag | meaning | a | b |
|---:|---|---|---|
| 0 | undefined | 0 | 0 |
| 1 | null | 0 | 0 |
| 2 / 3 | false / true | 0 | 0 |
| 4 | protocol integer (never a JS value) | value | 0 |
| 5 | JS Number | low 32 bits | high 32 bits (each 0..2^32−1) |
| 6 | string | pool offset | unit count (range-checked against pool) |
| 7 | array | section index ≥ 2 | row count (section len = count × row width) |
| 8 | object entries (enumeration order) | section index ≥ 2 | entry count |
| 9 | opaque ref | id ≥ 1 | 0 |
| 10 | absent own property (read yields undefined) | 0 | 0 |
| 11 | inspected object (its read fields follow in fixed slots) | 0 | 0 |

The following are all InvalidPacket:

- unused words that are nonzero;
- a tag outside the slot's allowed set;
- a wrong row width.

**JS value set ("any"):** tags {0,1,2,3,5,6,9}, plus 10 where a property read is modeled.

Every slot below states its set. "Read as undefined" means 10 behaves like 0.

### 2.3 Discipline

The core rejects all of the following as InvalidPacket:

- wrong version;
- wrong kind for the position (begin needs input; step needs await + completion);
- a length mismatch;
- `completion.op_id ≠ await.op_id`;
- `completion.seq ≠ await.seq`;
- `completion.op ≠ await.op`;
- a completion record not matching the outstanding request;
- a malformed state (§4.4).

`entry_step` on a done packet is InvalidPacket (P09, terminal re-entry).

The pure core cannot see replay. The host driver (`host.rs`) enforces these rules itself:

- each await is stepped at most once;
- only the newest await may be stepped.

A G8/P08 *duplicate* case then does two things:

1. The driver refuses the replay and records `driver: InvalidPacket`.
2. It calls the core with `(await_{k+1}, completion_k)`, which the core must reject by seq. Both are reported.

Request ids increase by one per await. The driver also guards seq exhaustion: it rejects seq > 2^53, which cannot occur within the size bound.

A boundary result is a done packet with `[5, code]` in the record. The codes are 0 InvalidPacket, 1 InputTooLarge and
2 InternalInvariant. The op is the input op if known, else 0, and the pool is empty. InternalInvariant marks a core bug and is never a pass.

## 3. Records

### 3.1 Input records (section 1 of kind 0)

| op | record (VS unless marked) |
|---|---|
| 1 | `dataDir` {6}; env slots {0,1,6,10}: `CLAUDE_CONFIG_DIR, GROK_HOME, XDG_DATA_HOME, QWEN_RUNTIME_DIR, QWEN_HOME, CODEX_HOME, HOME` (the last two are present but must never be read by the reference logic; M02 reads deps CODEX_HOME) |
| 2 | `query` any; `sources` {7}, row = `[source]` (1 VS, any) |
| 3 | `engine` any; `sourceRef` {9}; `folder` any; `optionsRef` {9} |
| 4 | as 3 with `thread` any between folder and optionsRef |
| 5 | `dataDir` {6}; `engine` {6}; `id` any |
| 6 | `dataDir` {6}; `id` any; `home` any |
| 7, 8 | one session record (below) |
| 9 | `sessions` {7}, row = one session record |

`?.trim()` on env values: tags 0, 1 and 10 give undefined; tag 6 is trimmed.

The following are outside the domain and rejected as InvalidPacket, recorded as excluded domain:

- non-string dataDir;
- non-string engine for op 5;
- non-string env values.

Each case is listed in `IMPL_NOTES.md`.

**Session record (10 VS = 30 words):**

| slot | field | allowed |
|---|---|---|
| 0 | `id` | any |
| 1 | `temporary` | any |
| 2 | `engine` | any |
| 3 | `externalThread` | {0,1,10,11} |
| 4 | `externalThread.engine` | any if slot 3 is 11, else exactly 0 |
| 5 | `externalThread.thread` | any if 11, else 0 |
| 6 | `externalThread.forks` | {0,1,10,7} if 11, else 0; row = `[fork]` (1 VS, any) |
| 7 | `terminal` | {0,1,10,11} |
| 8 | `terminal.cliSession` | any if slot 7 is 11, else 0 |
| 9 | `engineThreads` | {0,1,10,8}; entry row = `[key {6}, bound {0,1,2,3,5,6,10,11}, threadId (any if bound is 11, else 0)]` (9 words) |

The entry order of slot 9 is JS `Object.entries` order. The host produces it (§5.3); the core never reorders.

### 3.2 Requests (section 1 of kind 1, first word = request code)

| code | request | record after code | completion record `[status, ...]` (status 0 ok / 1 throw) |
|---:|---|---|---|
| 1 | `accounts` | engine word (0 claude / 1 codex) | ok `[0, count]` (protocol int ≥ 0); throw `[1, E]` |
| 2 | `account_at` | engine word, index word | ok `[0, rowKind (0 nullish / 1 other), id any, name any]`; never throws |
| 3 | `ambient_env` | — (always `CODEX_HOME`) | ok `[0, value {0,6}]`; never throws |
| 4 | `homedir` | — | ok `[0, {6}]`; throw `[1, E]` |
| 5 | `cwd` | — | ok `[0, {6}]`; throw `[1, E]` |
| 6 | `realpath` | path {6} | ok `[0, {6}]`; throw `[1, E]` |
| 7 | `stat` | path {6} | ok `[0, devText {6}, inoText {6}]`; throw `[1, E]` |
| 8 | `is_temporary` | ids {7}, row `[id any]` | `[0, results {7} row `[value any]`, thrown (0/1), E]`. With thrown = 0, E is six tag-0 slots. With thrown = 1, `results.count < ids.count` and E is the throw of call `results.count` |
| 9 | `invoke_engine` | route (0 claude, 1 codex, 2 opencode, 3 qwen, 4 grok), opKind (0 list / 1 read), sourceRef {9}, folder any, thread (10 for list, else any), optionsRef {9} | list ok `[0, rows {7} row `[rowRef {9}, title {6}]`]`; read ok `[0, recordRef {9}]`; throw `[1, E]` |

**Error observation E (6 VS):**

1. `valueRef` {9}: the host assigns one to every thrown value, primitives included.
2. `isEngineImportError` {2,3}.
3. `isError` {2,3}.
4. `name` any: the value of `thrown?.name`.
5. `message`: {6} if isError, else 0, since message is never read for non-Errors.
6. `code` any: the value of `thrown?.code`.

Errors thrown by homedir, cwd, realpath and stat are Error objects (isError 3). Their messages are native prose (P5).

Notes on the requests:

- **account_at** is a transport read of the array that the host snapshotted at `accounts`. Property reads of a plain row are unobservable. It is excluded from the P2 trace by P4. The design keeps await state O(1) per account instead of shipping the whole array.
- **is_temporary** is batched only for `bound_session_index`. Between two predicate calls in B:37–43 no other observation occurs, so a batch is observationally equal. The host must call the predicate in order and stop at the first throw. `excluded_from_bound` sends a batch of one.

### 3.3 Done record (section 1 of kind 3)

| first word | terminal | rest |
|---:|---|---|
| 0 | ok | op-specific (below) |
| 1 | throw_existing | valueRef {9} |
| 2 | throw_import | code {6} (`read-failed`), detail {0,6}, causeRef {9} |
| 3 | throw_account_wire | 0 `wireEngine.account.invalidId` / 1 `wireEngine.account.folder.codexHostExecution` |
| 4 | throw_plain_error | 0 (the Codex folder mismatch text) |
| 5 | boundary_error | 0 InvalidPacket / 1 InputTooLarge / 2 InternalInvariant |

The host maps the enums of terminals 3 and 4 to the exact pinned key and text:

- invalidId → `계정 번호 모양이 틀려.`
- codexHostExecution → `기본 Codex 계정 폴더에서는 추가 계정으로 실행하지 않아.`
- plain (terminal 4) → `Codex 계정 폴더를 확인하지 못해 쓰지 않았어.`

For terminal 2, the host computes the message as `detail ? "read-failed: " + detail : "read-failed"` (T:42) and keeps the cause.

**Ok payloads:**

| op | ok record after `0` |
|---|---|
| 1 | nothing; the result is the concatenation of section 3 rows over all awaits and the done packet |
| 2 | index {4} or undefined {0} |
| 3 | recipe {7}, row `[action (0 keep / 1 copy_title), rowRef {9}, title ({0} keep, {6} copy)]` |
| 4 | recordRef {9} |
| 5, 6 | path {6} |
| 7 | threads {7}, row `[engine any, thread any]` |
| 8 | value any (the raw `temporary === true \|\| isTemporary(id)` value) |
| 9 | entries {7}, row `[key {6}, sessionId any]` |

**Emitted source row (section 3, 6 VS = 18 words):** `source {6}, engine {6}, kind {6: home/account}, account`, then
`accountName` and `home {6}`:

- `account` is 10 for home rows, else the id {6}.
- `accountName` is 10 for home rows, else the name (any).

The host keeps rows only until the terminal. On an ok terminal it returns them all in order. On any other terminal it discards them (A09: no partial output).

### 3.4 Semantics the core must reproduce

- **Trim:** remove leading/trailing units from the exact ECMAScript set:
  - 0009, 000A, 000B, 000C, 000D, 0020, 00A0;
  - 1680, 2000–200A, 2028, 2029, 202F, 205F, 3000;
  - FEFF.

  0085 and 200B are not trimmed. `x?.trim() || fallback` falls back on undefined or an empty result.
- **Account id:**
  - A string matching `^acc_[a-f0-9]{16}$` exactly: 20 units, lowercase hex.
  - No trailing line terminator is accepted. JS `$` without the `m` flag matches only at the end.
  - Non-strings are invalid.
  - accountHome = `join(dataDir, "engine-homes", engine, id)`.
- **Strict equality** (`===`):
  - Tags 0 and 10 are equal.
  - Strings compare by units, and refs by id.
  - Numbers: NaN ≠ NaN, and +0 = −0. Otherwise compare bits.
  - Any other tag pairing is unequal.
- **Truthiness:**
  - Falsy: 0, 1, 2 and 10; a Number ±0 or NaN; the empty string.
  - Truthy: 3; a nonzero Number; a nonempty string; 9; 11.
- **Bound-key stringification** (template literal, `bound_session_index` only):
  - undefined → `undefined`; null → `null`;
  - booleans → `true`/`false`;
  - strings verbatim;
  - Numbers that are safe integers (|x| ≤ 2^53−1) → decimal, with −0 → `0`.

  Any other Number, or any ref, is InvalidPacket (excluded domain, recorded). `session_threads` emits values verbatim and never stringifies.
- **Threads (B:15–27):**
  1. If origin is truthy (11): yield `(origin.engine, origin.thread)`, then `(origin.engine, fork)` for each fork row. Nullish forks are empty.
  2. If terminal is 11 and `cliSession` is truthy: yield `(engine ?? "codex", cli)`. Here `??` replaces only 0, 1 and 10, so the empty string is kept (U09).
  3. For each entry: if bound is 11 and threadId is truthy, yield (the key's units before the first `:`, or the whole key, threadId).
- **excluded_from_bound:** if `temporary` is tag 3, return true with no request. Otherwise request `is_temporary([id])`:
  - a throw → throw_existing;
  - otherwise return the raw value.
- **bound_session_index:**
  1. Batch-request ids for every session whose `temporary` is not tag 3, in order.
  2. If the batch threw at index k, the terminal is throw_existing. No result is produced, but the trace holds calls 0..k.
  3. Otherwise iterate sessions in order. Skip a session that is literal-true or has a truthy result.
  4. For each thread of a kept session, build `key = str(engine) + ":" + str(thread)` and insert it only if absent (first write wins).
  5. Output in insertion order.

  The duplicate check may be a linear scan over inserted keys. A sort-free hash index is not required, but performance counts:
  - n entries with O(n²) key compares over 32 sessions × small rows is acceptable;
  - Q sizes must finish within the timeout.

  `IMPL_NOTES.md` states the bound.
- **find_source:** first row index whose `source` strictly equals `query`; undefined if none.
- **Route:** engine strictly equal to the string `claude`, `codex`, `opencode` or `qwen`, else grok (4). The check order is claude, codex, opencode, qwen.
- **List recipe:** for each row in order:
  - `title.length > 200` → copy_title with units 0..199 (a code-unit cut, so a high surrogate may be kept, T07);
  - otherwise keep.

  There is no row cap.
- **rethrow(E):**
  - If isEngineImportError, or `name` is the string `AbortError` → throw_existing(valueRef).
  - Otherwise throw_import("read-failed", isError ? message : undefined, valueRef).

  The host builds the message by truthiness of detail (an empty message gives `read-failed`).

## 4. Path machine (C:41–97, E:34–54)

### 4.1 Node 26.4.0 posix path

Port these from `node -e "console.log(require('path').posix.resolve.toString())"`, and likewise for `normalize`, `join`,
`dirname`, `basename` and internal `normalizeString`. Paste the dumped sources into `IMPL_NOTES.md`. `normalizeString` is
in `lib/path.js`; take it from the Node 26.4.0 tag source text cited in IMPL_NOTES, or reconstruct it and probe it.

These rules matter:

- **`resolve(p)` cwd calls:**
  - If `p` is `''` or `'.'`: call cwd once. If the result starts with `/`, return it verbatim (not normalized).
  - Otherwise, and in the general case: call cwd once **more** when p is not absolute, then `cwd + '/' + p + '/'`.
  - So `resolve('.')` with a relative cwd calls cwd **twice**.
  - Normalize with `allowAboveRoot = !absolute`. Return `/x` if absolute, else `x`, or `.` when empty.
- **`join(args)`:** drop empty args, join with `/`, then `normalize`. With no non-empty args the result is `.`.
- **`normalize`:**
  - Empty input gives `.`.
  - Keep a trailing `/`.
  - An empty result gives `/` if absolute, `./` if it had a trailing separator, else `.`.
- **`dirname`:** includes the `//` case.
- **`basename`:** no suffix argument.

The resolve function is split in two:

- **`resolve_cwd_calls(p)`** returns which cwd requests are needed: 0, 1, or the `.` fast-path sequence.
- **`resolve_finish(p, cwd_results)`** finishes the resolve.

A cwd throw escapes the current helper (§4.3).

### 4.2 canonicalPath(path) as resumable sub-machine

State fields:

- `orig`: the input path;
- `current`;
- `missing`: an ordered list of segments;
- `step`: one of resolving, real, stat or root-resolve.

The algorithm:

1. Set `current = resolve(orig)`, issuing cwd requests as in §4.1.
2. Request `realpath(current)`.
3. On ok, return `real` if `missing` is empty, else `join(real, ...missing)`.
4. On throw with `code` strictly unequal to the string `ENOENT`, request `stat(current)`:
   - On ok, the result is `dev:` + devText + `:ino:` + inoText, followed by `/` + missing joined by `/` if `missing` is nonempty.
   - On throw, climb.
5. On ENOENT, climb with no stat (C04/M06).
6. To climb:
   - Set `parent = dirname(current)`.
   - If `parent` equals `current` as units, return `resolve(orig)`. This is a **second** resolve, which may request cwd again.
   - Otherwise, unshift `basename(current)` onto `missing`, set `current = parent`, and go to step 2.

The loop measure is the length of `current`: dirname strictly shortens, or the loop stops. Each request is one state
transition, so no in-step loop runs over requests.

### 4.3 isHostCodexHome(home) and codexAccountFolder

Order of observations. Nothing is cached, and each call is a fresh request.

1. `target = canonical(home)`.
2. `h = hostCodexHome()`: `ambient_env`. If trim is empty or the value is undefined, `join(homedir(), ".codex")`. Then `host = canonical(h)`.
3. `homedir()`, then `dot = canonical(join(hd, ".codex"))`.
4. If `target == host` or `target == dot` (units), return true.
5. Evaluate the arguments: `h2 = hostCodexHome()` (fresh ambient, and maybe homedir). Then `sameFile(home, h2)`:
   - `stat(home)`; if ok, `stat(h2)`.
   - Equal exactly when devText and inoText are both equal.
   - Any throw gives false.
6. If false: `homedir()`, then `sameFile(home, join(hd, ".codex"))` the same way.
7. Return the result.

Note that `home` is passed to stat unresolved, as in the pinned code.

**codexAccountFolder(dataDir, {id, home}):**

1. `expected = account_home(dataDir, "codex", id)`, which may give the invalidId wire error.
2. If `home !== expected` (strict), throw the plain error (terminal 4).
3. If `isHostCodexHome(expected)`, throw the codexHostExecution wire error (terminal 3).
4. Otherwise return expected.

Any homedir/cwd throw inside escapes as throw_existing.

### 4.4 import_sources phases

Phase list, with the exact reference order:

| phase | action |
|---|---|
| CLAUDE_HOME | trim `CLAUDE_CONFIG_DIR`; if empty → request homedir → `join(hd, ".claude")`; emit `claude:home` |
| CLAUDE_ACCTS | request `accounts(claude)` (throw → throw_existing, rows discarded); then `account_at` 0..n−1: nullish or invalid id → skip; else emit `claude:<id>` row with `home = account_home(dataDir,"claude",id)` |
| CODEX_HOME | `hostCodexHome()` (ambient, maybe homedir; a homedir throw escapes); emit `codex:home` |
| CODEX_ACCTS | request `accounts(codex)` (throw escapes); per row: id invalid → skip; else run `codexAccountFolder(dataDir, {id, home: account_home(...)})`. Here `home === expected` always, so the plain-error branch is unreachable. **Any** throw inside that body (wire errors, homedir/cwd throws) skips the account and continues with the next row |
| GROK_HOME | trim `GROK_HOME`, else homedir → `join(hd, ".grok")` |
| OPENCODE_HOME | `xdg = trim(XDG_DATA_HOME)`; if nonempty and starts with `/` → `join(xdg,"opencode")`; else homedir → `join(hd,".local","share","opencode")` |
| QWEN_HOME | request homedir **first** (eager argument, E10); then `trim(QWEN_RUNTIME_DIR) \|\| trim(QWEN_HOME) \|\| join(hd,".qwen")` |
| DONE | ok |

A homedir throw outside a Codex account body escapes as throw_existing.

**State (section 2 of an await) is core-private.** Requirements:

- It is a validated word section of the form `[magic 0x454e5452, phase, sub-fields..., string descriptors (offset,len) into the await pool]`.
- Each await pool contains only the carried strings:
  - dataDir;
  - the four env strings still needed;
  - the canonical sub-machine strings (orig, current, missing segments, target, host, h2, hd).
  - The request strings follow.
- The account index and count are carried as words.
- Every field is range-checked on decode. A malformed or impossible state is InvalidPacket, never InternalInvariant.
- Per-step cost is O(carried strings + request).
- Emitted rows are not carried, which avoids O(A²) work.

## 5. Host (`host.rs`) and case format

### 5.1 Case JSON (one line per case; same file feeds oracle and host)

```json
{"id":"S08","op":"import_sources",
 "args":{"dataDir":"/data","env":{"CODEX_HOME":"/deps"}},
 "script":{"ambient":[{"ok":"/amb"}],"homedir":[{"ok":"/home/test"}]},
 "expect":{"...":"optional fixture assertions"}}
```

**JSON value codec (JV)**, shared by Python, Rust and the oracle:

| JV | JS value |
|---|---|
| JSON string | that string; **must be well-formed UTF-16**, since serde_json rejects lone surrogate escapes |
| `{"u16":[...]}` | any unit string (required whenever a lone surrogate is present) |
| `null` | null |
| `{"undef":1}` | undefined |
| `true` / `false` | booleans |
| `{"num":"<16 hex>"}` | IEEE bits |
| JSON integer | that Number, when it is a safe integer |
| `{"ref":"name"}` | an opaque sentinel object, the same identity per name within a case |
| `{"arr":[JV...]}` | an array |
| `{"obj":[["k",JV],...]}` | a plain object whose keys are own data properties defined in listed order (so JS reorders integer keys). The oracle defines them with `Object.defineProperty`, like `JSON.parse`, so a `"__proto__"` key is an ordinary own key and never calls the prototype setter |
| `{"absent":1}` | in an object entry list: the key is not set (used for session/record fields) |

**Per-op args:**

| op | args |
|---|---|
| 1 | `{dataDir, env:{NAME:JV}}`; a missing name means absent |
| 2 | `{sources:[{"source":JV}|{"absent":1}...], query:JV}`; each entry is a distinct object |
| 3 / 4 | `{source:{"ref":..,"engine":JV|absent, ...extra}, folder:JV, thread?:JV, options:{"ref":..}}` |
| 5 | `{dataDir, engine, id:JV}` |
| 6 | `{dataDir, account:{id:JV, home:JV|absent}}` |
| 7 / 8 | `{session}` |
| 9 | `{sessions:[...]}` |

A session object has these fields:

- `id`, `temporary`, `engine`: JV, or absent when omitted;
- `externalThread`: absent, null, undefined, or `{engine, thread, forks?: {"arr":[...]}|null|undefined}`;
- `terminal`: same shape with `cliSession` and an ignored `started`;
- `engineThreads`: `{"obj":[[slot, bound]]}` where bound is `{"threadId":JV, ...}`, a JV primitive, null, or undefined.

**Script.** Queues are consumed in call order. When a queue is exhausted, its last entry repeats. Defaults: homedir `/home/test`, cwd `/work`, ambient undefined.

| key | entries | default |
|---|---|---|
| `homedir`, `cwd` | `{"ok":JV string}` or `{"throw":ERR}` (a `{"u16":[...]}` result is decoded in the oracle like every JV string) | above |
| `ambient` | `{"ok":JV string or undef}` | undefined |
| `realpath` | `{"<path>":[{"ok":JV string}|{"errno":"ENOENT|EACCES|ELOOP|...","ref":..}...]}` | ENOENT |
| `stat` | `{"<path>":[{"ok":{"dev":"dec","ino":"dec"}}|{"errno":..}...]}` | ENOENT |
| `accounts` | `{"claude": {"ok":[row...]}|{"throw":ERR}, "codex": ...}`; a row is `{"id":JV,"name":JV}`, `null` or `{"undef":1}` | `{"ok":[]}` |
| `isTemporary` | call-order queue of `{"ok":JV}` / `{"throw":ERR}` | `{"ok":false}` |
| `engine` | `{"sync_throw":ERR}`, `{"reject":ERR}`, `{"rows":[{"ref":..,"title":JV,...extra}]}` or `{"record":"name"}` | — (required for ops 3/4) |

`dev` and `ino` are decimal digit strings (or non-negative integers) and are canonicalized by both the host and the oracle (BigInt rendering): leading zeros are stripped, `"0"` stays, so `"01"` and `"1"` are the same identity. Signs, spaces, radix prefixes, empty text and non-integers are refused as a bad script (host: case error, oracle: OracleError).

ERR is one of these forms, each with a required `"ref"`:

- `{"class":"Error|TypeError|EngineImportError|plain|primitive", ...}`;
- `"code"` (an import code, for EngineImportError);
- `"name"`, `"message"`, `"errcode"` (the `code` property);
- `"value":JV` (for `primitive`);
- `"rename":str` (sets `.name` after construction, R02).

`errno` errors are `Error` objects with `code`, plus a native-like message (P5).

### 5.2 Host driver loop

1. Encode args into the input packet, with ref ids assigned by first occurrence.
2. Call `begin`.
3. While the packet is an await:
   - check kind, op_id and that seq = previous + 1;
   - append the section 3 rows;
   - decode the request;
   - serve it from the script and append the trace entry;
   - build the completion;
   - call `step`.
4. On done, decode the terminal.

The host uses the canonical post-order rules:

- `accounts` snapshots the scripted array once.
- `is_temporary` calls the queue per id.
- `invoke_engine` records one engine call and returns rows. It keeps the original row objects (an ordered key/value list) by ref.

**Trace entries** use the same form on both sides:

- `["accounts","claude"]`
- `["ambient","CODEX_HOME"]`
- `["homedir"]`
- `["cwd"]`
- `["realpath",JV]`
- `["stat",JV]`
- `["isTemporary",JV]`
- `["engine",route,op,{"source":ref,"folder":JV,"thread":JV|absent,"options":ref}]`

account_at is not traced.

**Result JSON:**

```json
{"id":..,"terminal":"ok|throw_existing|throw_import|throw_account_wire|throw_plain_error|boundary_error",
 "value":..,"trace":[..],"driver":null|"InvalidPacket"}
```

The value depends on the terminal and op:

- **import_sources:** a list of source objects with key order `source, engine, kind, [account, accountName], home`.
- **find:** an index or `{"undef":1}`.
- **list:** an array of `["keep",ref]` / `["copy_title",ref,JV]`.
- **read:** `{"ref":..}`.
- **path ops:** JV.
- **threads:** `[[JV,JV]...]`.
- **excluded:** JV.
- **bound:** `[[JV,JV]...]`.
- **throw_existing:** `{"ref":..}`.
- **throw_import:** `{"code","message":JV,"cause":ref,"hasCause":true}`.
- **wire:** `{"key","message"}`.
- **plain:** `{"message"}`.
- **boundary:** `{"code"}`.

Strings are always rendered as JSON strings when well-formed, else as `{"u16":[...]}`.

### 5.3 Object entry order

For `engineThreads` the host reproduces JS own-property order:

1. Integer-index keys first, in ascending numeric order. A key is an integer index when it is the canonical decimal of an integer 0 ≤ n < 2^32−1 (no leading zeros except `0`, no sign).
2. Then the other keys in first-assignment order.
3. A repeated key keeps its first position and takes the last value.

This is the "verified at the codec boundary" rule of spec §4.2. V09 checks it against the oracle.

### 5.4 Probes (P06–P10, Q family G8)

A case may carry `"probe":{"at":k,"corrupt":C}`. It applies to the packet handed to the core at call k: 0 is begin, and k is the k-th step. C is one of:

- `version`
- `length`
- `tag`
- `unit` (pool word 65536)
- `op_id`
- `seq`
- `duplicate` (as §2.3)
- `reentry` (step on done)
- `oversize` (the input padded to 16,777,217 words)
- `exact_limit` (the input padded to exactly 16,777,216, which must run normally)

The expected result is in the fixture. These cases have no oracle comparison except `exact_limit`, which compares with the unpadded oracle result.

## 6. Oracle (`reference.mjs` + hooks)

The oracle runs Node 26.4.0 (it checks the version), invoked as
`node --experimental-strip-types --import ./reference-hooks.mjs reference.mjs < cases.jsonl`, one result line per case.

**Hooks** use `registerHooks` resolve/load, as in `../opencode/reference-hooks.mjs`:

- **`node:os`** imported by the pinned engine-import, codex-home and opencode helpers → `reference-primitives.mjs`. That module exports `homedir`, which serves the script.
- **`node:fs`** imported by codex-home → primitives. These export `realpathSync`/`statSync`, which serve the script. The real fs is used for anything else.
- **Pinned E's engine imports** → `fakes/engine-*.ts`:
  - `./engine-import/claude`, `codex`, `grok`, `opencode`, `qwen`.
  - The fakes record the route and arguments and serve the `engine` script.
  - Each fake is an `async` function. A `sync_throw` script entry is thrown before the first await.
  - `fakes/engine-opencode.ts` must re-export the **pinned** `opencodeDataDir`, and `fakes/engine-qwen.ts` the pinned `qwenRuntimeDir`. Load them from the pinned files with their other imports stubbed. Do not reimplement either helper.
  - The fakes also export dummy `copyGrokSession`, `grokLoadCheck` and `makeCodexRunDeps`.
- **`./engine-import/tail`** → the pinned tail, with its `child_process`/`engine-children`/`windows-system` imports stubbed. This keeps the real `EngineImportError` class.
- **`../shared/i18n/wire`** → `fakes/wire.ts`. Its `WireError extends Error` stores `key`; `wire(key)` returns `{key}`.
- **Other imports** → `fakes/stubs.ts`:
  - `./engine-path`, `./provider-accounts`, `./wire-reason`;
  - `../shared/contracts` (real if loadable, else `ENGINE_IDS` stub);
  - `codex-owned-runner` types, `node:crypto` (if it is a problem).
- **Process patches:**
  - `process.cwd` is patched to serve the script and record. The probe in §4.1 showed `path.posix.resolve` calls the patched function.
  - `process.env` is replaced for the call by a Proxy that records every key read. `CODEX_HOME` is served from `ambient`, and any other key read is an oracle error.
  - Both are restored after each case.

**Per op, the oracle calls the real export:**

- `importSources({dataDir, env, accounts})`, where `accounts` records and serves the script;
- `findSource`, reported via `indexOf` of the returned entry;
- `listExternal`, `readExternal`;
- `accountHome`;
- `codexAccountFolder`;
- `[...sessionThreads(s)]`;
- `excludedFromBound`;
- `[...boundSessionIndex(...)]`.

It classifies the outcome:

- **Ok list:** for each `i`, `out[i] === items[i]` → keep. Otherwise verify that `out[i]` is a plain object whose own enumerable entries equal `items[i]`'s except `title`, in the same key order → `copy_title`. Anything else is an oracle-detected mismatch.
- **Read:** the returned value must be the scripted record → `{"ref"}`.
- **Thrown values:**
  - a value identical to a scripted thrown value → throw_existing;
  - otherwise an EngineImportError → throw_import with the code, the message, and the cause mapped to a ref by `Object.is`, plus `'cause' in e`;
  - otherwise a WireError → key and message;
  - otherwise an Error → plain with its message;
  - anything else → `native_throw`. That is an oracle-side failure, reported as unexplained.

For B10, the oracle also runs a verbatim copy of R:133–134 (`titles.set` loop) over the case sessions and records the
last-wins title map as `"callerTitles"`. It is out of slice and not compared with Pruv; the report shows it.

## 7. Fixtures and comparison

`fixtures/<G>.json` holds the ten spec cases of group G (§7.1–7.15), each with an `"expect"` block:

- the exact terminal and value, and the trace where the spec names an observation;
- for I-group cases, the probe description (§8).

`_defaults.json` holds the spec defaults: homedir `/home/test`, cwd `/work`, dataDir `/data`, and the ids
`acc_0123456789abcdef` / `acc_fedcba9876543210`.

**test_impl.py:** for each fixture and each of the four pairs, run the host and compare:

- with `expect`;
- with the oracle (except probes);
- across the pairs (all four identical).

**Comparison (common.py):**

- Compare `terminal`, `value` and `trace` exactly (P0–P3), with no sorting.
- P4: refs are compared by name. account_at is absent from both traces.
- P5: when `realpath`/`stat`/`homedir`/`cwd` throws escape, their Error messages are not compared; the class, `code` and ref must match. Synthetic and wire messages compare exactly.
- P6: only probe cases with a recorded expected InputTooLarge.
- P7: I-group only (§8).

Each difference is reported as one of these classes:

- exact;
- P5 (native prose);
- P6 (resource bound);
- engine-policy (I only);
- excluded-domain (InvalidPacket on a recorded excluded input whose forbidden value is really reached: its session is not excluded by `temporary === true` or the scripted isTemporary answer, and no earlier answer throws; both sides noted);
- unexplained.

## 8. Forwarding integration (I01–I10, `integrate.py`)

The common host's `invoke_engine` handler has a forward mode. In that mode, the route is served by the existing piece's own driver, not a script:

| route | existing driver (built by that piece's `test_impl.py` in its `work/`) |
|---|---|
| claude | `bench/l3/claude` head/tail/window packet harness |
| qwen | `bench/l3/qwen` harness |
| codex | `bench/l3/codex` `codex_list` / `codex_read` harness |
| grok | `bench/l3/grok` begin/step harness with its fake child |
| opencode | `bench/l3/opencode` begin/step harness with `fake_exporter.py` |

`integrate.py` performs these steps:

1. For each route, pick one existing fixture of that piece in its exact domain, for list and for read.
2. Run the piece's driver once and capture its result JSON.
3. Register the captured rows or record as the engine result for the entry. Rows become refs with their titles.
4. Run the entry on `rs/rust`.

It then asserts:

- exactly one invocation;
- the route and arguments;
- for list:
  - the rows are unchanged except the common 200-unit title cut;
  - their order and metadata are preserved;
- for read: the same record object is returned;
- for errors: the piece's error object is forwarded to the classifier unchanged.

The existing drivers are built only if their `work/` artifacts are missing, and only for that piece (not full suites). No real
account, token or live service is touched. Any route whose existing driver cannot be run this way is reported as an integration
limit in RESULTS, not stubbed.

## 9. Q and mutants

**q.py** follows spec §8 exactly:

- xorshift32 seeded 20261118, with rejection-sampled `uniform(n)`;
- the exact family multiset (400/300/200/300/300/200/200/100), Fisher–Yates shuffled, then generated from the same stream;
- cases emitted as §5.1 lines;
- all four pairs and the oracle run.

It writes `work/q/report.json` with the class counts of §7. It persists each failing case with its seed and case number, its family, the first differing field, and a shrunk form.

**q_mutants.py:** one source mutation per mutant, each applied to a copy of the Pruv sources under `work/mut/Mxx/` and built on `rs/rust`. Then:

1. Run the killing fixture of spec §9.
2. Record killed (the fixture differs from expect, and not by crash or InternalInvariant) or verification-rejected.

| ID | mutation site |
|---|---|
| M01 | `u16.pruv` trim set → ASCII whitespace only |
| M02 | `sources.pruv` CODEX_HOME phase reads input env slot `CODEX_HOME` instead of `ambient_env` |
| M03 | `sources.pruv` OpenCode phase drops the absolute check |
| M04 | `sources.pruv` Codex account phase skips a row whose id equals the previously emitted Codex id (hash kept in ck6) |
| M05 | `sources.pruv` invalid Claude/Codex account → throw instead of skip |
| M06 | `canon.pruv` issues stat on ENOENT too |
| M07 | `canon.pruv` omits the `target == dot` and second sameFile checks |
| M08 | `dispatch.pruv` find keeps the last match |
| M09 | `dispatch.pruv` unknown engine → codex route |
| M10 | `dispatch.pruv` title cut backs off a trailing high surrogate |
| M11 | `dispatch.pruv` AbortError pass-through requires isError |
| M12 | `dispatch.pruv` non-Error detail = stringified value (pool text of `String(v)` for primitives; refs `[object Object]`) |
| M13 | `bound.pruv` terminal engine uses `\|\|` (truthiness) instead of `??` |
| M14 | `bound.pruv` slot prefix before the last colon |
| M15 | `bound.pruv` temporary truthiness instead of tag 3, and always calls the predicate |
| M16 | `bound.pruv` later write overwrites |

## 10. Proof obligations

Each pair must verify:

- every index into packet/pool/sections in bounds;
- every loop bounded (row counts, unit lengths, the path climb measure);
- no overflow in size sums;
- output size checked before building;
- the state decoder total (any words give a valid state or InvalidPacket);
- done packets never stepped.

Structural invariants:

- the emitted row shape;
- recipe rows referring only to completion rowRefs;
- a terminal ref always one received in the same op.

TS equivalence is established by the differential tests, not proved (spec §10.3).

## 11. Delegation split (write workers, file-bounded)

Every write worker runs only its own tests and never the full suites. The split, in order:

| step | files | worker test |
|---|---|---|
| W1 | `pruv.json`, `root.pruv`, `packet.pruv`, `value.pruv`, `u16.pruv`, `path.pruv`, `accounts.pruv`, `state.pruv`, `canon.pruv`, `sources.pruv`, `dispatch.pruv`, `bound.pruv` | none (no shell); compile errors are fixed by the lead |
| W2 | `host.rs`, `host/*`, `harness.rs`, `str_adapter.rs` | none |
| W3 | `reference*.mjs`, `fakes/*`, `oracle.py`, `common.py` | none |
| W4 | `fixtures/*`, `test_impl.py`, `integrate.py` | none |
| W5 | `q.py`, `q_mutants.py`, `README.md`, `Q_README.md` | none |

The lead builds, runs, fixes, and records actual results in `IMPL_NOTES.md` and RESULTS.md §140.
