# Implementation notes

The sources were first written without a shell (by following the passing `mcp`, `qwen` and package files); the
sections below the first one list the spots that were checked first. The first section records what the first
verifier and test runs changed.

## Changes after the first verifier and test runs

- `rpc::start` ensures `len(ctl) == 8 && ctl[6] == 0` (the paging loop invariant needs the line length bound on entry).
- `rpc::child_args` reads each argument span into `lo`/`hi` and checks bounds and char boundaries at run time before
  `substr` (the solver timed out proving them from the split array).
- `read::replay` calls `feed` (one turn: `begin`, optional `answer`, one `turns::tool(n)` with the item sum, where the hand adds 1 per item;
  `add_tools` and its loop were removed in reset step 2.12), which carries a small contract, so the replay loop no longer holds the collector's full ensures in one context.
  `turns::grow` stays in the caller (an inout parameter cannot be reassigned as a whole).
- Overflow-safe guards are written `po > len(hb) - pn` rather than `po + pn > len(hb)`.
- `read::finish` checks each plan item offset (`0..18000`) at run time and carries the text-length bounds in the row
  loop invariant.
- Plan layout: with 30 items `pl[2 + 2*29] = pl[60]` overlapped the blob-used counter, so the counters moved to
  `pl[62]` (blob bytes used) and `pl[63]` (at present); `pl` stays 64 words (`read_plan_limits` caught it).
- Mutant 12 (reply cap 32767) survived `read_budget_reply_cap_boundary`: with one-byte text the extra turn it admits
  is trimmed again by the collector's own 2 MiB pass, which counts the real clipped reply. The new
  `read_budget_reply_cap_scalar_clip` uses 3-byte scalars, so each clipped reply is 32766 bytes and the extra turn
  survives the second pass. `reference.mjs` now exits only after stdout drains: a bare `process.exit` cut a large
  last result short on a pipe (seen when only these cases ran).
- The fake gained `await` (read N client lines after `pre`, before the response): without it the client could close
  the child before the fake logged the reply to a server request (`list_server_requests` raced).

## Verifier-sensitive spots

1. **Loop invariants / contracts were written by hand.** Most likely to need adjusting: `rpc::wait` (copy of mcp's loop
   with the `st/line/chunk/tb/ts` facts, cap 8388608), `read::replay` (the long collector invariants copied from
   qwen `read_window`, with `st[0] == fed`), `read::absorb` (many array-bound invariants for `hs/ht/hb/pl/pb/fs/fb`),
   `item::fold_reply` (`room - sep >= 0` is argued from the case split `sep == 2 && room < 2`), `plan::take_segment`.
2. **`substr` needs boundary facts.** `plan::take_segment` guards `substr(t, m, len(t))` with
   `is_char_boundary(t, m) && is_char_boundary(t, len(t))` at run time (the `+1` after an ASCII marker is a
   boundary, but the verifier cannot know it). If the verifier rejects `is_char_boundary(t, len(t))`, drop that half.
3. **`len(String)` is never used**; offsets are tracked in `off` locals (`list::build`).
4. **Struct / enum payloads**: `item::RawTurn` (arrays, ints, a bool; built with `copy(...)` like `turns::Collected`) and
   `crate::turns::Collected` returned from `read::replay`. If a struct literal is not allowed there, split `RawTurn`
   into an out-array plus `[u8]` return values.
5. **`i64` into `u8` stores**: `util::int_bytes` (`48 + n`), `util::escaped` (`hex(...)`, bytes), `rpc::error_detail`
   (`out[i] = x` after a range test), `read::put_str`. These follow the `line[st[1]] = c` pattern of `transport`.
6. **Aliasing**: `slot` / `overall` are copied out of `ctl` before every `rpc::call(..., inout ctl, ...)` so no
   argument reads `ctl` while it is borrowed inout.
7. **Effects**: `rpc::start` is declared `! async, net, proc, alloc` (like `mcp_check`) because it builds a loop; `wait`,
   `call`, `open` are `! async, proc, alloc`.
8. `util::elements` walks the tape once (`next = tape[6j+3]`, parent check `tape[6j+4] == c`, as `access::array_at` and
   qwen `answer_blocks`). It returns an empty array for a non-array, a corrupt tape or more than 1048576 elements.
9. `ev_new(3)` holds one process and one timer; there is no init timer (`receive` is called with `init = -1`,
   `ready = true`, so the `init` branch of `poll` never fires).

## Deviations from the spec text (also noted in docs/CODEX_IMPORT_SPEC.md as "Deviation:")

