# OpenCode import: package A notes and the cross-module contract (packages B, C, D code against this)

Package A = `root.pruv`, `packet.pruv`, `util.pruv`, `pruv.json`. **Written without a shell: nothing here was compiled, verified or run.**
Expect compiler/verifier fixes. The spec is `docs/OPENCODE_IMPORT_SPEC.md`, then `DESIGN.md` (sections 1, 2, 3, 5). If this file and DESIGN.md
disagree on a word offset, DESIGN.md wins; additions beyond DESIGN.md are marked **(A-added)**.

## 1. Rules for every module

- Build every returned step packet with `crate::packet::step_packet` / `done_*` (never by hand). Root re-checks it.
- `ctl` arrives borrowed: use `ctl_next` / `ctl_await` / `ctl_final` (they copy), or `let mut c: [i64] = copy(ctl);`.
- Identifiers to avoid: `async`, `result` (outside ensures), `ref`, `mod`, `type`, `move`, `box`, `final`, `loop`.
- Module-level calls use full paths `crate::packet::...`, `crate::util::...`. Only root has `mod` lines (all modules incl.
  `path admit sql rows list export entry convert plan` are declared there). No consts: constants are nullary fns.
- A phase a module does not own is `done_internal(ctl, 28)`.

## 2. The `advance` signature (admit, sql, list, export, convert)

Root calls exactly (same text for each module; `convert::advance` is called only for mode 1 at phase 500):

```pruv
pub fn advance(cfg: [i64], cfg_text: [u8], ctl: [i64], query_words: [i64], query_bytes: [u8], rows: [i64], text: [u8],
               document: [u8], saved_result: [i64], event: [i64], completion: [u8]) -> [i64] ! alloc
```

Required contract (declare **at most** this; it is what root proves before every call):

```pruv
  requires len(cfg) >= 32 && len(cfg) <= 16416 && 0 <= cfg[1] && cfg[1] <= 5 && len(ctl) == 64 && len(event) >= 16
  requires 1 <= ctl[3] && ctl[3] <= 1000000000 && 0 <= ctl[4] && ctl[4] < ctl[3] && 0 <= ctl[2] && ctl[2] <= 999
  requires 0 <= ctl[5] && ctl[5] <= 15 && 0 <= ctl[6] && ctl[6] <= 2 && ctl[1] == cfg[1]
  requires len(cfg_text) <= 2147483647 && len(query_words) <= 2147483647 && len(query_bytes) <= 2147483647 && len(rows) <= 2147483647
  requires len(text) <= 2147483647 && len(document) <= 2147483647 && len(saved_result) <= 2147483647 && len(completion) <= 2147483647
  ensures len(result) >= 76
```

Root has also checked at runtime (the verifier cannot see it): cfg is `32+4N` words (N<=4096), version 1, `cfg[18]==N`, refs absent `(-1,0)` or inside `cfg_text`,
`cfg[2..4] >= 1` (timeouts), `cfg[5]` 0/1, direct plan ref (words 19,20) present iff mode 5, reserved 21..31 zero, environment table valid (unique nonempty keys
without NUL/`=`, values without NUL); ctl fully consistent (`ctl_ok_at`: refs at 24..37 inside `len(text)`, handles >= -1, flags 0/1, `ctl[20] <= ctl[21]`, reserved
40..63 zero, `(ctl[4]==0)==(ctl[5]==0)`, `ctl[6]==1` iff pending, `ctl[6]==2` iff phase 999); event shape (`event_shape`); if `ctl[6]==0` the event is the empty event,
if `ctl[6]==1` it is a completion with `event[1]==ctl[4]` and `event[2]==ctl[5]` (so the completion is **already matched**; the module still decodes status/class).
Phase 999 / `ctl[6]==2` never reaches a module.

Other functions callers expect (all `! alloc`; Package B/D write them):

