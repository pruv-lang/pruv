# Grok import: package A notes and the cross-module contract (packages B–E code against this)

Package A = `root.pruv`, `packet.pruv`, `util.pruv`, `pruv.json`. Written without a shell: **nothing here was compiled or verified**.
The spec is `DESIGN.md` (sections 1–3) and `docs/GROK_IMPORT_SPEC.md`. If this file and DESIGN.md disagree on a word offset, DESIGN.md wins
and this file has a bug; the only additions to DESIGN.md are marked **(A-added)**.

## 1. Rules for every module

- Every module function that returns a step packet builds it with `crate::packet::step_packet` / `done_*` (never by hand). Root re-checks it.
- `ctl` arrives borrowed: `let mut c: [i64] = copy(ctl);`, edit `c`, pass `c` to `step_packet`. Slot `s` record base: `128 + 32*s` (`crate::packet::slot_base(s)`).
- Names to avoid as identifiers: `async`, `result` (outside ensures), `ref`, `mod`, `type`, `move`, `box`, `final`, `loop`.
- Module-level calls use full paths: `crate::packet::...`, `crate::util::...`. Only root has `mod` lines (all modules incl. `path summary list framing entry read copy rpc` are already declared there).

## 2. The `advance` signature (all five modules)

Root calls exactly (same text for each module):

```pruv
pub fn advance(cfg: [i64], cfg_text: [u8], ctl: [i64], directory: [i64], rows: [i64], text: [u8],
               small0: [u8], small1: [u8], small2: [u8], small3: [u8], history: [u8],
               event: [i64], completion: [u8]) -> [i64] ! alloc          // path, summary, list, read
pub fn advance( ...same parameters... ) -> [i64] ! async, net, proc, alloc  // copy only
```

Required contract (what root proves before every call; declare **at most** this):

```pruv
  requires len(cfg) == 24 && 0 <= cfg[2] && cfg[2] <= 2147483647 && len(ctl) == 256 && len(event) >= 24
  requires 1 <= ctl[3] && ctl[3] <= 1000000000 && 0 <= ctl[4] && ctl[4] <= 15 && 0 <= ctl[2] && ctl[2] <= 999
  requires len(cfg_text) <= 100000000 && len(directory) <= 100000000 && len(rows) <= 100000000 && len(text) <= 100000000
  requires len(small0) <= 100000000 && len(small1) <= 100000000 && len(small2) <= 100000000 && len(small3) <= 100000000
  requires len(history) <= 100000000 && len(completion) <= 100000000
  ensures len(result) >= 268
```

Root has also already checked at runtime (modules may rely on it, the verifier cannot see it): every cfg ref is absent `(-1,0)` or inside
`cfg_text` and at most 1000000 long; `ctl` is well formed (below); the event passed `packet::event_shape(event, len(completion))` (version 1, ranges,
`event[18] == len(completion)`, refs inside the completion, readdir rows consistent: `event[17]==3 && len(event)==24+3*event[16]`, name refs inside
the completion bytes); when `ctl[4] != 0` and the event is not empty, the completion matches the outstanding request of its slot (id, slot, opcode).
The completion that root passes on is **already matched**: the module must still decode status/errno.

Mode/phase: root dispatches by `ctl[2]` only, not by mode. A module must treat a phase it does not know as `done_internal(ctl, 30)`.

Other functions of the table (non-`advance`) keep the §3 signatures. The ones other packages call:

| callee | signature (exact) | caller |
|---|---|---|
| `rpc::load_check` | `pub fn load_check(binary: str, home: str, folder: str, thread: str, timeout_ms: i64) -> [i64] ! async, net, proc, alloc` with `requires len(binary) <= 1000000 && len(home) <= 1000000 && len(folder) <= 1000000 && len(thread) <= 1000000 && 0 <= timeout_ms && timeout_ms <= 2147483647` and `ensures len(result) >= 28` | root (phase 600, `grok_load`), copy (after the rename completion) |

