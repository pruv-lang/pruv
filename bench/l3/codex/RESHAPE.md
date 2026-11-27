# Codex list and read import: Rust-shape reset (RUST_SHAPE_RESET §6 step 4.3f)

Scope: every function of `root.pruv`, `rpc.pruv`, `read.pruv`, `item.pruv`, `plan.pruv`, `list.pruv`, `folder.pruv`,
`util.pruv` and `packet.pruv` against a new hand baseline `hand/codex.rs` (std + serde_json; one function per Pruv
function, each marked `// Pruv: module::fn`). The older `hand/reader.rs` / `hand/replay.rs` (R09 perf lanes) cover only
the line reader and the replay; `codex.rs` covers the whole import (spawn, handshake, list, turns pages, fold, plan,
packet). It builds with `rustc -O --edition 2021` against the `host/` serde_json with no warnings. pruvc/, pruvc-rs/,
SPEC.md, docs/RUST_PARITY_SPEC.md and tests/ were not touched; no module moved to `lib` / `pkgs` and no shared module
changed, so no consumer runs were needed.

The JSON reader is the `pkgs/json` tape plus `pkgs/access`, not a serde `Value` (row J). The transport is the shared
`pkgs/transport` event loop, not a reader thread and a channel (row TH). The wire result is the host packet (row ABI).

What went:

- Untyped control state. `root` / `rpc` passed `ctl: [i64]` (8 slots, two of them write-only: the last id and the exit
  code). It is now `rpc::Ctl { fail, stage, slot, overall, got: bool, n }` built by `ctl_new`; the dead slots are gone.
  The error class is a `match error_class(..) { 1 => .., 2 => .., _ => .. }`; the `k == 1` / `k == 2` reply kinds are
  `if .. else if` (row VP).
- Copies the hand does not make. `kind_of` copied the raw id token (`raw(d, mid)`) to compare it; `id_is` now compares
  the token span in `d.source` in place. `not_handled` copied the id then joined three buffers; it reserves once and
  appends the id straight from `d.source`. `plan::take_segment` copied each line (`copy(subview)`) before trimming; it
  now trims the borrowed `subview` and copies only the kept item (`copy_range`). `list` built a `String`, checked it
  with `adapter_probe`, then copied it into bytes with `to_bytes`; it appends into one `[u8]` and passes it on.
  `read::finish` copied title / model with a `put_str` byte loop; now `copy_range` with a `str` source.