| callee | signature (exact) | called by |
|---|---|---|
| `path::utility` | `pub fn utility(cfg: [i64], cfg_text: [u8], mode: i64) -> [i64] ! alloc` with `requires len(cfg) >= 32 && len(cfg) <= 16416 && (mode == 3 \|\| mode == 4) && len(cfg_text) <= 2147483647`, `ensures len(result) >= 40`. Returns a **result packet** (`ok_value(mode, bytes)`), not a step packet | root, phase 600 |
| `plan::direct` | `pub fn direct(cfg: [i64], cfg_text: [u8]) -> [i64] ! alloc` with `requires len(cfg) >= 32 && len(cfg) <= 16416 && cfg[1] == 5 && 0 <= cfg[19] && 0 <= cfg[20] && len(cfg_text) <= 2147483647`, `ensures len(result) >= 40`. The input JSON array is `cfg_text[cfg[19]..cfg[19]+cfg[20]]`. Returns a **result packet** (`ok_plan_absent()` or a plan result), bad JSON/shape = read-failed at stage 24/25/27 | root, phase 500, mode 5 |

Root wraps both results with `done_wrap(ctl, res)` (which re-checks `result_shape`).

## 3. Phases and hand-off

| phase | owner | meaning |
|---|---|---|
| 0–99 | none | invalid (root: InternalInvariant stage 28) |
| 100–199 | `admit::advance` | modes 0,1,2: thread check (read), `path` computations, realpath(dirname DB) / lstat(DB) / realpath(DB), containment |
| 200–299 | `sql::advance` | db_open_readonly, schema queries, selection prepare/all, `rows::decode` into bank 2 |
| 300–399 | `list::advance` | counts, db_close, list clock, list/rows result, read hand-off (first row, folder, binary/tempRoot) |
| 400–499 | `export::advance` | mkdtemp/open/spawn/wait/TERM/KILL/close/stat/read/rm; leaves raw bytes in bank 4 or a saved result |
| 500 | root | mode 5: `plan::direct`, done. mode 1: `convert::advance` (the module owns exactly phase 500 and may use ctl[7] / ctl words for its own cursor, staying at 500) |
| 501–599 | none | invalid |
| 600 | root | modes 3,4: `path::utility`, done |
| 601–998 | none | invalid |
| 999 | terminal | set only by `done_*`; root rejects any further step |

Start phase from `opencode_begin`: modes 0,1,2 → 100; 3,4 → 600; 5 → 500. `opencode_begin` returns an action-2 packet with the empty event expected.

Recommended hand-off (the *owner changes the phase before a handoff*; a hand-off is a **continue** packet = `step_packet(ctl_next(c, newphase) with another word changed, patches, ep, words, bytes, ep, 2)`):

1. **admit** (100): list/read/rows. Read: `names::valid_thread` before any request (an invalid thread fails at stage 0 `input`; the exact class/code is
   fixed by the spec, B implements it — package A does not decide it). Stores DB path ref in ctl[24,25] and resolved parent in ctl[26,27] (bank 3 refs, text patched first).
   Success → phase 200. Admission failure = absence: **list** → phase 300 with ctl[8] = -1 (no DB; list takes the wall clock once and returns an empty list), **rows** → done with
   zero rows, **read** → done not-found. ctl[23] = return mode after DB work (0 list, 1 read, 2 rows).
2. **sql** (200): handles ctl[8] DB, ctl[9] selection stmt, ctl[10] count stmt (-1 absent). Missing required column → read-failed/detail `columns`, no message-schema query.
   Writes validated rows to bank 2 (`rows::decode` output, 7 words each) and ctl[21] = validated row count, ctl[20] = 0, ctl[22] = archive-column flag. Hand-off → 300.
   On a DB error: save the error result packet into bank 5 (`patch_resize(5,n)` + `patch_copy(5,0,n,3,0)`), ctl[38]=1, ctl[7] = 300 + close sub-phase, go to the DB close.
3. **list** (300–399): count get per validated row for mode 0 (cursor ctl[20] ≤ ctl[21]), then db_close (stage 10, failure ignored), then list-clock (stage 12, mode 0 only). Mode 0/2:
   done with list/rows result. Mode 1: after close, first validated row, folder compare (`folderKey`), binary/tempRoot truthiness; wrong folder/missing row → not-found done;
   missing/empty binary or tempRoot → engine-missing done; else store fallback title in ctl[32,33], first directory ctl[34,35], first thread ctl[36,37] and hand off to **400**.
   If ctl[38]==1 after the close, finish with the saved result: `done_wrap(ctl, saved_result)`.