`load_check` returns a **result packet** (§2 layout, `packet::result`/`error*`), **not** a step packet: status 0 with mode 3 meta for success, status 1 with
stage 24–26 and class/code for failure. `copy::advance` inspects `res[0]` (0 success, 1 error; meta words are `res[4..28]`), decides cleanup, and wraps with
`packet::done_wrap`. Root's phase 600 (mode 3) does `done_wrap(ctl, load_check(...))`. `grok_load` returns it directly (the host renders result packets).
Root's `grok_load` and phase 600 pass `cfg` words 12,13 (binary), 10,11 (**target home**), 6,7 (folder), 8,9 (thread), 2 (timeout); strings that are invalid
UTF-8 or absent arrive as `""` (via `util::cfg_str`).

## 3. Phase ranges and hand-off

| phase | owner | meaning |
|---|---|---|
| 0–99 | none | invalid (root answers InternalInvariant, stage 30) |
| 100–199 | `path::advance` | input checks, realHome, chain (home/sessions, folder, thread), folder encoding. Modes 0 list and 1 read start here. |
| 200–299 | `summary::advance` | small-json state machine and summary/active decisions. Mode 4 starts here; mode 1 hands to it after the chain; mode 0 uses it per candidate and for `active_sessions.json`. |
| 300–399 | `list::advance` | readdir, four-slot scheduler, sort, top 50, active file, final list result |
| 400–499 | `read::advance` | history lstat/realpath/open, identity fstat, size fstat, 256 KiB reads, close, `read::window` |
| 500–599 | `copy::advance` | mode 2 starts at 500: input checks, path building, realHome/chain (copy may call `path::` helpers directly rather than hand off), first lstat, entropy, mkdirs, transfer, second lstat, rename, `load_check`, cleanup |
| 600 | root | mode 3: `load_check` once, done |
| 600–699 other | `rpc` reserved | unused by root today (root only handles exactly 600); a module may not return a phase in 601–699 unless root is extended |
| 700 | root | mode 5: issue the rm of the path in cfg words 20,21 |
| 701 | root | mode 5 / `grok_caller_cleanup`: the rm completion arrived; done with no rows (rm errors are ignored) |
| 702–899 | none | invalid |
| 900–998 | none | invalid |
| 999 | terminal | set only by `done_*`; root rejects any further step |

Initial phase after `grok_begin`: mode 0 → 100, 1 → 100, 2 → 500, 3 → 600, 4 → 200, 5 → 700. `grok_begin` returns an action-2 (continue) packet; the host
then calls `grok_step` with the empty event.

Hand-off convention: a module that finishes its part returns a **continue** packet (`step_packet(c, patches, empty, words, bytes, empty, 2)`) with the next
module's phase written into `c[2]`; the host applies the patches, passes the empty event, and root dispatches to the new owner. To return the result
mode-specific data across a hand-off use ctl words (refs into bank 2, e.g. 16–27) — there is no other channel. Which module sets which phase is
for B–E to agree; the DESIGN §1 ctl table (16–29, slot +16…+29) is the storage. A module may also stay in its own range and use slot phases (`slot + 0`).

Progress rule (enforced by root): a continue packet must change at least one ctl word (it must not return the unchanged ctl). An await packet needs at least
one request or a non-zero pending mask. A done packet carries a result and no requests; it may be built while other slots are still outstanding (the host
discards them).

## 4. ctl words (DESIGN §1; indices)

0 schema(1) · 1 mode · 2 phase · 3 next request id · 4 pending bitmask (bit s = slot s) · 5 next readdir row · 6 readdir row count · 7 finished candidates · 8 admitted ·
9 copy skipped · 10 window start · 11 window end · 12 next history read offset · 13 verify called · 14 window omittedBefore · 15 follow-up phase ·
16,17 homeReal ref · 18,19 temp path ref · 20,21 title ref · 22,23 model ref · 24,25 copy path ref · 26,27 pending error ref · 28 small-json purpose · 29 close policy ·
30 **(A-added, root-owned)**: 1 = expects the empty event, 2 = expects a completion, 3 = terminal. Modules must never write it (root overwrites it on every packet it returns).
31–127 reserved 0.

