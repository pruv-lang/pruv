# M3/M4 shared contract (L3-10 request budget)

Fixed interface between `bud_api.pruv` / `bud_test_abi.pruv` (M3), `bud_store.pruv` / `bud_map.pruv` /
`bud_world.pruv` (M4) and the test harness in `m34_test/`. Spec: `docs/REQUEST_BUDGET_SPEC.md` §8, §9, §14;
resident state: `docs/RESIDENT_STATE_SPEC.md`.

Boundary rule: every decision is in Pruv. The Rust host only allocates buffers, calls entries, reads/writes files and
measures time. On `Grow` it allocates a zeroed buffer of `max(N, 2*len)` words, calls the grow entry, drops the old
buffer and retries the same call. It never reads or writes words of the state buffer.

## Build

All budget modules sit in `bench/l3/budget/` and contain no `mod` lines; they name each other only by
`crate::bud_x::f` and the shared library by `crate::ent_u16::rd`. A test root in `m34_test/` declares every module it
uses (`mod ent_u16; mod bud_api; ...`) and is compiled from the repository root with

    pruvc-rs/target/release/pruvc bench/l3/budget/m34_test/root_m3.pruv -o work/gen.rs \
        --lib bench/l3/budget --lib bench/l3/lib

`ent_u16` (in `bench/l3/lib`) gives total accessors: `rd(a,i)` (out of range reads 0), `wr(inout a,i,v)` (out of
range write dropped, `ensures len(a) == len(old(a))`), `zeros(n)`, `slice(a,s,e)`, `put_range(inout o, at, src, s, m)`.
Use `rd`/`wr` for every access to the state buffer and to untrusted input, so memory safety needs no invariant over
buffer contents. Keep entry `requires` O(1) (lengths only).

## M3: flat test encoding (`bud_api.pruv` constants, `bud_test_abi.pruv` codec)

Limit: `MAX_WORDS = 16_777_216` for any encoded input or output. A header `total_len` above it, or an input longer
than it, is `ResourceBound(input)`; a header `total_len` different from the real length is `InvalidInput`.

### Tagged value `V`

| tag | name | payload |
|---|---|---|
| 0 | absent (only as a top-level argument slot; elsewhere invalid) | — |
| 1 | undefined | — |
| 2 | null | — |
| 3 | false | — |
| 4 | true | — |
| 5 | int | one word `v`, `-(2^53-1) <= v <= 2^53-1`, otherwise `ResourceBound(number)` |
| 6 | string | `n`, then `n` UTF-16 units each `0..65535` (lone surrogates allowed) |
| 7 | number bits | one word: raw IEEE-754 bits (any 64-bit value) |
| 8 | array | `n`, then `n` values `V` |
| 9 | object | `n`, then `n` pairs: key string as `len, units` (no tag), value `V` |
| 10 | session ref | `len, units`: the stored session with that id |

Nesting depth at most 32 (top-level value is depth 1); deeper is `InvalidInput`. Every count/length is
`0 <= n` and must fit in the remaining words, otherwise `InvalidInput`. Values are checked iteratively or by
bounded recursion; validation is `O(words)`.

### Call

`[MAGIC_CALL=0x42554443, VERSION=1, total_len, op, now_ms, uuid_len, uuid units..., argc, V...]`

- `op`: 1 current, 2 isHalted, 3 attach, 4 replaceForUserMessage, 5 reopen, 6 charge, 7 noteFailure, 8 noteTool,
  9 forgetTurn, 10 runStarted, 11 runWaiting, 12 runEnded, 13 halt, 14 maybeClose, 15 raise, 16 decide, 17 setCard,
  18 hasCard, 19 clearAutoContinue, 20 recordAutoContinue, 21 stopAutoContinue, 22 bootRestore, 23 forgetSession,
  24 dispose, 25 fire (timer callback), 30 limitText, 31 refusalWire, 32 refusal, 33 continueText, 34 cardMissing,
  35 budgetUnitOf. Other codes are `InvalidInput`.