4. **export** (400): mkdtemp → open (wx 0600) → spawn → clock_now → observe loop (the DESIGN 3 transition table). Handles ctl[11] output file, ctl[12] child; ctl[13] timed-out, ctl[14] settled,
   ctl[15] child ended, ctl[16] start, ctl[17] deadline, ctl[18] last observation, ctl[19] termination stage; temp dir ctl[28,29], export path ctl[30,31]. After rm: if ctl[38]==1 →
   `done_wrap(ctl, saved_result)`; else hand off to **500** with the bytes in bank 4 (and ctl[39] = stat size). Oversized: result `{turns:[],omittedBefore:true}` + title built by export/convert.
5. **convert** (500, mode 1): decode bank 4, build the read result packet (mode 1) with the fallback title from ctl[32,33]; done.

Bank patches: a word bank (0 query words, 2 rows, 5 saved result) takes sources 1 (event words, absolute index in the event array, payload starts at word 16) or 3 (literal words);
a byte bank (1 query bytes, 3 text, 4 document) takes 2 (completion bytes) or 4 (literal bytes). Always resize first (`patch_resize`), then copy; offsets/counts are in the destination bank's
elements. Patches apply in order before the next request; root checks event/completion sources against `len(event)` / `len(completion)`.

## 4. ctl words (DESIGN 2; indices)

0 version 1 · 1 mode · 2 phase · 3 next request id · 4 pending id (0 none) · 5 pending opcode · 6 expected event (0 empty, 1 completion, 2 terminal) · 7 continuation phase ·
8 DB handle · 9 selection stmt · 10 count stmt · 11 output file · 12 child · 13 timed-out · 14 settled · 15 child ended · 16 monotonic start · 17 deadline · 18 last observation ·
19 termination stage (0 waiting, 1 TERM, 2 KILL) · 20 next row · 21 row count · 22 archive flag · 23 return mode · refs (start,len in bank 3, start -1 = absent) 24 DB path ·
26 resolved parent · 28 temp dir · 30 export path · 32 fallback title · 34 first directory · 36 first thread · 38 saved-result present · 39 export stat size · 40–63 reserved 0.
`ctl_init(mode, phase)`: handles 8..12 = -1, refs absent, next id 1, expected event 0 (2 if phase 999). Everything else 0. Root checks the outgoing ctl with the same rules
(refs only structurally, since the text bank may be patched).

## 5. Emitting one request and consuming its completion

To issue a request: `let id: i64 = ctl[3];` (precondition for `ctl_await`: `1 <= id <= 999999999`, `1 <= opcode <= 15`, new phase 0..998).

```pruv
let c: [i64] = crate::packet::ctl_await(ctl, 210, 4);          // phase that will consume the completion; ctl[3]=id+1, ctl[4]=id, ctl[5]=4, ctl[6]=1
let req: [i64] = crate::packet::request(id, 4, 5, 0, ctl[8], 0, 0, 0, 0, 2); // id,opcode,stage,flags,handle,a,b,c,argStart,argWords
return crate::packet::step_packet(c, patches, req, words, bytes, ep, 0);      // action 0: exactly one request
```

Argument words are in the step `words`; string/blob refs in them point into the step `bytes` pool (put each string with `append_str`/`cat_bytes`, remember `len(pool_before)` as its start).
`step_packet` checks per opcode: 1,2,3,6,7,12,13,14 and prepare (opcode 4, flags 0) take exactly `[pathRef]` = 2 words (present, inside `bytes`); 5,9,10,11,15 take 0 words;
execute (opcode 4, flags 1 = all, 2 = get, per DESIGN 3) takes `[P, 4*P typed values]`; spawn (8) takes `12 + 2*argc + 4*envCount` words (scalars as in DESIGN 3, refs inside `bytes`).
Request ids are exactly `ctl[3]` and the new `ctl[3]` is `id+1` (**(A-added)**: one request at a time, ids consecutive, guard: no request when `ctl[3] >= 1000000000`).
Row scalars `a,b,c,handle` must be `>= 0` (`handle >= -1`).

To consume a completion (the step after an await; `ctl[6]==1`): decode `event[3]` status (0 ok, 1 error), `event[4]` class (0 none, 1 OS, 2 SQLite, 3 binding-range, 4 host-capacity, 5 invalid-protocol),
`event[5]` native code, code text `completion[event[8]..+event[9]]`, message text `[event[10]..+event[11]]`, payload words `event[16..16+event[6]]`. Then the new ctl is built with `ctl_next`
(nothing pending) or `ctl_await` (a new request) or `ctl_final`/`done_*`. A completion is consumed exactly once: root rejects a packet whose pending id equals the consumed one.
The empty event (continue, `ctl[6]==0`) is `event_empty(event)`; the module never reads a payload from it.

