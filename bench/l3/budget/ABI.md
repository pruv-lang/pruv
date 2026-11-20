# L3-10 M3: flat test encoding (ABI)

Code: `bud_api.pruv` (constants), `bud_test_abi.pruv` (validators and writers), entries in `m34_test/root_m3.pruv`.
Fixed interface: `m34_test/CONTRACT.md` (section M3). All words are `i64`. Limit `MAX_WORDS = 16_777_216`.

## State is never encoded per call

The budget state lives in one resident buffer owned by the host between calls (REQUEST_BUDGET_SPEC §8.3). A call,
resume or outcome carries no state words. The only whole-state encoding is the final observation (`m4_observe`),
which is written once at the end of a run for comparison, never per call.

## Header

| word | call | resume | outcome |
|---|---|---|---|
| 0 magic | `0x42554443` (1112884291) | `0x42554452` (1112884306) | `0x4255444F` (1112884303) |
| 1 | version 1 | 1 | 1 |
| 2 | total_len (== number of words) | same | same |

## Tagged value V

| tag | name | payload | size |
|---|---|---|---|
| 0 | absent (only as a top-level call argument slot) | — | 1 |
| 1 / 2 / 3 / 4 | undefined / null / false / true | — | 1 |
| 5 | int, `|v| <= 2^53-1` else ResourceBound(number) | `v` | 2 |
| 6 | string | `n`, n units `0..65535` | 2 + n |
| 7 | number bits | raw IEEE-754 word | 2 |
| 8 | array | `n`, n values | 2 + Σ size |
| 9 | object | `n`, n pairs `keylen, units, V` | 2 + Σ (1 + keylen + size) |
| 10 | session ref | `len, units` | 2 + n |

Depth <= 32 (top-level value is depth 1), else InvalidInput(depth). Every count fits in the remaining words.

## Call / resume

Call: `[magic, 1, total_len, op, now_ms, uuid_len, uuid..., argc, V...]`, size `7 + uuid_len + Σ size(V)`.
`op` in 1..25 or 30..35; `0 <= now_ms <= 8_640_000_000_000_000` (else ResourceBound(number)); `uuid_len` 0 or 36;
`0 <= argc <= 8`; nothing after the last argument.
Resume: `[magic, 1, total_len, seq, now_ms, V]`, `seq >= 1`, size `5 + size(V)`; tag 0 is not allowed.

## Outcome

`[magic, 1, total_len, kind, ...]`

| kind | rest |
|---|---|
| 1 Done | `result V`, `effect_count`, effects |
| 2 Need | `effect_count`, effects, `query_code`, `query_argc`, `V...`, `seq` (>= 1) |
| 3 InvalidInput | `reason` 1..8 |
| 4 ResourceBound | `which` 1..3 |
| 5 Grow | `words N`, `N > 0` |

Effect: `code (1..13), argc (0..4), V...`, size `2 + Σ size(arg)`. Query: code 1..3, argc 0..4.
Sizes: Done `4 + size(result) + 1 + Σ effect`; Need `4 + 1 + Σ effect + 2 + Σ size(arg) + 1`; kinds 3–5 are 5 words.
Effect and query arguments never use tag 0.

## Status and rejection order

A status `>= 0` is a length; a rejection is `-(100*class + reason)` (`bud_api::reject`); `-1` means `out` too small.
Class 3 InvalidInput: 1 header, 2 length, 3 value, 4 op, 5 trailing, 6 depth, 7 state, 8 resume.
Class 4 ResourceBound: 1 input, 2 number, 3 state.

Order of checks:
1. `len(inp) > MAX_WORDS`, or (with at least 3 words) declared `total_len > MAX_WORDS`: `-401`.
2. Header: fewer than 3 words, bad magic or version: `-301`; `total_len != len(inp)` or fewer words than the fixed
   prefix: `-302`.
3. Content, left to right: op `-304`, now_ms `-402`, uuid/argc/values `-303` (depth `-306`, int range `-402`),
   resume seq `-308`, trailing words `-305`.

## Writers

`put_word`, `put_header` (magic, version, placeholder), `patch_len`, `put_int`, `put_bits`, `put_str_from`,
`put_units_from`, `put_value` (recursive, bounded by depth 32), `put_args`, `put_effect`, `put_query`, `put_effects`,
`put_call`, `put_resume`, `put_outcome`. Each returns the new position, or `-1` if `out` is too small. Every read of
input and every write goes through `ent_u16::rd` / `ent_u16::wr`, so memory safety does not depend on the contents.
The test entries validate first and then re-emit value by value, so `out[0..r) == inp`.

## M7 test host protocol (`m7_test/root.pruv` `t_call`, `bud_lifecycle.pruv`)

Rows for the M7 lifecycle test entry only; the M3 encoding above is unchanged.

