# L3-6 Grok import: implementation design

This is the implementation design for `docs/GROK_IMPORT_SPEC.md` (the spec wins when they disagree).
It was written from a source survey (2026-11-16) and translated into English; section 9 closes
the one item the survey left open (the exact Node sort schedule).

Summary: a **delta-step protocol** with a small control array. The host keeps the large
buffers (directory listing, candidate rows, strings, small JSON, history window) as opaque
banks and lends them read-only to Pruv. Pruv makes every decision: paths, admission, errors,
sorting, copying, cleanup. The listing keeps four slots and advances the matching slot on
each completion. After a copy, the Pruv code that consumed the rename completion calls the
ACP load check (which uses `proc_spawn_env`) exactly once.

## 1. Host/Pruv step protocol

### Current ABI and why this shape

Codex public functions take `str`, `i64` and return `[i64]` packets; the Rust harness calls
`codex_list`/`codex_read` of the generated module and only renders the returned packet.

- `[i64]`, `[u8]` arguments are borrowed, read-only arrays.
- A returned `[i64]` is owned; the LLVM binding takes ownership of the returned memory.
- `inout` on public entries and structs with buffer fields are not allowed.
- Internal functions may use `inout`, but cannot replace or resize a whole array.
- No `[str]`: use a byte pool plus `(start,len)`.

Sources: `bench/l3/codex/harness.rs:1`, `root.pruv::codex_list/codex_read`,
`SPEC.md:362–364,468–474`, `pruvc-rs/crates/pruv-core/src/bindings.rs:442–444,507–513`.

So, without bypassing the public ABI or changing a compiler:

```pruv
pub fn grok_begin(cfg: [i64], cfg_text: [u8]) -> [i64] ! alloc

pub fn grok_step(
  cfg: [i64], cfg_text: [u8],
  ctl: [i64],
  directory: [i64], rows: [i64], text: [u8],
  small0: [u8], small1: [u8], small2: [u8], small3: [u8],
  history: [u8],
  event: [i64], completion: [u8]
) -> [i64] ! async, net, proc, alloc
```

The proc/async effects of `grok_step` are only used by the post-rename verify branch of copy.
No foreign function that runs FS requests is added.

### Config packet

`cfg_text` is a UTF-8 byte pool; all refs point into it.

| cfg word | meaning |
|---|---|
| 0 | schema = 1 |
| 1 | mode: 0 list, 1 read, 2 copy, 3 load, 4 small-json, 5 caller-cleanup |
| 2 | RPC timeout ms; default 20000 |
| 3 | reserved, 0 |
| 4, 5 | source home start, len |
| 6, 7 | folder start, len |
| 8, 9 | thread start, len |
| 10, 11 | target home start, len |
| 12, 13 | binary start, len |
| 14, 15 | source engine start, len |
| 16, 17 | direct small-json path start, len |
| 18, 19 | direct small-json homeReal start, len |
| 20, 21 | caller-cleanup path start, len |
| 22, 23 | reserved, 0 |

A missing value is `(-1,0)`; a present empty string is `(valid offset,0)`. The host does not
normalize path inputs.

### Opaque banks

| bank | type | content |
|---|---|---|
| 0 | `[i64]` | readdir packet: raw rows plus name bytes |
| 1 | `[i64]` | per-candidate result rows; replaced by the sorted rows at the end |
| 2 | `[u8]` | strings/serialized data for paths, metadata, pending error packets |
| 3–6 | `[u8]` | small-json bytes for slots 0–3 |
| 7 | `[u8]` | history window, at most 8388608 bytes |

The host does not interpret banks. It only performs the resize/copy patches Pruv returns. New
space is zeroed, the existing prefix is kept, and completion data stays alive until all patches
of that step are applied.

### Control state

`ctl` is exactly 256 words; each step does only `copy(ctl)`.

| ctl word | meaning |
|---|---|
| 0 | schema = 1 |
| 1 | mode |
| 2 | global phase (stage numbers below are the base phases) |
| 3 | next request ID, starts at 1 |
| 4 | pending slot bitmask, 0–15 |
| 5 | next readdir row index to examine |
| 6 | readdir row count |
| 7 | finished candidate count |
| 8 | admitted candidate count |
| 9 | copy skipped |
| 10 | history window start |
| 11 | history window end, exclusive |
| 12 | next history positioned-read offset |
| 13 | verify called, 0/1 |
| 14 | window omittedBefore, 0/1 |
| 15 | global follow-up phase |
| 16, 17 | homeReal ref (bank 2) |
| 18, 19 | temp path ref |
| 20, 21 | read title ref |
| 22, 23 | read model ref |
| 24, 25 | returned copy path ref |
| 26, 27 | pending error serialized ref |
| 28 | small-json purpose: direct/summary/active |
| 29 | close policy: propagate/swallow/preserve-existing-error |
| 30–127 | reserved, 0 |
| `128 + 32*s + 0` | slot phase |
| `+1` | current readdir row index, or -1 |
| `+2` | outstanding request ID, or 0 |
| `+3` | outstanding opcode |
| `+4` | source handle ID, or -1 |
| `+5` | destination handle ID, or -1 |
| `+6` | size at lstat |
| `+7,+8` | expected dev high32, low32 |
| `+9,+10` | expected ino high32, low32 |
| `+11` | next read/write offset |
| `+12` | bytes of the current write chunk not yet written |
| `+13` | small-json purpose |
| `+14` | phase to return to after a shared helper |
| `+15` | result present |
| `+16,+17` | title ref |
| `+18,+19` | model ref |
| `+20,+21` | updatedAt ref |
| `+22,+23` | messages decimal ref |
| `+24,+25` | current file name ref |
| `+26` | accumulated small-json byte count |
| `+27` | current chunk write cursor |
| `+28` | follow-up close/cleanup phase |
| `+29` | EOF seen |
| `+30,+31` | reserved, 0 |