Progress rule (root): a continue must change at least one ctl word relative to the incoming ctl (change the phase or a cursor/handle); an await has exactly one request whose id/opcode equal `ctl[4]`/`ctl[5]`
and `ctl[6]==1`; a done has phase 999, `ctl[4]=ctl[5]=0`, `ctl[6]==2`, no patches and no request, and a `result_shape`-valid result; request ids never decrease.

## 6. `packet.pruv` helpers (all `pub`; the names other packages call)

| function | requires | ensures / notes |
|---|---|---|
| `step_magic() / result_magic()` | | `0x4f435331` = 1329812273 / `0x4f435231` = 1329812017 |
| `ctl_len() head_len() step_head() request_words() patch_words() event_head() result_head() bank_max()` | | 64, 76, 12, 12, 8, 16, 40, 2147483647 |
| `ref_ok(s,n,total) -> bool` | none | true ⇒ `total>=0 && ((s==-1 && n==0) \|\| (0<=s && s<=total && 0<=n && n<=total-s))` |
| `cat_words(a,b) -> [i64]`, `cat_bytes(a,b) -> [u8]` | each `len <= 1073741823` | `len == len(a)+len(b)` (join patch rows / request rows / pools) |
| `append_str(a:[u8], s:str) -> [u8]` | `len(a), len(s) <= 1073741823` | `len == len(a)+len(s)`; new bytes start at `len(a)` |
| `ctl_init(mode,phase) -> [i64]` | `0<=mode<=5 && 0<=phase<=999` | `len==64`, `[1]==mode`, `[2]==phase`, `[3]==1` |
| `ctl_final(ctl) -> [i64]` | `len(ctl)==64` | phase 999, `[4]=[5]=0`, `[6]=2` |
| `ctl_next(ctl,phase) -> [i64]` | `len(ctl)==64 && 0<=phase<=998` | `[2]==phase`, `[4]=[5]=[6]=0`, next id unchanged. Change another word too if the phase is unchanged |
| `ctl_await(ctl,phase,opcode) -> [i64]` | `len(ctl)==64 && 1<=ctl[3]<=999999999 && 0<=phase<=998 && 1<=opcode<=15` | `[2]==phase`, `[3]==ctl[3]+1`, `[4]==ctl[3]`, `[5]==opcode`, `[6]==1` |
| `patch_resize(bank,newlen) -> [i64]` | `0<=bank<=5 && 0<=newlen<=2147483647` | `len==8` |
| `patch_copy(bank,dst_off,count,src,src_off) -> [i64]` | `0<=bank<=5 && 0<=dst_off && 0<=count && 1<=src<=4 && 0<=src_off` | `len==8` |
| `request(id,opcode,stage,flags,handle,a,b,c,arg_start,arg_words) -> [i64]` | `1<=id && 1<=opcode<=15 && 0<=stage<=28 && 0<=flags && -1<=handle && 0<=a,b,c && 0<=arg_start && 0<=arg_words` | `len==12` |
| `step_packet(ctl,patches,requests,words,bytes,res,action) -> [i64]` | `len(ctl)==64 && len(patches)<=4096 && len(requests)<=4096 && len(words),len(bytes),len(res)<=2147483647` | `len>=76`. Runtime shape check, else a done packet with InternalInvariant stage 28: ctl version 1, phase 0..999, `len(patches)%8==0` (≤512 rows), requests 0 or exactly 12 words, action 0 ⇒ one request and `ctl[4]/[5]/[6]` = id/opcode/1, action 1 ⇒ no patches/requests + `result_shape(res)` + terminal ctl, action 2 ⇒ no request + `ctl[4]=[5]=[6]=0` + phase ≠ 999; patch ranges, bank/source type match, literal-source bounds; request fields and per-opcode argument shape |
| `done_wrap(ctl,res) -> [i64]` | `len(ctl)==64` | `len>=76`; finalises ctl, action 1; a malformed `res` becomes InternalInvariant |
| `done_error(ctl,stage,cls,code,native,raw_code:[u8],name:str,detail:str) -> [i64]` | `len(ctl)==64` | `len>=76`; mode from `ctl[1]` |
| `done_simple(ctl,stage,cls,code) -> [i64]` | `len(ctl)==64` | text-free error; out-of-range enums give InternalInvariant |
| `done_internal(ctl,stage) -> [i64]` | `len(ctl)==64` | CoreError (3) / InternalInvariant (6) at `stage` |
| `meta_new(mode) -> [i64]` | `0<=mode<=5` | `len==40`, magic/version/mode set, status 0, refs at 10,12,14,16,23,25,27 absent. Caller sets `[4]` N, `[5]` W, `[8]` omitted, `[9]` planPresent, `[10..17]` refs, `[18]` more |
| `ok_result(meta,rows,items,pool) -> [i64]` | `len(meta)==40 && len(rows)<=1073741823 && len(items)<=4096 && len(pool)<=1073741823` | `len>=40`. Fills K (`len(items)/3`) and T; runs `result_shape` (failure = InternalInvariant); combined size over 2^31-1 = CoreError/InputTooLarge stage 0 |
| `ok_value(mode,value:[u8]) -> [i64]` | `mode 3 or 4`, `len(value)<=1073741823` | present scalar ref `(0,len)` (empty value is valid) |
| `ok_plan_absent() -> [i64]` | | mode 5 with planPresent 0 |
| `simple_error(mode,stage,cls,code) -> [i64]` | `0<=mode<=5 && 0<=stage<=28 && 1<=cls<=3 && 0<=code<=6` | `len==40` |
| `internal_packet(mode) -> [i64]` | `0<=mode<=5` | `len==40`: CoreError/InternalInvariant stage 28 |
| `error(mode,stage,cls,code,native,raw_code:[u8],name:str,detail:str) -> [i64]` | none | `len>=40`; bytes `raw_code ++ name ++ detail`, an empty string is an absent ref; bad enums or strings over 10 MB → internal |
| `result_shape(res) -> bool` | `len(res) <= 2147483647` | see below |
| `event_empty(event) -> bool` | `len(event) >= 16` | version 1, everything 0, code/message refs `(-1,0)`, exactly 16 words |
| `event_shape(event, clen) -> bool` | none | true ⇒ `len(event) >= 16`; version 1, id/native ≥ 0, opcode 0..15, status 0/1, class 0..5, `len == 16 + event[6]`, `event[7] == clen`, words 12..15 zero, refs inside the completion, status 1 ⇒ class ≥ 1 and no payload |