Slot `s` record at `128 + 32*s + k`: 0 phase · 1 row index or -1 · **2 outstanding request id (0 none)** · **3 outstanding opcode** · 4 source handle or -1 · 5 dest handle or -1 ·
6 size · 7,8 dev hi/lo · 9,10 ino hi/lo · 11 next offset · 12 unwritten bytes of chunk · 13 small-json purpose · 14 return phase · 15 result present ·
16,17 title · 18,19 model · 20,21 updatedAt · 22,23 messages decimal · 24,25 file name · 26 small-json bytes · 27 chunk write cursor · 28 follow-up phase · 29 EOF seen · 30,31 reserved 0.

Initial values from `packet::ctl_init(mode, phase)`: schema 1, `ctl[3] = 1`, `ctl[30] = 1`, refs at 16,18,20,22,24,26 = -1 (absent), and for each slot
`+1, +4, +5 = -1` and refs `+16,+18,+20,+22,+24 = -1`. Everything else 0. A ref in ctl is `(start,len)` into **bank 2**, `-1` start = absent.

Constants (all in `packet.pruv`): `magic()=1196577585 (0x47525331)`, `ctl_len()=256`, `head_len()=268`, `c_mode()=1 c_phase()=2 c_next_id()=3 c_pending()=4 c_state()=30`,
`slot_base(s)=128+32*s (0<=s<=3)`, `bank_directory()=0 bank_rows()=1 bank_text()=2 bank_small(s)=3+s bank_history()=7`.

### Request id, slot records, pending mask (the protocol root enforces)

To emit a request on slot `s` (0..3), in the new ctl `c`:

1. `let id: i64 = ctl[3];` (1 ≤ id ≤ 1e9); `c[3] = id + c_count` where `c_count` is the number of requests this packet issues (at most 4; ids are consecutive `id, id+1, …`, any order of rows).
2. `c[128+32*s+2] = id;  c[128+32*s+3] = opcode;`
3. `c[4] = bor(c[4], 2^s)` — `2^s` is 1,2,4,8; bit `s` set **iff** the slot record id ≠ 0 (root rejects any other combination in both the incoming and the outgoing ctl; opcode must be 0 exactly when id is 0).
4. Build the row with `packet::request(id, s, opcode, flags, handle, offset, count, mode, p1s, p1l, p2s, p2l, ds, dl, stage)` and pass the rows concatenated (`cat_words`) as `requests`.
   Refs `(p1s,p1l)`, `(p2s,p2l)`, `(ds,dl)` point into the step's **literal-byte pool** (`bytes` argument of `step_packet`), `(-1,0)` absent. Put each literal string into the pool with
   `append_str`/`cat_bytes` and remember `len(pool_before)` as its start.
5. A slot has at most one outstanding request. At most 4 requests per packet and at most one per slot. Use `action = 0` (await).

To consume the completion that root passes (`event[2]` = slot, `event[1]` = id, `event[3]` = opcode, already matched): clear the slot record in the new ctl — `c[b+2] = 0; c[b+3] = 0;
c[4] = band(c[4], 15 - 2^s)` — **unless** the same slot gets a new request in the same packet (then step 2/3 overwrite id/opcode with the new fresh id). Root rejects a packet whose completed slot still carries
the old request id (a completion is consumed exactly once).

`band`, `bor`, `bxor` are builtins; `c[4] = band(c[4], bxor(2^s, 15))` also clears the bit. `packet.pruv` has no helper for this on purpose (keep it inline).

For an `await` after a completion with nothing new on that slot, the other slots must still be pending (`c[4] != 0`), otherwise return a `continue` or a `done`.

Empty event: `event[1] == 0 && event[2] == -1` — `crate::packet::event_empty(event)`; it appears exactly after `grok_begin` and after a **continue** packet.

## 5. Completion decoding helpers

`packet::event_shape(event, clen)` (root already ran it) guarantees: `len(event) >= 24`, `event[0]==1`, `event[4] in 0..1` (status), `event[5] in 0..4` (errno class), `event[7] in 0..8` (kind),
`event[13] >= -1`, `event[15] in 0..1`, `event[18] == clen`, refs `(event[19],event[20])` and `(event[21],event[22])` inside the completion, readdir rows `event[24+3i..]`: name ref
inside the completion and kind 0..8. Read `event[8]` size, `event[9..13]` dev/ino limbs, `event[14]` bytes read/written, `event[16]` row count.
Row count `event[16]` is at most `(len(event)-24)/3`.