Slot `s` is 0–3. Serial work before/after the listing, and read/copy/small-json, use slot 0.
Candidates enter slots in raw enumeration order, and a candidate's row in bank 1 is its **raw
entry index**, not its slot number.

### Step return packet

All elements are `i64`; elements of byte areas must be 0–255.

| word | meaning |
|---|---|
| 0 | magic = `0x47525331` |
| 1 | protocol version = 1 |
| 2 | action: 0 await-completion, 1 done, 2 continue-after-patches |
| 3 | ctl words = 256 |
| 4 | patch count |
| 5 | request count, 0–4 |
| 6 | literal-word pool length |
| 7 | literal-byte pool length |
| 8 | final-result words |
| 9–11 | reserved, 0 |
| 12–267 | new ctl |
| then | patch rows, 8 words each |
| then | request rows, 16 words each |
| then | literal-word pool |
| then | literal-byte pool, one word per byte |
| last | final-result packet (§2) |

Total length is `268 + 8*patches + 16*requests + literalWords + literalBytes + resultWords`.
Only `done` carries a final result. `continue` re-enters after internal storage steps; the host
then passes the empty event. Each continue branch must advance the phase (no unbounded busy loop).

### Storage patch, 8 words

| word | meaning |
|---|---|
| 0 | 0 resize, 1 copy |
| 1 | destination bank ID |
| 2 | destination offset; 0 for resize |
| 3 | copy element count, or length after resize |
| 4 | source: 0 none, 1 event words, 2 completion bytes, 3 literal words, 4 literal bytes |
| 5 | source offset |
| 6, 7 | reserved, 0 |

Units are destination-bank elements. Word→byte copies check every element is 0–255. Each
offset is relative to the bank **just before that patch runs**. All patches are applied before
any primitive is issued.

### Primitive request, 16 words

| word | meaning |
|---|---|
| 0 | request ID |
| 1 | slot, 0–3 |
| 2 | opcode |
| 3 | flags |
| 4 | handle ID or -1 |
| 5 | positioned offset or 0 |
| 6 | requested byte count or 0 |
| 7 | create mode or 0 |
| 8, 9 | path1 ref |
| 10, 11 | path2 ref |
| 12, 13 | write data ref |
| 14 | stage |
| 15 | reserved, 0 |

Refs point into the step's literal-byte pool.

| opcode | primitive | arguments |
|---|---|---|
| 1 | readdir | path1; flags 1=withFileTypes, 0=names |
| 2 | lstat | path1 |
| 3 | realpath | path1 |
| 4 | open-read | path1; flags 1=O_NOFOLLOW where available |
| 5 | fstat | handle |
| 6 | positioned read | handle, offset, count |
| 7 | close | handle |
| 8 | mkdir | path1; flags 1=recursive; mode=448 (0700) |
| 9 | rename | path1→path2; plain OS rename |
| 10 | recursive force remove | path1; flags=3 |
| 11 | exclusive create | path1; mode=384 (0600) |
| 12 | positioned write | handle, offset, data |
| 13 | entropy | count=6 |

open and fstat are separate so a barrier can sit between them, and so Pruv decides whether to
close after an identity failure.

### Completion packet

`event` is a 24-word header plus readdir rows; `completion:[u8]` holds the raw bytes.

| event word | meaning |
|---|---|
| 0 | version = 1 |
| 1 | request ID |
| 2 | slot |
| 3 | opcode |
| 4 | status: 0 success, 1 OS error |
| 5 | errno class: 0 none, 1 ENOENT, 2 EACCES, 3 EEXIST, 4 other |
| 6 | native errno, or 0 |
| 7 | kind: 0 unknown, 1 file, 2 directory, 3 symlink, 4 FIFO, 5 socket, 6 char, 7 block, 8 other |
| 8 | size |
| 9, 10 | dev high32, low32 |
| 11, 12 | ino high32, low32 |
| 13 | returned handle ID or -1 |
| 14 | bytes read/written |
| 15 | EOF, 0/1 |
| 16 | readdir row count |
| 17 | row width; 3 for readdir, else 0 |
| 18 | completion byte length |
| 19, 20 | raw OS code string ref |
| 21, 22 | diagnostic message ref |
| 23 | reserved, 0 |
| `24+3*i` | name start |
| `25+3*i` | name len |
| `26+3*i` | entry kind |