Result packet (DESIGN 5): `[0x4f435231, 1, status, mode, N, W, K, T, omitted, planPresent, title(2), model(2), value(2), planAt(2), more, stage, class, code, native, rawCode(2), name(2), detail(2), 29..39 = 0, rows(N*W), items(3*K), bytes(T)]`, length `40 + N*W + 3*K + T`.
`result_shape` enforces: widths by mode (0 list 10; 1 read 10 with N==1; 2 rows 7; 3,4,5 → 0), plan items K≤30 (status 0..3, refs inside the pool), `planPresent==0` ⇒ `K==0`, `more==0`, `planAt` absent; `planPresent==1` ⇒ `1<=K<=30`,
`more>=0`; refs absent `(-1,0)` or inside the byte pool and on UTF-8 scalar boundaries; byte values 0..255; list rows require thread and title refs present; read rows require prompt and reply present, N==1; rows rows require id/directory/title present;
dataDir/folderKey require the value ref present; error packets have N=W=K=0, no title/model/value/plan, `stage<=28`, class 1..3, code 0..6 with class 1 ⇒ code 1..4 and class 3 ⇒ code 5/6. Title/model refs are only allowed in mode 1.
Row words: list `[threadRef(2), titleRef(2), updatedAtRef(2), messagesPresent, messages, maybeOpen, 0]`; read `[promptRef(2), replyRef(2), createdAtRef(2), completedAtRef(2), tools, clipped]`; rows `[idRef(2), directoryRef(2), titleRef(2), updatedMs]`.

### Error enums (DESIGN 5)

class 1 EngineImportError, 2 RawOsError, 3 CoreError · code 0 none, 1 not-found, 2 read-failed, 3 engine-missing, 4 timeout, 5 InputTooLarge, 6 InternalInvariant ·
stage 0 input, 1 db-parent, 2 db-lstat, 3 db-realpath, 4 db-open, 5 schema-session, 6 schema-message, 7 select, 8 count-prepare, 9 count, 10 db-close, 11 read-admission, 12 list-clock, 13 temp-create,
14 output-open, 15 spawn, 16 wait, 17 term, 18 kill, 19 give-up, 20 output-close, 21 output-stat, 22 output-read, 23 cleanup, 24 parse, 25 shape, 26 convert, 27 plan, 28 result.
Root itself only reports stage 0 (invalid config, banks over 2^31-1 = InputTooLarge) and stage 28 (InternalInvariant: any packet/ctl/event violation, bad phase).