- Allocations the hand does not make. `req_list` / `req_read` / `req_turns` built pieces with `join` / `int_bytes` /
  `escaped` (one buffer per piece); each now does one `arr_reserve` and `arr_append` / `push_int` / `push_escaped`
  (hand `Vec::with_capacity` + `extend_from_slice`, grok's form). `util::to_bytes` filled a zero array by index; now
  `arr_reserve` + `arr_append`. `item::fold_reply` wrote the reply into a shared 32 KiB scratch `fb` with its state in
  `fs[3]`, and `read_turn` copied the prefix out (`head`); the reply now grows in the turn's own `reply: [u8]` with
  `used` / `clipped` as `inout` scalars, and `RawTurn` moves it (`put_piece`, `head` and the `fs` / `fb` buffers of
  `root` / `read::absorb` are gone). The prompt is a `[u8]` grown with `arr_append` (was a `String`, then `to_bytes`).
- Byte loops the hand does not write. `rpc::receive` scanned for LF byte by byte; it is one `find_byte`. `plan` split
  lines by a byte loop; it is a `find_byte` split loop. `read::stamp_str` rebuilt a `str` from the held bytes to check
  it; `stamp_ok` checks `span_ok(subview(..))` in place.
- Parallel arrays. `util::trim_span` returned a 2-slot array; it returns `Bounds { lo, hi }`.

## 1. Structure table (§1.5 form)

| step | Pruv before (4.3e tree) | Pruv now (4.3f) | hand Rust (`hand/codex.rs`) | copies / allocations (now vs hand) |
|---|---|---|---|---|
| control state | `ctl: [i64]` 8 slots (2 write-only) | `Ctl` struct, 6 fields | locals + `Result<_, Fail>` | same data; row TH for the loop |
| spawn + handshake | `start` / `open` on `ctl[..]` | same steps on `Ctl` | `start` + `call(initialize)` | same |
| line assembly | byte loop for LF in `receive` | `find_byte` in `receive` | `read_lines` thread, `read_until(b'\n')` | 0; row TH |
| line parse | `trim_span` → `[i64; 2]` | `trim_span` → `Bounds` | `serde_json::from_slice` | 1 tape vs 1 `Value` (row J) |
| reply kind | `raw(d, mid)` copy, then `id_is` on the copy | `raw_span` + `id_is(d.source, s, e, id)` | `v["id"].as_u64() == Some(id)` | 0 vs 0 |
| not handled | `raw` copy + `join` of three pieces | one `arr_reserve`, three `arr_append` | `Vec::with_capacity` + `extend_from_slice` | 1 vs 1 |
| requests | `join` / `int_bytes` / `escaped` per piece | one `arr_reserve`, `push_int`, `push_escaped` | `Vec::with_capacity`, `push_escaped` | 1 vs 1 |
| error class | three `if cls == ..` | `match error_class(..)` | `match error_class(..)` | same |
| turns page cursor | `raw(d, nxt)` | same (`raw` = `copy(subview)`) | `v["nextCursor"].to_string()` | 1 vs 1 |
| turn fold | shared `fb[32768]` + `fs[3]`, `put_piece` byte loop, `head` copy out | `reply: [u8]` grown per piece, `used` / `clipped` inout | `fold_reply(&mut String, ..)` | 0 extra vs 0 |
| turn prompt | `String`, then `to_bytes` | `[u8]` + `arr_append` | `String::push_str` | 0 extra vs 0 |
| held turns | `hs` / `ht` / `hb` flat arrays | same | `Vec<Turn>` | row VL |
| plan lines | byte loop split, `copy(subview)` per line | `find_byte` split, borrowed `subview` trimmed, `copy_range` of the kept item | `text.lines()`, `trim()`, `to_string()` of the kept item | 1 per kept item vs 1 |
| plan `at` | `copy(raw.completed)` | same | `Option<String>` clone | row 4.3d-M (no conditional borrow) |
| list rows | `String` + `adapter_probe` + `to_bytes` | one `[u8]` + `arr_append`, moved to `list_ok` | `Vec<Row>` | 0 extra; row ABI |
| read result | `put_str` byte loops for title / model | `copy_range` with a `str` source | `Window` struct | row ABI |
| stamp check | `stamp_str` rebuilds a `str` | `stamp_ok`: `span_ok(subview(..))` | `Option<String>` | 0 vs 0 |

## 2. Differences that remain

- VL (pkgs, open): held turns are the `hs` / `ht` / `hb` flat store, not `Vec<Turn>` of owned strings.
- TH (open): the hand reads lines on a thread into a `sync_channel`; Pruv runs the `pkgs/transport` event loop with
  explicit `step` / `receive` pumps. The pump loops in `root` (handshake, list pages, turns pages) are written out per
  request where the hand has one `call`; folding them needs a closure or a callback (row TH).
- UL (open): every loop carries an explicit cap and `decreases`.
- J (open): JSON is the tape; serde `Value` indexing is `util::field` / `access::*`.
- 4.3d-M (language): `copy(iso.text)` in `item::stamp` and `copy(raw.completed)` for the plan `at` stay; `move s.f` of a
  `[u8]` field of a `let`-bound struct is `type/array_alias`.
- 4.3f-P (bench, new, §3): the plan state is `pl: [i64; 64]` + `pb: [u8; 18024]` (counts, item offsets, blob), not a
  `Plan { items: Vec<String>, .. }` struct; changing it touches `read::absorb`, `finish` and `root` invariants.

## 3. Codex structure review (hand vs Pruv, per function) and what was applied

One Codex review (gpt-6.2-astra, high) of all 68 functions in the nine files against `hand/codex.rs`: 34 match (the
ABI packet words, `fold_reply`, `push_escaped`, `trim_span`, `id_is`, `start`, `step`, ...), 33 diff, 1 bug. Applied:

- Bug: `rpc::not_handled` on a request with no `id` member wrote `{"id":,"error":..}`; the hand serializes the absent
  key as `null`. It now appends `null` (unreachable from the pump, which calls it only after `kind_of` saw an `id`).
- `item::read_turn` kept a prompt piece counter `np` for the separator; it checks `len(prompt) > 0` like the hand.
- `list::build` kept `off` beside `tx` with `len(tx) == off` as an invariant; it uses `len(tx)`. The `len(idv) > 0`
  test before `valid_thread` (which rejects empty ids) is gone (mutant `list-admission-bypassed` follows it).
- `packet::wire_read` had no caller (`read::finish` uses `read_header`); removed.

Tried and reverted: `folder::same` and the `"lair"` originator prefix as one `eq_range` (hand slice compare / `starts_with`);
`eq_range` takes `[i64]` / `[u8]` / `[u16]` only and rejects `str` (`type_mismatch`). Row 4.3f-S.

Not applied (rows). Each needs more than the time cap allowed, a shared module change, or touches VL. Step 4.4e
(§3b) closed A, O, N, K, L, T, R and part of P and D; B, E, F, S and the `Option<Plan>` part of P stay open:

| row | finding | where | what the hand does |
|---|---|---|---|
| 4.3f-A | failure paths allocate and build requests before returning: receive buffers before `start` succeeds, read stores before the folder check, a provisional list fail packet, requests built after stdin closed; `InputTooLarge` folder check the hand lacks; cwd decoded twice; trimmed model copied | `root` | allocates after spawn and folder check; returns at once |
| 4.3f-O | `absorb` reports "budget full" through `hs[4]` and returns 0; `root` reads `data` and decodes the cursor before testing it; the request id is a second counter | `read::absorb`, `root` | `absorb` returns `true`; the caller stops before the cursor |
| 4.3f-N | `ctl.n = 0` after each parse is overwritten by the next `step`; `parse_line` takes `inout line` and an unread `st` | `rpc`, `root` | `parse_line(&[u8])` |
| 4.3f-B | closed in 4.4v: the private `rpc::receive` copy is gone (`step` calls `transport::receive`); `line` and the held arena `hb` start empty with a reserve and grow by `arr_append` (`extend_from_slice`); `ByteBuffer` / `byte_buffer` / `grow_bytes` and the unread stderr tail removed (stderr is still read and dropped: row 4.4v-SN in `../lib/RESHAPE.md`) | `util`, `rpc`, `root`, `read::absorb`; `pkgs/transport` | empty `Vec` + `extend_from_slice`; stderr discarded |
| 4.3f-K | `kind_of` looks up `method` when there is no `id`, and re-reads the number span with `raw_span` | `rpc::kind_of` | returns early on no `id` |
| 4.3f-E | `util::elements` builds the whole id array, then consumers walk it again | `util`, `list`, `item`, `read` | iterates; stops at 50 / at the budget (needs a `pkgs/access` element cursor; run all consumers) |
| 4.3f-L | `has_phrase` lowercases per comparison; `error_class` / `error_detail` decode the whole message before taking 300 units | `rpc` | lowercases 300 units once; `take(300)` on the iterator |
| 4.3f-F | the reply byte budget is clipped in `read_turn` and again in `absorb`; `replay` re-validates UTF-8 and re-folds the already folded reply; `text_layout` + `finish_inplace` plan twice; `finish` copies rows by a loop; `bad` is a 1-slot array | `item`, `read`, `packet::assemble`; `pkgs/turns` | sums raw lengths, clips once on admission; one budget pass |
| 4.3f-T | `item_text` keeps `first` / `total` beside `out` | `item::item_text` | borrows the direct `text`, joins only content parts |
| 4.3f-P | plan state is `pl: [i64; 64]` + `pb: [u8; 18024]` allocated before any plan; `marker_end` returns -1 for "no marker" and checks an unreachable `m > n` | `plan`, `root` | `Option<Plan>` built on the first plan; `marker_end` returns 0 |
| 4.3f-R | `child_args` copies each argument with `substr` before `proc_args_push` | `rpc::child_args` | passes `&str` |
| 4.3f-D | `str_at` uses the general owned decoder and re-checks the length | `util::str_at` | `as_str()` on the parsed value (`access::string_text_scanned_owned` exists) |
| 4.3f-S (language) | `eq_range` rejects `str` operands | `folder::same`, `list::build` originator | slice `==`, `starts_with` |

Accepted as is: request ids are written digit by digit into the request buffer (`push_int`); the hand makes a
`to_string()` temporary, so Pruv allocates less. The turns cursor is resent as the raw token (`README.md` policy),
where the hand re-escapes the decoded string.

## 3b. Step 4.4e (Codex rows of 4.3f)

Changed `root`, `rpc`, `read`, `item`, `plan`, `folder`, `util` (`codex/` only; no shared module touched).

- 4.3f-A closed (one part kept): `start` runs before any receive buffer; the read stores (`hs` / `ht` / `hb` / `pl` /
  `pb`) are allocated only after the thread result passed the cwd / folder check; no provisional fail packet; every
  request is built after `rpc::closed` saw stdin open; cwd is checked by `folder::lone_at` on the raw span (one
  decode); the trimmed model is moved when trimming keeps it whole. Kept: the `InputTooLarge` folder bound, which
  carries `len(req) <= 6 * len(folder)` for `req_list` (row 4.4e-FB).
- 4.3f-O closed: `read::absorb` returns `enum Absorbed { Open, Full, Broken }`; `Full` stops before the cursor is
  read, `hs[4]` is gone, the request id is `page + 3` like the hand.
- 4.3f-N closed: `parse_line(line: [u8], n)` (no `inout`, no `st`); the dead `ctl.n = 0` after each parse is gone.
- 4.3f-K closed: `kind_of` returns `enum Kind { Ignore, Server, Response }`, returns at once with no `id`, and reads
  the number from the tape span (no `raw_span`). The pumps `match` on it.
- 4.3f-L closed: `rpc::head` walks the message once with `jstr::step` and stops at 300 units, lowering ASCII on the
  way for `has_phrase`; `error_detail` takes the same 300 units without decoding the rest.
- 4.3f-T closed: `item_text` keeps only `out`; the separator test is `len(out) > 0`.
- 4.3f-R closed: `child_args` passes `subview(argv, lo, hi)`.
- 4.3f-P partly: `marker_end` returns 0 for "no marker" and the `m > n` test is gone. The `Option<Plan>` state stays
  `pl` / `pb` (row 4.3f-P kept, with VL).
- 4.3f-D partly: `str_at` uses `access::string_text_scanned_owned`. The 50331648 length test stayed here (row
  4.4e-DL). That reason was wrong: the `JsonDocument` invariant bounds the source by 16777216 bytes; 4.4z removes
  the test (§3c).

Codex structure comparison (gpt-6.2-astra, high; Codex 2 account): the 4.4e changes match the hand. Not applied, all
older than 4.4e or open shared rows:

| row | finding | where | what the hand does |
|---|---|---|---|
| 4.4e-LP | a lone surrogate or `1e400` in a member Pruv never reads is accepted | `rpc::parse_line` (`pkgs/json`) | serde rejects the line, waits for the next |
| 4.4e-EC | the 1000000 pump cap counts `step` calls (an empty poll counts) | `root` pumps | counts complete lines; Pruv's cap is the `decreases` measure |
| 4.4e-NU | an argument with an inner NUL is split into two by the host | `rpc::child_args`, `harness.rs` | keeps it; spawn fails `engine-missing` |
| 4.4e-FB | a folder over 1000000 bytes fails code 1 before spawn | `root` | spawns; `engine-missing` when the binary is absent |

Also repeated: held stores allocated for an empty page (4.3f-P / VL), element arrays (4.3f-E), owned decoding (4.3f-D),
stderr tail (4.3f-B, closed in 4.4v).

New rows: 4.4e-LP, 4.4e-EC, 4.4e-NU, 4.4e-FB (folder bound kept for the request size proof), 4.4e-DL (`doc_local` source bound),
4.4e-VT (the whole-root verify exceeds 300 s on this machine even at the unchanged baseline, load about 10).
Not started: 4.3f-E, 4.3f-F (shared modules), 4.3f-S (language). 4.3f-B closed in 4.4v.

## 3c. Step 4.4z: row 4.4e-DL

- `util::str_at` no longer tests `len(x) > 50331648`. The decoder ensures the text is at most 3 x the token span
  (`pkgs/access`), the `JsonDocument` invariant puts the span inside the source and bounds the source by 16777216
  (`pkgs/json`), so the result is at most 50331648. A proof-only `assert` of the span and source facts for `id`
  links the two; it is erased, so the arm is `return move x;`. 4.3f-D stays open only for the owned decode (the hand
  uses `as_str()`).
- Codex structure comparison (gpt-6.2-astra, high, default account): no branch added; the test removed cannot fire.
  Performance was not measured (older differences remain).
- Results (`rs/rust`, `rs/llvm`): `test_impl.py` 136/136 and 136/136 (verified); `q.py 300` 323 cases, 0 unexplained
  (262 exact, rest policy P1/P2/P3/P4/P6/P8) on both (exit 1 only for the absent `py` runners); `q_mutants.py`
  markers 16/16 match once (none in `util.pruv`); the full mutant run was not done (4.4e-VT).

## 4. Results

Step 4.4e, after the last code change (`root` verified in 285 s with a warm proof cache), `rs/rust` and `rs/llvm`:

| check | rs/rust | rs/llvm |
|---|---|---|
| `codex/test_impl.py` (verifier + fixtures) | PASS, 136/136 | PASS, 136/136 |
| `q.py` 300 random + 20 probes + 3 heavy | exact 262, P1 1, P2 2, P3 2, P4 53, P6 2, P8 1, unexplained 0 | same |
| `q_mutants.py` originals (6 retargeted to the 4.4e code) | 16 / 16 match exactly once | - |
| `q_mutants.py` full run (stopped at the time cap) | `wrong-id-accepted` killed by fixtures; `initialized-omitted`, `unsupported-mapping-lost`, `not-found-alternative-dropped` hit the 600 s compile wall (reported as `verifier-rejected` with detail `timeout`, so not a kill); 12 not run | - |
| hand `codex.rs` | builds with `rustc -O --edition 2021` + `host/` serde_json, 0 warnings | - |

The TS oracle passed 136/136 in every run. P1-P8 are the classes `q.py` already explains; no case needed a rerun. The
frozen py pairs were not built (PF); `q.py` was driven on the two rs pairs only. No perf timing was run. The 4.4e
`q.py` classes equal the 4.3f ones. `q_mutants.py` builds each mutant without a proof cache under the 600 s
`compile_pruv` wall, which the codex root exceeds on this machine (row 4.4e-VT); the mutant run needs a larger wall
or a cache before it can be read.