All refs point into the completion bytes. A realpath success uses the whole completion as the
path; read and entropy successes use the whole completion as data. Errors carry only code and
message. The empty event is version=1, request=0, slot=-1, the rest 0; it is only allowed for
init/continue. Pruv checks `(request ID, slot, opcode)` against its outstanding record; a
duplicate or wrong completion is `InternalInvariant`, never an OS failure.

### Per-mode flow

- **list:** realHome → chain → readdir → assign four slots. Each slot: candidate plainDir →
  summary lstat/realpath/open/fstat/read-to-EOF/close → summary decision. A finished slot takes
  the next unprocessed name at once. After all candidates: sort → read the active file → 50 rows.
- **read:** thread check → realHome/chain/summary → history lstat/realpath/open → identity fstat
  → **second fstat for the window size** → 256 KiB reads → close → parse the stored window once.
- **small-json:** lstat/containment/size≤65536 → open/fstat → read to EOF → close → parse once.
  Only the active purpose accepts an array root. Growing past 16777216 bytes closes, then P5
  `InputTooLarge`.
- **copy:** input checks and path building → realHome/chain → first target lstat → entropy →
  parent/temp mkdir → source readdir → check and transfer each allowed file → second target lstat
  → rename → Pruv `rpc::load_check` → success, or target cleanup.
- **load:** a public wrapper that skips the FS steps and calls the same `rpc::load_check`.
- **caller cleanup:** a test caller runs read→copy→publish in order. If publish fails, a separate
  caller-cleanup entry issues the rm request for the returned copy path. The `removeGrokCopy`
  undo policy is not implemented.

Copy does not use the size as a transfer cap; it reads 64 KiB pieces until EOF. A short write
re-requests the remaining suffix. A successful 0-byte write is a no-progress error. An error
after a partial write keeps the original error, closes open descriptors, then cleans up the temp.

### Cost

An 8 MiB history is usually 32 reads. Each completion is stored once into a bank and parsed once
at the end; no step re-serializes 8 MiB or replays earlier lines. The listing stores the directory
once and each candidate writes its own row. Every candidate beyond 50 is examined; no cap is applied
before the final sort. The fixed ctl copy is about 2 KiB per step. Bank growth uses geometric
capacities chosen by Pruv, so total copying stays linear. No separate session-count or copy-byte cap.

## 2. Result and error packets

Keeps the Codex `[status,metaWords,rowWords,byteLength,...]` shape; schema 2 marks Grok.

| absolute word | meaning |
|---|---|
| 0 | status: 0 success, 1 error |
| 1 | metadata words = 24 |
| 2 | row words |
| 3 | text byte length |
| 4–27 | metadata below |
| from 28 | rows |
| after rows | text bytes, one word per byte |

| metadata word | meaning |
|---|---|
| 0 | schema = 2 |
| 1 | mode |
| 2 | omittedBefore, 0/1 |
| 3 | row count |
| 4, 5 | title ref |
| 6, 7 | model ref |
| 8, 9 | copy path, or small-json value ref |
| 10 | row width |
| 11 | small-json present, 0/1 |
| 12 | copy skipped |
| 13 | error stage |
| 14 | error class |
| 15 | EngineImport/Core code enum |
| 16 | errno class |
| 17 | native errno |
| 18, 19 | raw error.code JSON ref |
| 20, 21 | error name ref |
| 22, 23 | diagnostic text ref |

All refs point into the final text area; absent is `(-1,0)`. Success packets have all error
scalars 0 and error refs absent. There is no `lastPlan` field.

### List row: 12 words

| word | meaning |
|---|---|
| 0, 1 | thread ref |
| 2, 3 | title ref; an empty title is present |
| 4, 5 | updatedAt ref |
| 6, 7 | path ref |
| 8, 9 | messages decimal token ref |
| 10 | maybeOpen: 0 absent, 1 true |
| 11 | reserved, 0 |

The internal candidate bank uses 13-word rows: word 0 is admitted 0/1, then the 12 words above.
Bank rows follow the raw enumeration index. Non-admitted rows are zero after word 0 and are never
emitted.

### Read row: the existing 10 words

| word | meaning |
|---|---|
| 0, 1 | prompt ref |
| 2, 3 | reply ref |
| 4, 5 | createdAt ref |
| 6, 7 | completedAt ref |
| 8 | tools |
| 9 | clipped: 0 absent, 1 true |

copy/load/small-json/caller-cleanup have no rows. A load success emits only the `{ok:true}`
envelope. An absent small-json is `{ok:true,present:false}`, kept apart from JSON `null`.

### Error classes