## 7. `util.pruv` helpers (all `pub`)

| function | contract | notes |
|---|---|---|
| `to_bytes(s:str) -> [u8]` | `len==len(s)` | |
| `span_bytes(t,s,l) -> [u8]` | `len<=len(t)` | empty for an absent/invalid ref |
| `cfg_str(t,s,l) -> str` | `len<=1000000` | `jstr::adapter_probe`; `""` for absent/invalid/invalid UTF-8/over 1000000 |
| `bank_limit() -> i64` | `== 2147483647` | |
| `fits(used,add,limit) -> bool` | true ⇒ `0<=used<=limit && 0<=add<=limit-used` | overflow-free |
| `bank_fits(cur,add) -> bool` | true ⇒ `cur+add <= 2147483647` (both ≥0) | false ⇒ report CoreError/InputTooLarge (stage 0 or the current stage), no partial success |
| `pool_bytes(pool,add)`, `pool_str(pool,s)`, `pool_dec(pool,n)` | lens ≤ 1073741823, `n >= -9223372036854775807` | pool ++ add; `pool_dec` appends the decimal |
| `format_i64(n) -> [u8]` | `n >= -9223372036854775807` ⇒ `1<=len<=20` | decimal, no leading zeros |
| `real_50_hi()` / `real_50_lo()` | `== 1078525952` (0x40490000) / `== 0` | the LIMIT parameter 50.0 as REAL `[2, hi, lo, 0]` |
| `limbs_ok(hi,lo)`, `real_negative(hi)`, `real_exp(hi)` (0..2047), `real_mantissa(hi,lo)` (0..2^52-1), `real_is_finite(hi)` | limbs in 0..4294967295 | binary64 from unsigned limbs |
| `real_is_integral(hi,lo)` | | finite whole number (±0 included) |
| `real_is_safe_integer(hi,lo)` | | integral and `|v| <= 9007199254740991` |
| `real_to_integer(hi,lo) -> i64` | `-9007199254740991 <= result <= 9007199254740991` | exact value if `real_is_safe_integer`, else 0 (negative zero → 0) |
| `raw(d,id) -> [u8]`, `str_at(d,id) -> str` (`len<=50331648`), `elements(d,id) -> [i64]` | as Grok | `elements`: one-pass child ids of an array; empty for non-array/empty/corrupt/over 1048576 |

## 8. Deviations from DESIGN.md and judgement calls

- **Request rows carry a pending slot only (A-added checks).** Root requires the request id to equal `ctl[3]` and the new next id `ctl[3]+1`; the packet layer ties `ctl[4]/[5]/[6]` to the request. DESIGN only says "ids increase".
- **`plan::direct(cfg, cfg_text)`** (no document bank): the direct plan input is the cfg_text ref in words 19,20, as DESIGN 2 lists it under config; bank 4 stays the export bytes. `path::utility(cfg, cfg_text, mode)` likewise.
- **Config strictness (A-added):** timeouts `>= 1`, direct plan ref present iff mode 5, env table at most 4096 pairs (cfg length 16416). The DESIGN says "positive" for test overrides only; real configs use 20,000/1,500/1,000.
- **Expected-event and pending words are packet-checked** (`step_packet`), not only root-checked, so a module cannot build a malformed packet even in unit tests.
- **Event stricter than DESIGN:** a failed completion (status 1) must have class ≥ 1 and no payload; the empty event requires code/message refs `(-1,0)`.
- **Result-shape strictness (A-added):** UTF-8 scalar-boundary refs, list rows need present thread/title, read has exactly one row, class/code pairs as above.
- **Verifier-sensitive spots (unproven; expect fixes):** `packet::result_shape` (`n*w` handled per width), `packet::values_ok`/`args_ok` index arithmetic, `root::out_ok` (`76+8*pc` indexing), `root::env_ok` nested loops, `util::format_i64` (`48 + v % 10` stored into `u8`).
- Not done in package A: any module marked B/C/D above, host, fixtures, harness. Nothing was compiled, built or run.