## 6. Step packet helpers (`packet.pruv`, all `pub`)

| function | requires | ensures / notes |
|---|---|---|
| `ref_ok(s,n,total) -> bool` | none | true ⇒ `total>=0 && ((s==-1 && n==0) || (0<=s && s<=total && 0<=n && n<=total-s))` |
| `ctl_init(mode,phase) -> [i64] ! alloc` | `0<=mode<=5 && 0<=phase<=999` | `len==256 && result[3]==1` |
| `ctl_final(ctl) -> [i64] ! alloc` | `len(ctl)==256` | `len==256`; phase 999, pending 0, slot records cleared (used by `done_*`) |
| `slot_base(s)` | `0<=s<=3` | `== 128+32*s` |
| `patch_resize(bank,newlen) -> [i64] ! alloc` | `0<=bank<=7 && 0<=newlen` | `len==8` |
| `patch_copy(bank,dst_off,count,src,src_off) -> [i64] ! alloc` | `0<=bank<=7 && 0<=dst_off && 0<=count && 1<=src<=4 && 0<=src_off` | `len==8`; src 1 event words, 2 completion bytes, 3 literal words, 4 literal bytes; units = destination-bank elements; offsets relative to the bank just before that patch |
| `request(id,slot,opcode,flags,handle,offset,count,mode,p1s,p1l,p2s,p2l,ds,dl,stage) -> [i64] ! alloc` | `1<=id && 0<=slot<=3 && 1<=opcode<=13 && 0<=flags<=3 && -1<=handle && 0<=offset && 0<=count && 0<=mode && 0<=stage<=30` | `len==16` |
| `cat_words(a,b) -> [i64] ! alloc` | `len(a)<=50000000 && len(b)<=50000000` | `len==len(a)+len(b)` (join patch rows or request rows) |
| `cat_bytes(a,b) -> [u8] ! alloc` | same bounds | `len==sum` |
| `append_str(a:[u8], s:str) -> [u8] ! alloc` | `len(a)<=50000000 && len(s)<=50000000` | `len==len(a)+len(s)`; new bytes start at `len(a)` |
| `step_packet(ctl,patches,requests,words,bytes,res,action) -> [i64] ! alloc` | `len(ctl)==256 && len(patches)<=4096 && len(requests)<=64 && len(words)<=1e8 && len(bytes)<=1e8 && len(res)<=1e8` | `len>=268`. Runtime shape check (else a done packet with InternalInvariant stage 30): `ctl[0]==1`, `0<=ctl[2]<=999`, `len(patches)%8==0`, `len(requests)%16==0`, ≤512 patches, ≤4 requests, action 0..2, patch row ranges/literal bounds (src 3: `src_off<=len(words) && count<=len(words)-src_off`; src 4 likewise with `len(bytes)`), request row ranges and path/data refs inside `bytes`, `request[15]==0`; action 1 ⇒ no patches/requests and `res` is a result packet (`result_shape`); action 0/2 ⇒ `len(res)==0`; action 2 ⇒ no requests; action 0 ⇒ ≥1 request or `ctl[4]!=0` |
| `done_wrap(ctl,res) -> [i64] ! alloc` | `len(ctl)==256` | `len>=268`; `ctl_final` + action 1 with result `res` (any `[i64]`, checked) |
| `done_error(ctl,stage,cls,code,eno,native,raw:[u8],name:str,detail:str) -> [i64] ! alloc` | `len(ctl)==256` | `len>=268`; mode word from `ctl[1]` |
| `done_simple(ctl,stage,cls,code) -> [i64] ! alloc` | `len(ctl)==256` | text-free error; out-of-range enums give InternalInvariant |
| `done_internal(ctl,stage) -> [i64] ! alloc` | `len(ctl)==256` | class 6, code 2 (InternalInvariant) at `stage` |
| `meta_new(mode) -> [i64] ! alloc` | `0<=mode<=5` | `len==24`; schema 2, mode, every ref absent (words 4,6,8,18,20,22 = -1), rest 0 |
| `result(meta,rows,text) -> [i64] ! alloc` | `len(meta)==24 && len(rows)<=1e8 && len(text)<=1e8` | `len>=28`; call it as `crate::packet::result(..)`. Checks: `meta[0]==2`, mode 0..5, `meta[2],[11],[12]` 0..1, refs 4-5/6-7/8-9 inside `text`, **error scalars `meta[13..17]==0` and error refs (18..23) absent**, `meta[10]` row width ∈ {0,10,12} with `meta[3]*width == len(rows)` (width 0 ⇒ count 0 and no rows; mode ≥ 2 ⇒ width 0), list row (12): refs 0-1,2-3,4-5,6-7,8-9 inside text, word 10 ∈ 0..1, **word 11 == 0**; read row (10): refs 0-1..6-7, word 8 ≥ 0, word 9 ∈ 0..1. Any failure returns the InternalInvariant packet (stage 30, class 6, code 2) for `meta[1]`. |
| `error(stage,cls,code,eno,native,raw,name,detail) -> [i64] ! alloc` | none | `len>=28`, status 1; mode word 0. Enums out of range or strings > 10 MB → InternalInvariant packet. Text = `raw ++ name ++ detail`; an empty string is an absent ref. |
| `error_mode(mode, stage,cls,code,eno,native,raw,name,detail) -> [i64] ! alloc` | none | same with the mode word set |
| `simple_error(mode,stage,cls,code) -> [i64] ! alloc` | `0<=mode<=5 && 0<=stage<=30 && 0<=cls<=7 && 0<=code<=8` | `len==28` |
| `internal_packet(mode) -> [i64] ! alloc` | `0<=mode<=5` | `len==28` |
| `result_shape(res) -> bool` | `len(res)<=1e8` | length-consistent result packet |
| `event_empty(event)`, `event_shape(event, clen)` | `len(event)>=24` for `event_empty` | see §5 |