| class | meaning |
|---|---|
| 0 | none |
| 1 | EngineImportError |
| 2 | AbortError — representation reserved; D3 paths are not implemented and not in Q |
| 3 | RpcResponseError |
| 4 | raw OS error |
| 5 | raw URIError |
| 6 | Core error |
| 7 | other raw Error |

Code enum: `0 none, 1 InputTooLarge, 2 InternalInvariant, 3 not-found, 4 grok-exists,
5 grok-load-failed, 6 timeout, 7 read-failed, 8 engine-missing`.

`RpcResponseError.code` may be a number, null, string, object or array, so it is kept as a JSON
value ref; missing is an absent ref. Diagnostic text is not compared; class/name/code/stage are.

| stage | meaning |
|---|---|
| 0–7 | input, realHome, chain, readdir, candidate-dir, file-lstat, containment, open |
| 8–15 | identity-fstat, size-fstat, read, close, parse, summary, sort, active |
| 16–23 | first-target-lstat, entropy, parent-mkdir, temp-mkdir, exclusive-create, write, second-target-lstat, rename |
| 24–30 | spawn, initialize, session/load, temp-cleanup, target-cleanup, caller-cleanup, result |

Cleanup errors go into the trace but never replace a pending original error.

Small-json results and raw RPC values are serialized by Pruv as JSON fragments: numbers keep the
raw token, keys keep an escaped spelling that preserves UTF-16 identity, string values are the
P2-decoded value. If the host converted to binary64 first the P10 evidence would be lost, so the
fragment keeps number tokens.

## 3. Modules and verified contracts

`StepArgs` below is **shorthand for the full `grok_step` argument list of §1**, not a language
type. No buffer struct enters the public ABI.

| file | public functions | contracts to verify; packages used |
|---|---|---|
| `root.pruv` | `grok_begin(cfg:[i64], cfg_text:[u8])->[i64]`; `grok_step(StepArgs)->[i64]`; `grok_load(binary:str,home:str,folder:str,thread:str,timeout_ms:i64)->[i64]`; `grok_caller_cleanup(path:str)->[i64]` | config/packet shape checks, mode dispatch, completion match, a single terminal. load/step are `async,net,proc,alloc`. |
| `packet.pruv` | `ref_ok(s:i64,n:i64,total:i64)->bool`; `step_packet(ctl:[i64],patches:[i64],requests:[i64],words:[i64],bytes:[u8],result:[i64],action:i64)->[i64]`; `result(meta:[i64],rows:[i64],text:[u8])->[i64]`; `error(stage:i64,cls:i64,code:i64,eno:i64,native:i64,raw:[u8],name:str,detail:str)->[i64]` | no overflow in length math, every ref and row width, byte ranges, enum ranges. `bytes`. |
| `util.pruv` | `to_bytes(s:str)->[u8]`; `str_at(d:JsonDocument,id:i64)->str`; `raw(d:JsonDocument,id:i64)->[u8]`; `elements(d:JsonDocument,id:i64)->[i64]`; `json_value(d:JsonDocument,id:i64)->[u8]`; `escaped(s:str)->[u8]` | each tape subtree walked once, InternalInvariant never hidden as malformed, scalar boundaries. `json,access,jstr,bytes,text`. |
| `path.pruv` | `encode_folder(folder:str)->[u8]`; `join(a:str,b:str)->str`; `folder_same(cwd:JsonDocument,id:i64,folder:str)->bool`; `inside(home:str,path:str)->bool`; `valid(thread:str)->bool`; `advance(StepArgs)->[i64]` | uppercase-hex percent encoding, POSIX join dot/duplicate-separator handling, separator-aware containment, all-slash folderKey, lone-surrogate cwd mismatch. `bytes,text,names,json,access,jstr`. |
| `summary.pruv` | `advance(StepArgs)->[i64]`; `decode(data:[u8],purpose:i64,folder:str,thread:str)->[i64]`; `active(data:[u8],threads:[i64],text:[u8])->[i64]` | size pre-check vs EOF read, close on every exit, root/ID/cwd checks, P9 token check, active fallback priority. `json,parse,access,jstr,text,time,bytes`. |
| `list.pruv` | `advance(StepArgs)->[i64]`; `finish(rows:[i64],text:[u8],active:[u8])->[i64]`; helper `sort_indices(rows:[i64],text:[u8])->[i64]` | at most four outstanding, one per slot, next index monotone, results keep input index, 50 rows after sorting, exact Node comparator schedule (§9). `text,time,names,bytes`. |
| `framing.pruv` | `scan(window:[u8],omitted:bool)->[i64]`; `dropped_user(head:[u8])->bool` | LF and 1 MiB cap, CR kept, first-prefix discard vs drop, final fragment, replacement-decoded 8192-byte raw watcher. `bytes,text,jstr`. |
| `entry.pruv` | `feed(d:JsonDocument,inout st:[i64],inout sl:[i64],inout buf:[u8])->i64`; `content(d:JsonDocument,id:i64)->str`; `add_tools(n:i64,inout st:[i64],inout sl:[i64])->()` | synthetic nullish check, removes exactly one query line, timestamp nullish fallback, tool N times after the answer; passes collector pre/post invariants through. `json,access,jstr,text,time,turns,bytes`. |
| `read.pruv` | `advance(StepArgs)->[i64]`; `window(window:[u8],omitted:bool,title:str,model:str)->[i64]` | window ≤ 8 MiB, framing event order, collector growth done by the owner, final suffix and omitted flag. `json,lines,turns,bytes,text`. |
| `copy.pruv` | `advance(StepArgs)->[i64]`; `allowed(name:str)->bool`; `temp_name(thread:str,entropy:[u8])->str` | exact 13-name allow-list, skipped counted once, offset progress, first/second lstat asymmetry, a single verify after rename, error-preserving cleanup. `bytes,text,names`. |
| `rpc.pruv` | `load_check(binary:str,home:str,folder:str,thread:str,timeout_ms:i64)->[i64]`; `kind_of(d:JsonDocument,id:i64)->i64`; `not_handled(d:JsonDocument)->[u8]`; `parse_line(line:[u8],n:i64)->JsonParse` | Process/Loop must-consume, slot and transport bounds, exact ids 1/2, 1000000 pump bound, chunk drain, capability strictly true, error class kept. `json,access,jstr,lines,tail,bytes,text,args,transport`. |

