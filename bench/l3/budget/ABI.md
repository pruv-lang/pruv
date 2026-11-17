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
| op | `[code, now, quiet, origin, size, unit x5, defaultUnlimited, sessionId key, budgetId key, uuid key]`, key = `n, words(n) packed words` (packed-u16: `ceil(n/4)` i64 words, big-endian u16 lanes, zero tail lanes; P05b) |
| code | 1 current, 2 isHalted, 3 attach, 4 replaceForUserMessage, 5 reopen, 6 maybeClose |
| quiet | the host's `isQuiet()` answer, supplied up front on every call: 0 / 1; -1 = not known |
| fx | `fx[0]` = words used; effects 13 timerClear, 20 changed, 21 carryHeld (the old id is in out), 22 uuid consumed, 23 isQuiet consulted (maybeClose only, always the first effect) |
| out tag 3 | `[3, n, words(n) packed words]` budget id ref; replaceForUserMessage with effect 21 appends the old id `[m, words(m) packed words]` (P05b: ids stay packed, the host unpacks at egress) |
| out tag 4 | `[4, n, off]` budget view (current): the record is `st[off..off+n)`, valid until the next call that changes `st` |
| status | 0 done, 2 invalid, 3 out / fx too small, 5 Need isQuiet (only when quiet = -1), -N Grow (grow to N words, call again) |

The host consumes its next `isQuiet` answer only when the effects hold `[23]` (also on a non-zero status), so the
call count on the host side matches the reference, which consults `isQuiet` once and only after the session, state
and clock checks pass. Before this change `maybeClose` returned 5 to ask and the host called again, and `current`
copied the record into `out` as `[4, n, words...]`.