- `now_ms`: `0 <= now_ms <= 8_640_000_000_000_000`, otherwise `ResourceBound(number)`.
- `uuid_len` is 0 or 36; units `0..65535`.
- `0 <= argc <= 8`; exactly `argc` top-level values follow (tag 0 allowed here); nothing after them.

### Resume

`[MAGIC_RESUME=0x42554452, VERSION=1, total_len, seq, now_ms, answer V]` — `seq >= 1`, exactly one value, nothing after.
(The match of `seq` against the state's pending sequence is a semantic check of later milestones.)

### Outcome

`[MAGIC_OUT=0x4255444F, VERSION=1, total_len, kind, ...]`

| kind | rest |
|---|---|
| 1 Done | `result V`, `effect_count`, effects |
| 2 Need | `effect_count`, effects, `query_code`, `query_argc`, `V...`, `seq` |
| 3 InvalidInput | `reason` (1 header, 2 length, 3 value, 4 op, 5 trailing, 6 depth, 7 state, 8 resume) |
| 4 ResourceBound | `which` (1 input, 2 number, 3 state) |
| 5 Grow | `words N` (`N > 0`) |

Effect: `code, argc, V...`. Codes: 1 save(sessionId, record V), 2 changed, 3 stopMain(sessionId), 4 stopDelegation(id),
5 stopAttachedTurns(sessionId), 6 holdQueued(sessionId), 7 carryHeld(from, to), 8 queueContinue(sessionId, text),
9 dropHeld(sessionId), 10 cancelRecovery(sessionId), 11 announce(sessionId, text), 12 timerSet(budgetId, delayMs),
13 timerClear(budgetId). `0 <= argc <= 4`. Query codes: 1 runningDelegationIds(ref), 2 isQuiet(sessionId),
3 releaseHeld(sessionId, budgetId); `0 <= query_argc <= 4`.

Encoded size (checked by the harness): call = `7 + uuid_len + Σ size(V)`, resume = `5 + size(V)`;
`size` = 1 for tags 0–4, 2 for tags 5 and 7, `2 + n` for 6 and 10, `2 + Σ size(child)` for 8,
`2 + Σ (1 + keylen + size(value))` for 9. Outcome: Done `4 + size(result) + 1 + Σ effect`,
effect = `2 + Σ size(arg)`; Need `4 + 1 + Σ effect + 2 + Σ size(arg) + 1`; InvalidInput/ResourceBound/Grow `5`.

### M3 test entries (`m34_test/root_m3.pruv`)

All return a status: `>= 0` is the number of words written to `out`; negative is a rejection
`-(100*class + reason)` with class 3 InvalidInput, 4 ResourceBound (e.g. `-302` length error, `-401` input bound),
`-1` if `out` is too small.

- `pub fn m3_call(inp: [i64], inout out: [i64]) -> i64` — validate a call; on success re-emit it into `out` value by
  value through the encoder functions of `bud_test_abi` (not a bulk copy), so `out[0..r) == inp` exactly.
- `pub fn m3_resume(inp: [i64], inout out: [i64]) -> i64` — same for a resume.
- `pub fn m3_outcome(inp: [i64], inout out: [i64]) -> i64` — validate an outcome encoding and re-emit it through the
  outcome writer functions (header, then per kind: value, effects, query, seq; the writer back-patches `total_len`).

## M4: resident state (`bud_store.pruv`, `bud_map.pruv`, `bud_world.pruv`)

### Buffer layout (`bud_store`)

Header, 16 words: 0 magic `0x42554453`, 1 version 1, 2 total words (`== len(st)`), 3 slot base, 4 slot count
(power of two, `>= 8`), 5 entry base, 6 entry capacity, 7 entry top (high-water mark), 8 free-list head (-1 none),
9 arena base, 10 arena top, 11 arena end, 12 allocated entries (roots + live), 13 used slots (live + tombstones), 14 live arena
words, 15 pending-Need sequence (0 none).

`layout(L)` is a deterministic function of `L` alone: the largest power-of-two `slots >= 8` such that
`16 + slots + (slots/2)*ENTRY_WORDS + (slots/2)*32 <= L`; entry capacity `slots/2`; the arena is everything after the
entry table up to `L`. `L` below the minimum (`slots = 8`) cannot hold a state.

Header validation (`valid(st) -> bool`, O(1)): magic, version, word 2 equals `len(st)`, and every region word equals
what `layout(len(st))` gives, `0 <= entry top <= capacity`, free head `-1` or `< entry top`,
`arena base <= arena top <= arena end`, counts non-negative. All later reads still go through `rd`/`wr`, so a
corrupted body can give wrong answers but never an out-of-bounds access or a non-terminating loop (every loop is
bounded by a header capacity).

Capacity: `need(st, entries, words) -> i64` returns 0 if `entries` more entries (free list + unused capacity, and
`used slots + entries <= slots*3/4`) and `words` more arena words fit, otherwise the smallest `N` (found by doubling
from `len(st)`) whose fresh `layout(N)` fits `live entries + entries` and `live arena words + words`. Every
state-changing operation calls `bud_map::make_room` with its worst case before the first write and returns `Grow(N)`
with no write. `make_room` is `need` plus tombstone recovery: when only the used-slot bound fails and the live entries
fit, it rebuilds the slot table in place (amortized O(1)) and asks `need` again.

Arena: `[i64]` words for keys (UTF-16 units) and blobs. Each entry owns a key region (off 4, capacity 18) and a blob
region (off 12, capacity 19). A key or blob that fits its entry's region is written in place; otherwise it is appended
and the region becomes the new words. A freed entry keeps its regions, so a reused entry (free-list head) writes back
in place and steady churn does not grow the arena. Dead words of outgrown regions are reclaimed by `grow`.

### Entries and maps (`bud_map`)

`ENTRY_WORDS = 20`. Entry fields: 0 state (0 free, 1 live), 1 owner entry, 2 owner generation, 3 generation,
4 key off, 5 key len, 6 hash, 7 prev, 8 next, 9 child head, 10 child tail, 11 child count, 12 blob off,
13 blob len, 14–17 scalar values v0–v3, 18 key region capacity, 19 blob region capacity. -1 is "none" for links.

Entries 0, 1, 2 are the roots (live from `init`, never removed, no key): 0 sessions, 1 clocks, 2 toolSeen.
A map is "the children of an owner entry", kept in insertion order by the prev/next links and head/tail in the owner.
JS `Map` semantics: setting an existing key keeps its position; delete then set appends at the tail.

One open-addressing hash index (linear probing) over all non-root entries: slot word `0` empty, `-1` tombstone,
`tag(h) * 2^32 + e + 1` entry `e` with hash `h`, where `tag(h)` is the top 30 bits of `h` (so a probe skips most
non-matching entries without reading them). The hash mixes owner, owner generation and the key units (FNV-1a style
with `wrapping_mul`, `band` to the slot mask). `find` matches an entry only when it is live and its hash, owner, owner
generation (equal to the owner's current generation) and key all match; probing stops at an empty slot and is bounded
by the slot count. A slot word may name a free entry, or an entry since reused for another key (see removal below);
such a slot never matches and is not reused by inserts, so it behaves as a tombstone until the next rehash.
Load policy: before an insert that fits, the index is rebuilt in place (dropping tombstones) when the used slots would
pass half the slot count while the live entries take at most a quarter of it; before an insert that does not fit only
because of the 3/4 used bound, likewise. Each rebuild is O(slots) and at least slots/4 inserts from the next.

Removing an entry with children (e.g. `forgetTurn`) unlinks it and frees it with its children (one level; the world
has no deeper maps): the entry's own slot gets a tombstone, the children's slots are left in place (lazy deletion,
as above), every key/blob word leaves the live count, generations are bumped and the entries go on the free list.
Cost `O(probe length + children)` with O(1) per child, amortized O(1) per child insert. No reachable ghosts remain:
a free entry is never found, iterated or counted.

`grow(prev, inout next)`: requires `valid(prev)` and `len(next) >= len(prev)`; writes a fresh layout for `len(next)`
and copies every reachable entry (roots in order, each map in insertion order, children after their parent), key and
blob into the new arena, rehashing. Tombstones, stale slots and dead arena words are dropped. O(len(next)).

### World operations (`bud_world`)

- sessions (root 0): key sessionId, blob = opaque session record words (later milestones define them).
- clocks (root 1): key budgetId, blob = owning sessionId units, v0 since-present (0/1), v1 since, v2 timer armed (0/1),
  v3 timer deadline; children = runs: key runKey, v0 waiting (0/1).
- toolSeen (root 2): key turnId; children = activities: key activityId (a set).

### M4 test entries (`m34_test/root_m4.pruv`)

Status codes: 0 ok, 1 Grow (`out[0] = N`), 2 InvalidInput (bad op encoding or invalid header), 3 `out` too small.

- `pub fn m4_init(inout st: [i64]) -> i64` — writes an empty state (header, three roots); returns 0, or the minimum
  number of words `N > 0` if `st` is too small (nothing written).
- `pub fn m4_grow(prev: [i64], inout next: [i64]) -> i64` — 0 ok, 2 invalid prev or `next` too small.
- `pub fn m4_op(inout st: [i64], op: [i64], inout out: [i64]) -> i64` — one state operation; on 0, `out[0]` is the
  number of result words and `out[1..]` the result.
- `pub fn m4_observe(st: [i64], inout out: [i64]) -> i64` — whole-state dump (final observation only); 0 ok.

Keys in `op` are `len, units...`. Op encoding `[code, args...]`, result words:

| code | op | args | result |
|---|---|---|---|
| 1 | session_put | key, `n`, `n` words | `[1]` if new, `[0]` if replaced |
| 2 | session_get | key | `[0]` absent, or `[1, n, words...]` |
| 3 | session_del | key | `[1]` removed, `[0]` absent |
| 4 | session_keys | — | `[count, key...]` in order |
| 5 | clock_put | budgetId key, sessionId key | `[1]` new, `[0]` existed (sessionId replaced, position kept) |
| 6 | clock_del | budgetId | `[1]`/`[0]` (runs go with it) |
| 7 | clock_set | budgetId, since_present, since, armed, deadline | `[1]` ok, `[0]` no such clock |
| 8 | clock_get | budgetId | `[0]` or `[1, sessionId key, since_present, since, armed, deadline, run_count]` |
| 9 | run_set | budgetId, runKey, waiting | `[2]` no clock, `[1]` new, `[0]` updated (position kept) |
| 10 | run_del | budgetId, runKey | `[1]`/`[0]` |
| 11 | run_list | budgetId | `[0]` no clock, or `[1, count, (key, waiting)...]` |
| 12 | clock_keys | — | `[count, key...]` |
| 13 | tool_note | turnId, activityId | `[1]` new, `[0]` already seen |
| 14 | tool_forget | turnId | `[1]`/`[0]` |
| 15 | tool_count | turnId | `[count]` (0 if no turn) |
| 16 | tool_list | turnId | `[count, key...]` |
| 17 | turn_keys | — | `[count, key...]` |
| 18 | stats | — | `[allocated entries, used slots, live arena words, len(st)]` |

`m4_observe` writes `[sessions count, (key, n, words)..., clocks count, (key, sessionId, since_present, since,
armed, deadline, run count, (runKey, waiting)...)..., turns count, (turnId, count, activityId...)...]`, with `out[0]`
= number of words that follow.