`decode` returns the §2 schema too: summary puts title/model in metadata and the list info in a
single list row. `active` returns a 0/1 array, one per input candidate row.

`framing::scan` returns `[count, kind,lo,hi, ...]`, 3 words per event: `kind=0` is a normal
LF-terminated line span, `kind=1` a drop marker whose `lo,hi` is the (≤ 8192-byte) prefix span.
The first window prefix makes no event. `read::window` consumes the events once, in order.

Because `turns::finish` requires `base≤3500000`, **build the turn text first with base=0**, then
append title/model. A long model from a grown summary must not get a new 200-unit limit.

Loops decrease by measures taken from the input (window bytes, tape subtree end, entry count,
tool array length). Copy is not wrapped in a fixed-count loop; one step consumes one completion,
so large files need no step limit.

`pruv.json`:

```json
{
  "name": "l3_grok_root",
  "version": "0.1.0",
  "root": "root.pruv",
  "dependencies": {
    "json": {"path": "../pkgs/json", "version": "0.1"},
    "parse": {"path": "../pkgs/parse", "version": "0.1"},
    "access": {"path": "../pkgs/access", "version": "0.1"},
    "jstr": {"path": "../pkgs/jstr", "version": "0.1"},
    "lines": {"path": "../pkgs/lines", "version": "0.1"},
    "tail": {"path": "../pkgs/tail", "version": "0.1"},
    "bytes": {"path": "../pkgs/bytes", "version": "0.1"},
    "text": {"path": "../pkgs/text", "version": "0.1"},
    "time": {"path": "../pkgs/time", "version": "0.1"},
    "names": {"path": "../pkgs/names", "version": "0.1"},
    "turns": {"path": "../pkgs/turns", "version": "0.1"},
    "args": {"path": "../pkgs/args", "version": "0.1"},
    "transport": {"path": "../pkgs/transport", "version": "0.1"}
  }
}
```

`lex` stays a transitive dependency owned by the parser; root does not use it directly.
The legacy `bench/l3/lib` is not used.

## 4. `host.rs` and the primitive schedule

The host only:

1. reads JSONL config and fixture schedules;
2. calls the generated public functions and applies bank patches;
3. dispatches the 13 primitives of §1;
4. keeps the descriptor ID → real descriptor table;
5. delivers the **first arrived** completion from the completion channel;
6. checks result packet shape/ranges and renders JSON;
7. runs test-only traces and barriers.

readdir returns every name and kind. The host does no name/thread/allow-list filtering, JSON
parsing, containment comparison, identity decision or sorting. FileType unknown is kept.

dev/ino travel losslessly as two 32-bit limbs. Real inode numbers of separate fixture trees are
mapped to logical identities when comparing, but the importer gets the real values. Node's
rounding of dev/ino beyond the JS-safe range needs its own check; exact-u64 comparison is not
claimed to equal TS.

### Schedule format

```json
{
  "readdir": [
    {"path": "$SRC/sessions/%2Fwork", "names": ["A", "B", "C", "D", "E"]}
  ],
  "entropyHex": ["000102a0feff"],
  "rules": [
    {
      "match": {"op": "lstat", "path": "$DST/sessions/%2Fwork/A", "occurrence": 2, "barrier": "before-call"},
      "actions": [{"kind": "mkdir", "path": "$DST/sessions/%2Fwork/A", "mode": 448}]
    },
    {
      "match": {"op": "rename", "path": "$DST/sessions/%2Fwork/A.lair-import-000102a0feff", "occurrence": 1, "barrier": "before-call"},
      "error": {"code": "EACCES"}
    }
  ]
}
```