| row | words |
|---|---|
| op | `[code, now, reserved, origin, size, unit x5, defaultUnlimited, sessionId key, budgetId key, uuid key]`, key = `n, words(n) packed words` (packed-u16: `ceil(n/4)` i64 words, big-endian u16 lanes, zero tail lanes; P05b) |
| code | 1 current, 2 isHalted, 3 attach, 4 replaceForUserMessage, 5 reopen, 6 maybeClose |
| reserved | the host writes 0 to `op[2]` |
| fx | `fx[0]` = words used; effects 13 timerClear, 20 changed, 21 carryHeld (the old id is in out), 22 uuid consumed |
| out tag 3 | `[3, n, words(n) packed words]` budget id ref; replaceForUserMessage with effect 21 appends the old id `[m, words(m) packed words]` (P05b: ids stay packed, the host unpacks at egress) |
| out tag 4 | `[4, n, off]` budget view (current): the record is `st[off..off+n)`, valid until the next call that changes `st` |
| status | 0 done, 2 invalid, 3 out / fx too small, -N Grow (grow to N words, call again) |

Inside `maybeClose`, `is_quiet(k, ks, kn, fxn)` synchronously queries the session.
The key starts at word `ks`, has `kn` u16 units, and occupies `ceil(kn/4)` packed words.
The host answers 0 (false) or 1 (true). Any other answer returns status 2 without writing
state, effects, or the result. The drivers log each query as
`E quiet <answer> <kn> <packed words...> @<fxn>`, where `fxn` counts effects at query time
and is 0 for `maybeClose`.

## M10 halt host protocol (`m10_test/root_m10.pruv`, `bud_halt.pruv`; P24)

The M7 key and record rows above apply. The halt call adds its own query, effects and statuses:

| row | words |
|---|---|
| query | `running_ids(k, ks, kn, bk, bs, bn, fxn)`: runningDelegationIds(ref), asked by halt after SETTLE; ref = sessionId key `k[ks..]` (kn units) and budgetId key `bk[bs..]` (bn units); `fxn` = the number of effect words recorded so far (`fx[0]`; a real host ignores it, the test host logs it) |
| answer | `[0, n, (len, words(len) packed words)*]`, ids as packed key words; anything else (status not 0, a non-canonical key, words left over) makes halt return 2 |
| fx | SETTLE's 13 timerClear (if any), then halt's 31 cancelRecovery, 32 stopMain, `[33, i]` stopDelegation of answer id i, 34 stopAttachedTurns, 35 holdQueued, 36 save, 20 changed, 37 announce |
| status | 0 done, 2 invalid, 3 fx too small for the fixed part or a short view (nothing written), 6 the answer does not fit fx or the store room, a size above 2^32 words, or a record above 2^30 words (refused before any write) (terminal), -N Grow (grow to N words, call again) |

- When the host answers the query, it treats `fx[1..fxn)` as already performed (SETTLE's timerClear comes before the
  query).
- fx is valid on every status >= 0. On 2 or 6 after SETTLE, fx holds SETTLE's effects (a timerClear) and the store
  holds SETTLE's record change (activeMs, activeSince dropped); the host applies the effects as on status 0.
- Word and typed clocks share SETTLE arithmetic. Clamp the time inputs and the previous activeMs to
  ±(2^53 − 1), add nonnegative elapsed time, and saturate the new activeMs at the same bounds.
  This also normalizes activeMs when a consuming interval settles with zero elapsed time.
- A Grow (-N) carries no effects and writes nothing; the host grows `st` and calls again. The query is asked only on
  the call that gets past the first reserve.
- Status 6 is terminal: the query was observed, so the call cannot Grow and retry. Its store bound is QUERY_ROOM
  (256 words) reserved before the query, plus any free words already in the arena that the second reserve's in-place
  rehash recovers. In-call growth is queued (Appendix P24).
- M7's isQuiet used a prefetched answer before P24's form; it was already moved to the synchronous form (`is_quiet`).

## M12 resident auto-continuation (m12_test)

The test root exposes `m12_init`, `m12_grow`, `m12_load`, `m12_set_card`,
`m12_query`, and `m12_dump` with the M11 shapes. The additional entry is
`m12_auto(st, op, ref, ref_sid, caller, packet, out)`.

| op | operation | additional input |
|---|---|---|
| 1 | clearAutoContinue | ref and its packed session key |
| 2 | recordAutoContinue | ref, its packed session key, and one admitted JB value (empty means undefined) |
| 3 | stopAutoContinue | packed caller session key and optional ref with its packed session key |

Refs are V values. Supply `[1]` and an empty ref key for an omitted or
undefined stop ref. Packed keys use `[unit_count, packed_words...]`, including
`[0]` for an empty string. Arguments unused by the operation are empty.

Status 0 writes `[k, findSession, save, changed, V result, owner...]`, where
`k` counts the words following it. The V result is undefined for clear and stop
and boolean for record. The owner is present only on a stop save and is a
framed packed session key. Save precedes changed. On status 1, retry after
growing to the word count in `out[0]`. Statuses 2 and 3 indicate invalid input
and insufficient output space. The operation does not edit the budget on a
nonzero status.

The harness accepts `auto <op> | <ref> | <ref_sid> | <caller> | <packet>`
as decimal words. The test probe `room` prints `ok <grows> <free>` on the
Pruv lane, where `grows` counts Grow statuses so far and `free` is
`min(len(st), st[11]) - st[10]`; the hand lane prints `ok 0 0`. The fixed
checks use it to flag a Grow when the edit's size fits the free arena. It supports `perfR` / `handR` (record), `perfC` / `handC`
(clear), `perfT` / `handT` (flip), and `perfS` / `handS` (synthetic
stop). Each command takes size and repetitions and prints
`ns <ns/op> <checksum> alloc <allocations including reallocations>`.