- **No package edit.** The spec asks `turns` for `tool_n` and a folded-reply entry. Instead: `item.pruv` folds replies
  with its own copy of the `answer` arithmetic (`fold_reply`), and `read::replay` calls `turns::begin`, one
  `turns::answer(F)` (when the turn has >= 1 reply), `turns::tool` in a loop, then `finish`. When the fold was
  clipped (`fs[2] == 1`) the replayed text is `F` plus one 4-byte padding scalar (U+10000): a clipped fold is always
  >= 32765 bytes (a cut leaves at most 3 unused bytes; the room-below-2 case is 32768), so the padded text always
  overflows the budget and `clip_bytes` cuts exactly the padding off again, setting `clipped` and saturating the
  counter. An exact fit (`32768` bytes, nothing cut) is not clipped, and a later reply is then ignored without the
  flag, as in TS. `completedAt` is applied only for a turn with at least one reply (TS calls `answer` only per reply).
- Replies longer than 2^31 etc. cannot occur (line cap 8 MiB, JSON limit 16 MiB).
- **Failure detail**: `error_detail` is the response message cut to 300 units with non-ASCII units shown as `?`
  ("rejected" when it has none); never compared.
- **isoOf defect** (corrupt tape, status < 0) is treated as an absent timestamp (qwen fails the import instead).
- `error.code` matches -32601 as JS `===` on the parsed binary64 number: the token's significant digits must lie
  between the exact decimal bounds 32601 -/+ 2^-39 (half an ulp; ties go to the even 32601, so both ends match) with
  the fifth digit in the units place (`rpc::is_minus_32601`). `-32601`, `-32601.0`, `-3.2601e4`, `-326010E-1`,
  `-32601.000000000001` and both exact ties match; `-32601.5`, `32601`, just past either tie do not (fixtures
  `read_unsupported_*`, `read_code_*`). The digit test was cross-checked against Python `float()` on 300,000 tokens.
- The lone check decodes the `cwd` units once more (`string_units`) besides `str_at`. It runs once per import on one
  string, so it is kept as the simplest correct form (confirmation audit 01dc18a1 item 1, judged not a hot path).
- Folder match: UTF-8 byte equality after `key_len`; a `cwd` with a lone surrogate (high or low) is rejected first
  by `folder::lone` (fixtures `read_cwd_lone_surrogate`, `read_cwd_lone_low_surrogate`), a valid pair matches
  (`read_cwd_surrogate_pair`).
- Known limits (P7, G10): `transport`'s `ev_proc_wait` after EOF and its blocking `ev_proc_write` are not bounded by
  the timeout. Both are G-track items (deadline-aware wait / nonblocking exit poll, nonblocking write).
- The 200-turn / 2 MiB held arena is a fixed 8,700,000 bytes (prompts <= 2,097,152, folded replies <= 200 x 32,768,
  stamps <= 200 x 48).

## TS behaviours interpreted

- `readTurn` order: items are classified in order; `plan` text counts only when `typeof text === 'string'` and
  `trim()` is truthy; any other `type` (including a missing or non-string type) is a tool; non-object items are dropped.
- `itemText`: `item.text` wins when it is a string even if blank (a blank `text` does NOT fall through to `content`).
- Budget `size` uses the sum of the raw reply bytes capped at 32768 (not the folded length).
- The last plan: `codexPlan` can never return none for a nonblank plan text (a nonblank line always yields an item),
  so "newest turn wins even when codexPlan returns none" cannot be constructed; `planSeen` is still set before the
  empty-turn skip and the budget check, as in TS.
- `folderKey` strips *all* trailing `/` (`/\/+$/`), all-slash strings stay unchanged (`"/"` vs `"//"` do not match).
- A server request is any line whose `method` is a string and whose `id` member exists (even `null`); the reply echoes
  the raw id token. A response is any other object whose `id` is a number token equal to the decimal id.
- `readCodex` thread/read: any error response is not-found, a transport failure propagates (timeout, read-failed).
- `thread/list` error response -> codex-unsupported (any error), cursor never used (non-win32).
- Spawn failure is always engine-missing (TS: the child's `error` event); no `command_missing` host function is used.

## Test-side notes

- Fixture expectations were derived by hand from the TS source (no run): check `ts/oracle` first (it checks the pinned TS
  against the expected JSON); a TS mismatch there points to a wrong fixture, not to Pruv.
- Spots in fixtures worth a look if they fail: `list_updated_forms` (`updatedAt: ""` should be absent in both),
  `read_prompt_multibyte_clip` (10922 characters), `read_reply_multibyte`, `read_budget_*` (budget arithmetic),
  `read_plan_limits` (`more` 5), `read_error_cut_*` (300-unit cut).
- `q.py` random timestamps include floats and non-canonical ISO strings (class P4); texts include lone surrogates only in
  the fixed probes (P2).
- `common.fake_binary()` writes an `sh` wrapper that execs `sys.executable -B q_server.py "$@"`; paths with spaces are
  single-quoted.