Barriers are `before-call` and `after-result-before-delivery`. Race actions are limited to file
replacement, symlink replacement, mkdir, unlink, append and truncate; file content comes from
fixture blob refs. Delays use `hold:"A"` with a `releaseWhen` condition, never wall-clock sleeps.

The concurrency fixture holds A's first request until E starts: when one of B–D finishes, E must
enter that slot at once. A separate fixture makes the first four requests all outstanding, then
releases them.

A readdir schedule must match the real returned name set exactly; no names are silently dropped or
added. Entropy replay exists only in test mode; production uses OS entropy.

The trace records `issue/complete`, op, logical path, request ID, slot, handle, offsets, byte
counts, mode, errno, kind, identity and a data digest. Raw data goes into a separate artifact when
needed.

## 5. TypeScript reference and trace comparison

The pinned `reference/` files are not modified. New files: `reference.mjs`,
`reference-hooks.mjs`, `reference-primitives.mjs`.

The existing hook points at Lair's TypeScript install outside the repository. The new hook must not
run that external dependency unconditionally; it takes an explicitly configured transpiler path.
Grok must not stub opencode features it does use; `folderKey` runs the pinned implementation.

The new hook only redirects the `node:fs`, `node:fs/promises`, `node:crypto` imports of pinned
`grok.ts`/`tail.ts`, and the `node:child_process` import of `stubs.ts` needed to observe spawn. The
shim's own builtin imports are not redirected.

### FS shim boundary

- `lstat/realpath/readdir/mkdir/rename/rm/open` delegate to the real Node functions with barriers
  and traces attached.
- The returned FileHandle facade observes `stat/read/close`.
- `readFile('utf8')` concatenates primitive reads to EOF and decodes the Node Buffer way.
- `createReadStream` is a Node stream adapter with a numeric fd and explicit read/close callbacks,
  so injections reach the real reads.
- `createWriteStream` likewise observes wx/mode and open/write/writev/close callbacks.
- Pinned `pipeline` and the importer's catch run unchanged.
- Stream success/error/auto-close behaviour is checked against native runs on fixed fixtures; do not
  assume a plain Proxy also intercepts native internal I/O.

### Compared exactly vs normalized

**Always identical:**

- per-path admission primitive order and success/failure;
- the positions of the first and second target lstat;
- no content read before the identity check;
- wx creation, mode, rename, cleanup targets;
- entropy call count, 6 bytes, temp location/format;
- ACP request order and content, verify call count;
- results and final file bytes/kinds/modes.

**Normalized (representation only):**

- fixture root prefix;
- real fd/PID and request numbers → logical operation/handle mapping;
- dev/ino numbers of separately built trees → same/different relations;
- native stream read/write splitting → contiguous offset ranges and total bytes per handle;
- completion timing of independent listing slots → per-candidate order plus concurrency conditions.

Ranges are never merged across an error or a race barrier. History requests check the 256 KiB window
selection separately. Normalization never erases the presence, order or target of cleanup actions.

Per case:

```js
const before = IMPORT_LIMITS.rpcMs;
try {
  IMPORT_LIMITS.rpcMs = caseSpec.rpcMs ?? 20000;
  // deps.env = process.env
  // run pinned export
} finally {
  IMPORT_LIMITS.rpcMs = before;
}
```

Cases run serially. Copying the Codex approach of only setting `options.timeoutMs` does not apply
to Grok.

The copy runner's verify calls the real `grokLoadCheck`; fake scripts produce RPC/timeout failures.
Plain errors from the callback itself are a separate case with an explicit test fault added to the
real load callback, and never count as a successful verify.

Check Node 26.4.0 and record the V8 version. The sort runs the pinned native sort unchanged. A
separate `sortProbe` may record comparator call pairs and result indices, but that trace must never
be fed to Pruv as input.

## 6. Test harness

### Fixture format

```json
{
  "name": "copy-second-exists",
  "mode": "copy",
  "umask": 18,
  "source": {"engine": "grok", "home": "$SRC"},
  "folder": "/work",
  "thread": "A",
  "targetHome": "$DST",
  "tree": [
    {"path": "$SRC", "kind": "dir", "mode": 493},
    {"path": "$SRC/sessions/%2Fwork/A", "kind": "dir", "mode": 448},
    {"path": "$SRC/sessions/%2Fwork/A/summary.json", "kind": "file",
     "bytes": {"utf8": "{\"info\":{\"id\":\"A\",\"cwd\":\"/work\"}}"}, "mode": 420}
  ],
  "schedule": {},
  "acp": {
    "initialize": {"result": {"agentCapabilities": {"loadSession": true}}},
    "session/load": {"result": null}
  },
  "expected": {"class": "EngineImportError", "code": "grok-exists"}
}
```

Tree recipes support dir/file/symlink/hardlink/FIFO. Bytes are `utf8`, `hex`, `repeat`, `concat`
or a blob ref. `1e400`, `-0` and invalid UTF-8 are kept as raw file text, never round-tripped
through Python JSON values first.