Result packet layout (absolute): `[status, 24, rowWords, textLen, meta0..meta23 (words 4–27), rows (from 28), text bytes]`; length `28 + rowWords + textLen`.
Meta (offset inside meta): 0 schema 2 · 1 mode · 2 omittedBefore · 3 row count · 4,5 title ref · 6,7 model ref · 8,9 copy path / small-json value ref · 10 row width ·
11 small-json present · 12 copy skipped · 13 stage · 14 class · 15 code · 16 errno class · 17 native errno · 18,19 raw error.code ref · 20,21 name ref · 22,23 diagnostic ref.
Refs are into the **result's** text area (not the banks).

Step packet layout (absolute): header 0 magic · 1 version 1 · 2 action · 3 ctl words 256 · 4 patch count · 5 request count · 6 literal words · 7 literal bytes · 8 result words · 9–11 0;
12–267 ctl; then `8*pc` patch words; `16*rc` request words; literal words; literal bytes (one word per byte); result packet. Length `268 + 8*pc + 16*rc + lw + lb + lres`.
Patch row: 0 kind (0 resize, 1 copy) · 1 bank · 2 dst offset (0 for resize) · 3 copy count or new length · 4 source · 5 source offset · 6,7 0.

### Error enums (DESIGN §2)

class: 1 EngineImportError · 2 AbortError (reserved) · 3 RpcResponseError · 4 raw OS · 5 raw URIError · 6 Core · 7 other raw Error.
code: 0 none · 1 InputTooLarge · 2 InternalInvariant · 3 not-found · 4 grok-exists · 5 grok-load-failed · 6 timeout · 7 read-failed · 8 engine-missing.
errno class (event word 5): 0 none · 1 ENOENT · 2 EACCES · 3 EEXIST · 4 other. stage 0..30 as in DESIGN §2: 0 input, 1 realHome, 2 chain, 3 readdir, 4 candidate-dir, 5 file-lstat,
6 containment, 7 open, 8 identity-fstat, 9 size-fstat, 10 read, 11 close, 12 parse, 13 summary, 14 sort, 15 active, 16 first-target-lstat, 17 entropy, 18 parent-mkdir, 19 temp-mkdir,
20 exclusive-create, 21 write, 22 second-target-lstat, 23 rename, 24 spawn, 25 initialize, 26 session/load, 27 temp-cleanup, 28 target-cleanup, 29 caller-cleanup, 30 result.
`error()` requires class ≥ 1 for a failure (class 0 is only for success packets). Use `class 4` (raw OS) with `eno`/`native` for an OS failure that propagates raw;
use `done_error(..., raw, name, detail)` with class 3 and `raw` = `util::json_value` of `error.code` for an RpcResponseError.