Each side builds its own tree from the same recipe. The whole source and target are snapshotted
before and after a run, comparing bytes/kind/mode and symlink targets. Setup creation and chmod end
before the baseline snapshot; nothing outside the schedule touches the tree while the importer runs.

### Fake ACP

The Grok argv is exactly `agent --no-leader stdio`; unlike Codex, no scenario/log paths are
appended. Each case's fake executable finds its sibling scenario/log itself.

`q_server.py` checks and records:

- argv, cwd, the real `GROK_HOME`, inherited parent env;
- initialize with ID=1 and the exact capabilities/clientInfo;
- `jsonrpc:"2.0"` and LF;
- `session/load` ID=2 only after loadSession strictly true;
- an `initialized` notification is a violation;
- server-request replies with the raw ID and `-32601`;
- null/absent result as success;
- P6 premature response in a single ≤512-byte write;
- P11 server request / later response after stdin half-close;
- P12 premature id=2 after more than 8192 bytes of padding;
- P7 EOF-without-exit, blocked write, inherited pipe, and the external watchdog.

A watchdog kill never counts as success.

### Compare and build

`test_impl.py` reuses the Codex `compile_pruv`/`build_llvm_objects` flow to build four runners:

| compiler | backend | output |
|---|---|---|
| Python | Rust | `work/py_rust/runner` |
| Python | LLVM + Rust shim | `work/py_llvm/runner` |
| Rust | Rust | `work/rs_rust/runner` |
| Rust | LLVM + Rust shim | `work/rs_llvm/runner` |

Every build uses verified mode and the G4b manifest; nothing passes via `--no-verify`.

Compared: JSON result, error class/code/stage, ACP transcript, primitive trace, final tree. P10
numbers are read keeping the raw token and compared as binary64 values; overflow→TS null is accepted
only at its trigger.

P4 can change the top 50 and the sort order, so timestamps are not simply deleted from the output:
the baseline is **a TS rerun on a fixture where only the unsupported raw date tokens are edited**.
P1–P6 and P9–P12 likewise check each input trigger and where the allowed difference appears. P8 does
not apply to Grok.

`q.py` runs seed 20261116, 2000 scenarios plus the fixed cases. `q_mutants.py` applies M01–M16 one at
a time and judges them on rs/rust first. A kill is a verifier rejection or a result/trace difference
on a normal run; build errors, crashes and watchdog kills are not kills.

## 7. Work packages and file ownership

Paths are relative to `bench/l3/grok/`. Nobody modifies the pinned `reference/`.

| package | owned files | depends on | provides | acceptance checks for a shell-less worker |
|---|---|---|---|---|
| A. ABI and common forms | `root.pruv`, `packet.pruv`, `util.pruv`, `pruv.json`, `IMPL_NOTES.md` | this document | cfg/ctl/step/result forms, public entries, JSON fragment helpers | every word offset and length formula, host-allowed types in root, ref/overflow guards, declarations match calls in the other packages |
| B. paths and small JSON | `path.pruv`, `summary.pruv` | A | chain/containment helpers, slot small-json state machine, summary/active decisions | every close path of readSmallJson, 65536 vs growth cap, surrogate cwd, nullish fallback, P9 fixtures traced by hand |
| C. listing | `list.pruv` | A, B, §9 | four-slot scheduler, indexed collection, sort/top-50 | hand-trace the A–E trace, no cap before sort, all candidates kept, comparator returns 0 for missing, sort follows `sort/v8sort.py` exactly |
| D. history | `read.pruv`, `entry.pruv`, `framing.pruv` | A, B | window → collector → read packet | drop-marker order, query/tag boundaries, synthetic false/0/empty, tools N times, collector contracts and growth sites, base=0 assembly |
| E. copy and ACP | `copy.pruv`, `rpc.pruv` | A, B; G10-PROC-ENV | copy state machine, real load check, rollback | 13-name allow-list, the two asymmetric exists checks, every fd/Process/Loop ended, one verify after rename, original error kept on failure |
| F. host, oracle, tests | `host.rs`, `harness.rs`, `str_adapter.rs`, `host/Cargo.toml`, `host/Cargo.lock`, `host/src/lib.rs`, `reference.mjs`, `reference-hooks.mjs`, `reference-primitives.mjs`, `common.py`, `oracle.py`, `test_impl.py`, `q_server.py`, `q.py`, `q_mutants.py`, `fixtures/**`, `Q_README.md`, `README.md`, generated `work/**` | A's forms; the rest at run time | primitive executor, bank storage, schedules, four-pair build, comparison | no importer decisions in the host, every opcode wired, pinned files untouched, fixed fixture categories map to M01–M16, policy classifier limited to its triggers |

`host/src/lib.rs` is a thin entry that includes the driver/renderer of `host.rs` as a library. The
harness links the generated `sol` module to that driver. The str adapter only does the same
representation conversion as Codex.

Order:

1. A fixes the packet/entry declarations first.
2. B, D, E, F can be written in parallel against those declarations.
3. C (scheduler and sort) can be written in parallel too, following §9.
4. The main agent links, compiles, verifies and runs the fixed tests.
5. After re-checking only the fixed failure paths, run the four-pair Q and the mutants.