### Banks

0 directory `[i64]` · 1 candidate rows `[i64]` (13 words per candidate: admitted + 12 list-row words, raw enumeration index) · 2 strings `[u8]` ·
3..6 small-json bytes for slots 0..3 · 7 history window `[u8]`. Every bank length is at most 100000000 on entry (root checks; larger ⇒ done InputTooLarge: stage 0, class 6, code 1).
The host zeroes new space and keeps the prefix on resize; geometric growth is the module's job.

## 7. Request/opcode quick reference (DESIGN §1)

1 readdir(path1; flags 1 = withFileTypes) · 2 lstat · 3 realpath · 4 open-read(flags 1 = O_NOFOLLOW) · 5 fstat(handle) · 6 positioned read(handle, offset, count) · 7 close(handle) ·
8 mkdir(path1; flags 1 recursive; mode 448) · 9 rename(path1→path2) · 10 recursive force rm(path1; flags 3) · 11 exclusive create(path1; mode 384) · 12 positioned write(handle, offset, data ref) · 13 entropy(count 6).
Root itself only issues opcode 10 (slot 0, stage 29, flags 3, handle -1, offset/count/mode 0) for caller cleanup.

## 8. `util.pruv` (all `pub`, all `! alloc`)

| function | contract | behaviour |
|---|---|---|
| `to_bytes(s:str) -> [u8]` | `len(result)==len(s)` | |
| `span_bytes(t:[u8], s, l) -> [u8]` | `len(result)<=len(t)` | bytes `t[s..s+l]`; empty for an absent or out-of-range ref |
| `cfg_str(t:[u8], s, l) -> str` | `len(result)<=1000000` | the text at that ref; `""` for absent/invalid ref/invalid UTF-8/over 1000000 (uses `jstr::adapter_probe`, the only `[u8]→str` conversion that exists) |
| `str_at(d:JsonDocument, id) -> str` | `len(result)<=50331648` | decoded string value (lone surrogates are U+FFFD, P2); `""` for any non-string or bad id (callers check the kind first) |
| `raw(d, id) -> [u8]` | | raw source token (strings keep quotes); empty on a bad id |
| `elements(d, id) -> [i64]` | | ids of an array's elements in one tape pass; empty for non-array/corrupt/empty/over 1048576 |
| `escaped(s:str) -> [u8]` | `requires len(s)<=100000000`, `ensures len(result)<=6*len(s)` | JSON string content (no quotes): `"` `\` and C0 controls (`\u00xx`, lowercase hex) escaped |
| `json_value(d, id) -> [u8]` | | JSON fragment of the subtree: number/literal tokens raw, keys raw escaped spelling (with quotes) + `:`, string values re-escaped from the decoded value, no whitespace, `,` between members. **Empty = bad id, corrupt tape, depth over 129, or output over about 49 MB** (a real fragment is never empty); callers treat empty as InternalInvariant, not as malformed input. |

## 9. Decisions and spec points not fully met

- Verifier-sensitive spots (all verify now on both compilers): `util::json_value` (stack depth/forall invariant over `stk`, room invariants around `reserve`), `packet::step_args_ok`
  (`%` and `/` on lengths), `root::out_ok` (index arithmetic `268+8*pc+16*r`), `band`/`bor` use for the pending mask.
- **Assumption:** a key record's tape span (`tape[6j+1..6j+2]`) includes its quotes, like a string value's span (as `access::find` hands it to `jstr::units_equal`). `json_value` copies it verbatim.
- **A-added ctl word 30** (root-owned state) and the root's output checks (`out_ok`) go beyond DESIGN §1: they turn "single terminal", "empty event only after continue", and "a completion consumed once"
  into runtime checks. If ctl word 30 conflicts with a module need, say so; the alternative is dropping the word and the `st` check in `grok_step`.
- Result `mode` word: `error()` writes mode 0; use `error_mode`/`done_error` (mode from `ctl[1]`) so the packet carries the right mode. `result()` takes the mode from `meta[1]`.
- `done` packets are returned by modules with `packet::done_*`; root does **not** stamp ctl word 30 on the error packets it builds itself (the host stops at done anyway); it stamps every packet that passes `out_ok`.
- Row-width rule in `result()`: width must be 0, 10 or 12 independent of mode (except mode ≥ 2 ⇒ 0). So the summary `decode` packet (list row, mode 0, width 12, count ≤ 1) and the read packet (mode 1, width 10) both pass.
- `grok_load` and phase 600 use the **target home** (cfg 10,11) for GROK_HOME in mode 3 (DESIGN does not say which home); copy passes its target home itself.
- Timeout: `cfg[2]` must be 0..2147483647; `0` is passed through to `load_check`, which decides what 0 means (host default is 20000).
- `grok_caller_cleanup` and mode 5 treat an empty/absent path as InternalInvariant at stage 29 (nothing to remove is not a legitimate call).
- `rm` errors in phase 701 are never reported (the entry returns success with no rows), matching "cleanup errors never replace an original error".

## 10. Decisions made while testing against the reference

Implementation:
- **Read size.** The summary reader asks for 65536 bytes per read, the reference's `readFile` chunk. The trace keeps a
  digest only for a single-read run, so a different chunk size is a visible trace difference, not just a speed choice.
- **`rpc::send_or_fail`.** A failed write to the child's stdin marks the request failed at once (read-failed, spec P11)
  instead of waiting for the RPC timer.
- **Stage of a missing summary / path failure.** `summary.pruv` reports `absent_fin` and `path.pruv` failures at the
  stage the reference reports them (found by the fixture run).
- **Read title and model (audit I1).** `read::p_finish` uses the title and model refs from `summary::summary_from` as
  they are. Summary already applied `one_line(title, 120)` and `trim(model)`; a second `one_line` dropped the trailing
  space that the 120-unit slice can leave (119 `a` + `" b"`), which the reference keeps.
- **Late server requests (audit I3).** After the response that settles the last request (ok, error object, or bad
  capabilities), `rpc::load_check` still drains the rest of that chunk and answers server requests in it with -32601,
  as the reference's data handler does before the child is closed. A failed write there is ignored (the outcome is
  already settled, and the reference ignores stdin errors).
- **Framing memory (audit P1).** `framing::scan` used to preallocate `3 * (n/2 + 2) + 1` words for any window and then
  copy the events into the result (about 192 MiB for an 8 MiB window of `"0\n"`). It now counts the events first and
  fills one result of exactly `1 + 3 * count` words in place (about 96 MiB in that case; the result format is three
  words per event, so it can't be smaller without changing the caller).
- **Read model length (audit I2).** The 65536-byte summary cap is a size check before the read; a summary that grows
  after the check is read to end of file (up to the 16 MiB growth cap), and the reference keeps its model whatever
  the length. `read::p_finish` takes the model with `util::long_str` (up to 20000000 bytes, the summary guard)
  instead of `cfg_str` (1000000), which turned a longer model into an empty one.
- **Not changed: directory count bound (audit I4).** List rejects a directory packet of more than 5000000 entries with
  InputTooLarge. This bounds a core packet (3 words per entry, spec P5 "bounded core packets"); it is not a cap on
  admitted sessions.

Harness and oracle (`common.py`, `oracle.py`, `q.py`, `q_server.py`):
- **Logical paths.** `$SRC` / `$DST` in schedule rules and actions mean the source / target **home**, not the case root
  (both the TS hooks and `host.rs` map them that way), so rule paths are written home-relative. Relative symlink
  targets pass through unchanged; `snapshot` maps absolute symlink targets back to logical paths so the two sides'
  trees compare equal.
- **`_transfers`.** Inside one copy transfer the order of source-handle and destination-handle primitives is stream
  timing in the reference, so both traces are put in a canonical order (create, source handle, destination handle).
  After a failed create, source reads that raced it are dropped from the reference trace only (the reference's
  read stream may start before the create fails); the implementation's trace keeps them, so an implementation
  that reads after a failed create is still a difference (audit O6: an earlier filter dropped them on both sides).
- **Flipped copy target.** When an ACP policy (P6, P7, P11, P12) flips a copy's outcome, the failed side removes its
  target and the successful side keeps it, so the dst trees differ. That tree difference is put under the same policy
  only when it is exactly one target subtree, present on the ok side and absent on the failed side, with the rest of
  the tree equal (Q seed 20261116 had 14 such P6 cases: an `id` token `1.0` that TS accepts and Pruv does not).
- **Error view.** Raw OS errors compare stage/class/code plus errno class, native number and the raw code string;
  RPC errors compare the raw `error.code` token and whether it was present. Diagnostic text is never compared.
- **P1 baseline.** P1 (JSON Pruv's parser rejects: invalid UTF-8 or depth over 128) is judged against a TS rerun on
  the case with only those lines / files replaced by same-length filler (`oracle.p1_case`). A case that triggers both
  P1 and P4 also gets a rerun with both edits (`'P1+P4'`, `oracle.p14_case`): the P1 lines are chosen on the original
  bytes first, then the P4 edit runs on the rest (audit O3). File P1 is judged only by these reruns; the shape rules
  that accepted any subsequence of rows when P1 was triggered anywhere are gone (audit O1).
- **P4 baseline value.** An unsupported date token becomes the string `"invalid"`, not `null`: TS reads no date from
  it, and `timestamp ?? created_at` doesn't fall back to the other key (audit O2).
- **Symlinked home.** `snapshot` of a home that is itself a symlink records the target's tree (audit O4).
- **ACP P1 rule.** A rejected ACP reply line is never delivered, so where TS fails on another ground at initialize or
  session/load, Pruv waits out that same request (timeout at the same stage); `acp_class_policy` explains it as P1,
  only when the trigger is a rejected ACP reply (`'P1-ACP'`). ACP policies never exempt argv or dst tree differences
  (audit O7); they explain the result, error and request stream, and P6/P7/P11/P12 the trace after spawn.
- **Fake `load_ok`.** The fake ACP server decides whether `session/load` was allowed by parsing the initialize reply it
  actually wrote, the way the reference data handler does (audit O5): lines with a string `method` are skipped, the
  first line whose id is a number equal to the request id settles it, an object `error` rejects (`error: null` does
  not), and `agentCapabilities.loadSession === true` is required. Every initialize reply is judged this way on all
  the bytes written for it, including `together` items and the `also_correct` line, so an omitted result or a
  correct reply after an unmatched raw / idtoken line is judged like the client sees it (confirmation audit).
- **RPC timers.** Fixtures and Q cases that wait out a stalled request use `rpcMs` 3000 (was 300–500, then 1500;
  `verify-timeout-removes-target` was the last one at 400, found by the confirmation audit). Under four
  pairs × six jobs on a loaded machine the shorter timers let a run time out before the fake ACP had even started,
  on either side.
- **Q generator.** The race action that puts a file at the target is `replace-file` (the reference harness kind); the
  earlier `file` kind was a generator bug that the Rust host rejected.
- **Host `replace-file`.** `host.rs` now creates the file with `mode` (default 0o644) and then chmods it when a mode is
  given, like the reference hook's `writeFileSync` + `chmodSync`; before, the umask left a different mode on the
  raced target.
- **Host schedule (audit).** `host.rs` follows `reference-primitives.mjs` more closely: an explicit mode 0 is kept
  (`Option<u32>` for `replace-file` and `mkdir`); `append` creates a missing file; occurrence counts are per
  (op, path); the first rule that injects an error ends that kind for the call, and a before-call error skips the
  after rules; an after-hold applies only when the call succeeded or its after error was injected; `releaseWhen`
  supports `event` (issue, barrier, complete, counted per op and path) and `held`; when the scripted entropy runs
  out it falls back to OS randomness. Known approximations: `held` counts buffered completions, and only the last
  hold on a call applies. The after-result barrier is noted when the completion arrives, before its hold is checked,
  so a hold whose `releaseWhen` is that call's own barrier event releases at once as in the reference (before, the
  barrier was noted at delivery, after the hold, and such a hold never released; confirmation audit).