A worker writing source without a shell is not verification; compile and run acceptance stays with
the main agent. Shared package changes are not part of this split.

## 8. Risks, confirmed facts and open items

| item | files/functions checked | conclusion and minimal response |
|---|---|---|
| no public mutable state | `SPEC.md:468–474`; internal body generation in `pruvc/llvmgen.py`; Rust twin `llvmgen.rs:6110` | no `inout` host entry or extern bypass; the official borrowed-array ABI plus delta packets. |
| cost of returning buffer structs | `SPEC.md:347–364`; `turns::Collected` | owning assignment of buffer fields can deep-copy; large persistent state does not round-trip through structs. |
| FS catalog gaps | `_add(file_*)` in `pruvc/foreign.py:65–75` | no readdir/lstat/no-follow/wx/positioned read; use the D1 primitive host, decisions stay in Pruv. |
| environment override | `pruvc/foreign.py:87`; `docs/G10_PROC_SPEC.md:730`; `RESULTS.md §135` | `proc_spawn_env` exists; `rpc::load_check` uses it. |
| Process/Loop ownership | Codex `rpc::start`, shared `transport::close`; `docs/G10_ASYNC_SPEC.md §2` | a failed-spawn Process is still consumed with wait/kill; once moved into a loop, the loop is closed on every normal path. |
| timer limits | `transport::receive` (`ev_proc_wait` after EOF), `transport::send` (`ev_proc_write`), `transport::close` | P7/D5 apply; no ad-hoc timeout bypass inside the importer. |
| stdin failure | `transport::send/receive`, pinned `tail::withRpcChild` | difference allowed only at the P11 trigger; do not assume stdout keeps being read on a failed process slot. |
| chunk order | Codex `rpc::open`; `st[2],st[4]` of `transport::receive` | do not copy Codex open as is: after initialize settles, process the remaining complete lines of the same chunk with an empty outstanding set, then send request 2. |
| number representation | `access::number`, `time::iso_of`, pinned `grok::listGrok` | P4/P9/P10 apply; a full JS binary64 implementation is not a new language prerequisite. |
| collector metadata offset | `turns::finish` `base≤3500000` requires | build turn text at base=0 and append title/model; do not truncate long models. |
| dev/ino and Node rounding | pinned `tail::openRead` `info.dev === expect.dev`/ino; host raw metadata boundary | real identifier ranges not checked by running; claiming TS equality beyond the JS-safe range needs Pruv to compare the binary64 rounding of the raw limbs; identity decisions never move into the host. |
| native stream instrumentation | pinned `grok::copyGrokSession`, `tail::openLineWindow` | a FileHandle Proxy is not enough; callback-based real read/write/close instrumentation plus native-parity fixed cases. |
| exact sort schedule | pinned `grok.ts:118` `items.sort` | **closed in §9.** |
| cancellation, groups, Windows | D3, P5; `docs/G10_PROC_SPEC.md §9`, `G10_ASYNC_SPEC.md` non-goals | out of scope for this contract; only the AbortError packet form is reserved, not in Q's success scope. |

## 9. Exact sort schedule (Node 26.4.0, V8 14.6.202.34-node.21)

The listing comparator is `(a, b) => Date.parse(b.updatedAt ?? '') - Date.parse(a.updatedAt ?? '') || 0`.
A missing or unparsable date gives NaN, and `NaN || 0` is 0, so a missing date compares equal to
everything: the comparator is not a total order, and the result depends on V8's exact call schedule.

`sort/v8sort.py` is a port of V8's `Array.prototype.sort` (Torque TimSort, `array-sort.tq`), plus
one behaviour found by probing Node: **arrays of length ≤ 7 skip run detection and go straight to
binary insertion sort** (`BinaryInsertionSort(0, 1, n)`). For longer arrays: minrun from
`ComputeMinRunLength` (`while n >= 64 { r |= n & 1; n >>= 1 }; n + r`), `CountAndMakeRun` (strictly
descending runs reversed), binary insertion to minrun, `MergeCollapse` / `MergeForceCollapse`, and
`MergeAt` with `GallopRight` / `GallopLeft` trimming and `MergeLow` / `MergeHigh` with galloping
(initial `minGallop` 7, `kMinGallopWins` 7). Only `cmp(...) < 0` is ever tested.

`sort/check.py` runs `sort/probe.mjs` under Node and compares the final order **and the full
comparator call sequence** with the port on 3000 seeded cases (sizes 0–3000, half with the Grok
comparator over dates with 0–90% missing, half with arbitrary inconsistent comparator tables):
0 mismatches over 3962043 comparator calls.

`list.pruv::sort_indices` must implement exactly this algorithm with the comparator on candidate
rows (the sign of `parse(b) - parse(a)` when both dates parse, else 0, using the P4 date policy).
The Q harness checks Pruv's order against the TS reference; the probe trace is never Pruv input.
