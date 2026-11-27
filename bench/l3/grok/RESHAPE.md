# Grok ACP load check and RPC lines: Rust-shape reset (RUST_SHAPE_RESET §6 step 4.3)

Scope: the whole of `rpc.pruv` (the per-line scan of DB65, `RESHAPE_LINE.md`, plus everything DB65 left on the tape and the
load-check driver) against `perf/hand_rpc.rs`, which is extended here with the load check (`load_check`, `Rpc`, `Check`)
so that every Pruv function has a hand counterpart. Step 4.3b (§3c) closed the four rpc structure rows (U, V, C, D) and
`entry::query_range`; the rest of the history path (`perf/hand_history.rs`: the `pkgs/turns` collector store and
`read::assemble_store`) is still open (`../lib/RESHAPE.md` "Next"). pruvc/, pruvc-rs/, SPEC.md and tests/ were not
touched; `bench/l3/lib` and `bench/l3/pkgs` are unchanged (no shared-module consumer runs needed).

Tables §1 and §2 describe the tree after 4.3b; line numbers are those of 4.3b.

What went:

- The tape fallback. An escaped key in a tracked object is matched while decoding (`key_is`, hand `key_is`); an object
  `error` is read from spans (`detail`, `compact`, hand `detail`/`compact`). `rpc.pruv` no longer calls the `util` tape
  parser, `json_value` or `error_detail`, and `Cap.fb` is gone. The `perf_root.pruv` tape lane (`rpc_line_tape`) is gone.
- The internal word packets. The 16-word `ctl` `[i64]` and the `aux` byte buffer that carried the load-check state
  between `pump`, `drain` and `load_check` are one typed struct `Check` (hand `Check`): `fail`, `stage`, the loop slots,
  the line length, `detail` and `raw`. The only word packet left is the mode 3 result packet `load_check` returns to the
  host (§3 keep list: the host boundary).
- Copies the hand does not make: `req_load` built the request by `join` of five pieces; it now reserves once and appends
  (hand `Vec::with_capacity` + `extend_from_slice` + `push_escaped`). The perf split copied nothing before either, but
  walked byte by byte; it now uses `find_byte` (hand `position`/memchr).

## 1. Structure table (§1.5 form)

| step | Pruv before (4.2 tree) | Pruv now | hand Rust | copies / allocations (now vs hand) |
|---|---|---|---|---|
| per line: trim, scan | `trim_bounds`, `scan` into `Cap`; escaped key in a tracked object or `c.err == 6` → whole-line tape parse | the caller trims and passes the view `t`; `scan(t, c)` walks `value` → `array`/`object` → `key_slot` → `key_is`; UTF-8 checked per sequence in `sstr` (4.3b) | `trim_bounds`, `scan(t)`: `Sc::value` → `array`/`object` → `member` → `key_is`; `utf8_width` in `string` | 0 / 0; same |
| decoded text (method, update text) | `text_span` returned a tagged span; `buf` `resize`d to the encoded length, `dec` stored by index | `text_of(t, c, which, inout buf) -> ref [u8] from(t, buf)`: a view of `t`, or `buf` truncated and filled by `dec` (plain runs `arr_append`ed up to each `\`) (4.3b) | `text_of(src, v, buf) -> &'a [u8]`, `buf.clear()`, `unescape` with `extend_from_slice` | 0 extra; same |
| server request → not-handled reply | `not_handled_line`: one exact-size zero-filled alloc, head/tail stored byte by byte (`put_lit`) | `arr_reserve` + three `arr_append` | `Vec::with_capacity(head+id+tail)` + three `extend_from_slice` | 1 alloc; same |
| response with an object error: detail | tape `error_detail` (owned str) → `aux` | `detail(view, c, inout ck.detail)`: spans of `error.message`, 300 units, into the reused Vec | `detail(t, &cap.msg, &mut ck.detail)` | 0 extra; same |
| response with an object error: code | tape `json_value(error.code)` (owned str) → `aux` | `compact(view, s, e, inout ck.raw, inout buf)`: one walk; escaped strings decoded into `buf` and re-escaped | `compact(t, &cap.code, &mut ck.raw, buf)` | `buf` reused; same |
| initialize response check | tape `field(result).agentCapabilities.loadSession` | `Cap.res`, `Cap.caps`, `Cap.ls` filled by the scan (tracks 6, 7) | `res_obj`, `caps_obj`, `load_true` filled by `member` | 0; same |
| state between steps | `ctl: [i64]` (16 words) + `aux: [u8]` | `Check` struct, `inout ck` | `Check` struct, `&mut ck` | none; same |
| session/load request | `join` ×5 + `escaped` ×2 (each a fresh buffer) | one `arr_reserve(fixed + 6·thread + 6·folder)`, `arr_append`, `push_escaped` | `Vec::with_capacity`, `extend_from_slice`, `push_escaped` | 1 alloc; same |
| initialize request | `to_bytes(lit)` | same | `REQ_INIT` borrowed | 1 alloc vs 0 (gap L) |
| reading the child | G10 loop: `transport::receive_grow` into `line`, 64 KiB `chunk` | same | reader thread, 64 KiB `read`, `chunk[..n].to_vec()` over `mpsc`, `recv_timeout(deadline)` | Pruv 0 per chunk; hand 1 per chunk (gap TH) |
| writing the child | `transport::send(l, slot, data, budget)` (owned data) | same | `write_all(&data)` on `Option<ChildStdin>` | owned vs borrowed (E2 below) |
| result | mode 3 packet | same, `failure(code, stage, raw, take detail)` gives the detail Vec to the text | `Result<(), LoadError>` | host boundary |
| perf split (`grok_rpc_lines`) | per-byte loop for LF | `find_byte(win, 10, start, len)` | `iter().position` | 0; same |

## 2. Line table against `perf/hand_rpc.rs`

Kinds: `=` same statements; `E1` a check the hand gets from an index panic or a debug assert (Pruv has no panic path, so
the test is explicit and never taken on valid input); `E2` an ABI or host-API difference at a boundary; `P` proof text
only. Hand lines are `perf/hand_rpc.rs`, Pruv lines `rpc.pruv` unless marked.

| hand | Pruv | kind | notes |
|---|---|---|---|
| `space` :77 | `space` :45 | = | the `c < 194` early return added after the structure review |
| `trim_bounds` :111 | `trim_bounds` :66 | = | ASCII fast path; the dead `lo == i` branch of the back walk removed after the review; `pub` since 4.3b (the callers trim) |
| `utf8_width` :147 | `utf8_width` :181 | = / P | new in 4.3b; same lead and second-byte ranges |
| `Sc::ws` :181 | `ws` :125 | = | |
| `Sc::string` :192 | `sstr` :208, `stop` :142 | = / E1 | Pruv returns the span as `Val` instead of moving `self.p`; the end-of-line guard is explicit; since 4.3b `stop` stops at bytes >= 128 and `sstr` checks the sequence with `utf8_width`, and `\u` checks the four digits with `hexv` and stops at the first bad one (the hand's `is_ascii_hexdigit`; before, `hex4` computed the value) |
| `Sc::number` :243 | `num_end` :260, `digits` :243 | = | |
| `Sc::lit` :286 | `lit_end` :321 | = | |
| `Sc::value` :296 | `value` :400 | = / E1 / P | one record per value, else-if dispatch on the first byte; containers call `array`/`object` with `d - 1` (P: `decreases 2 * d + 1` and `2 * d + 2` for the mutual recursion); `rec` is an `inout i64` (hand `self.rec`) |
| `Sc::array` :343 | `array` :442 | = / P | 4.3b: a separate loop; after a comma only the comma is consumed and `value` skips the space (as the hand, after the 4.3b review) |
| `Sc::member` :364 | `key_slot` :373 + the reset chain in `object` | = | same track → slot table; resets run after the colon is seen (as the hand, after the 4.3b review) |
| `Sc::object` :405 | `object` :469 | = / E1 | 4.3b: a separate loop; key, colon, `member`, value, slot store (`else if` chain for the hand `match`, gap VP) |
| `key_is` :454 | `key_is` :332 | = | plain keys compare bytes; escaped keys decode `\uXXXX` and the one-letter escapes while comparing |
| `hex4` :497 | `hex4` :168, `hexv` :158 | = | |
| `unescape` :506 | `dec` :577, `push_cp` :564 | = / P | 4.3b: `find_byte` for the next `\`, the plain run `arr_append`ed, decoded chars pushed (1-4 bytes); no `resize`, no stores by index |
| `text_of` :555 | `text_of` :625 | = | 4.3b: `-> ref [u8] from(t, buf)` (the two-source borrow): a view of `t` when plain, else `buf` truncated, filled by `dec`, and viewed |
| `emit` :570, `detail` :576 | `detail` :643 | = / E1 | `emit` is inlined (two `arr_push` sites); the `kind != STR` and span tests are one guard |
| `compact` :620 | `compact` :690, `hexd` :679 | = / E1 | output bound `len(out) <= 6 * (e - s)` and `e - s <= 16777216` (the perf input cap) are contracts only; closing quote is the hand's inline escape-skipping scan; since 4.3b `buf` is truncated and filled by `dec` |
| `scan` :674 | `scan` :524 | = / gap IW | 4.3b: no whole-line UTF-8 pass, `Cap` has no `lo`/`hi`/`root`/`rec`; the captures are reset field by field because an `inout` struct cannot be assigned a fresh value as a whole (hand returns a fresh `Cap`; gap IW, §3) |
| `line` :685 | `perf_root.pruv` `rpc_line` | = | trims, scans the view, `line_kind`, `not_handled_line`, `detail`/`compact`, `text_of` ×2; since the 4.3b review `compact` runs for any `code` in a line of up to 16 MiB (before, a `code` token over 8 MiB was skipped: an output difference against the hand) |
| `rpc_lines` :738 | `perf_root.pruv` `grok_rpc_lines` | = | |
| `Check`, `Fail` :772-788 | `Check` :777 | = / E2 | `fail` is an `i64` code (0 = None); `slot`, `timer`, `budget`, `ready`, `n` hold the loop handles the hand keeps in `Rpc` |
| `REQ_INIT` :790 | `req_init` :843 | E2 | gap L: no borrowed `[u8]` of a literal for `send` |
| `push_escaped` :793 | `push_escaped` :849 | = | |
| `req_load` :805 | `req_load` :868 | = | |
| `Rpc::send` :834 | `transport::send` | E2 | the G10 transport takes the bytes owned and applies the overall budget as the write deadline; a write failure half-closes stdin in both |
| `Rpc::step` :844 | `step` :818 | E2 (TH) | `receive_grow` on the event loop vs `recv_timeout` on the channel; the 8 MiB line cap, timeout (fail 5) and read failure (fail 6) map one to one |
| `Rpc::on_line` :887 | `fast_kind` :753, `line_kind` :536 + the `fk` arms of `pump`/`drain` | = | the caller trims and passes the view; `drain` (id 99) skips the response test, as the hand (4.3b review) |
| `Rpc::pump` :913 | `pump` :912 | = / P | the event cap counts loop events, not lines (UL) |
| `Rpc::drain` :936 | `drain` :950 | = / P | `n < 70000` cap (gap UL) |
| `on_reply` :946 | `on_reply` :889 | = / E1 | span guards on the subview; M16 marker kept |
| `load_check` :962, `Rpc::before_send` :929 | `load_check` :1012, `start` :781, `before_send` :980 | E2 (TH) | spawn + loop registration is `start`; a failed start now returns before any buffer is made (as the hand, after the review); `before_send` is in the hand too now (see §3) |
| `failure` :1031 | `failure` :993 | = / E2 | takes `raw` and `detail` owned (after the review; before, `raw` was borrowed from `Check`); `LoadError` is written as the mode 3 packet |

## 3. Differences that remain

| difference | reason | row |
|---|---|---|
| the child is read on the host event loop, not by a reader thread over a channel | no threads or channels in the language | TH (open, `../lib/RESHAPE.md`) |
| `transport::send` takes the request bytes owned (`req_init()` allocates the literal) | `send` and the loop API take `[u8]` by value; no borrowed `[u8]` of a `str` literal | L (open) |
| pump/drain loops carry event caps (`1000000`, `70000`) | no unbounded loop | UL (open) |
| `load_check` returns the `[i64]` result packet | host boundary (§3 keep list) | — |
| `failure` takes `ck.detail` by moving the field out and refilling it with an empty array on the next statement | no struct destructuring; a moved field must be reassigned at once (an empty Vec does not allocate) | none (same cost) |
| `scan` resets the 22 `Cap` fields one by one where the hand returns a fresh `Cap` | `c = cap_new()` on an `inout c: Cap` is `type/immutable` ("inout parameter c cannot be reassigned as a whole"); returning a fresh `Cap` would allocate nothing either way, but callers reuse `c` through `inout` | IW (new, `../lib/RESHAPE.md` 4.3b) |
| the slot dispatch in `object` is an `else if` chain where the hand has `match` arms | an `i64` scrutinee takes no literal arms (R-6u-6 literals are payload positions only), and sixteen separate `if slot == k` statements (the first 4.3 form) made verification of `value` run for hours | VP (new, `../lib/RESHAPE.md`) |

Structure rows (not language gaps). U, V, C and D were left open by 4.3 and are **closed in 4.3b** (§3c); R is
**closed in 4.3c** (§3d):

| row | Pruv | hand | change |
|---|---|---|---|
| 4.3b-R (closed 4.3c) | `load_check` made the line buffer with `alloc(4096, 0)` | `Vec::with_capacity(4096)` | `receive_grow` starts from length 0; `load_check` reserves 4096 on an empty buffer |

The 4.3 table, kept for the record (all four closed in 4.3b):

| row | Pruv | hand | change |
|---|---|---|---|
| 4.3b-U | `scan` runs `valid` (`utf8_valid` over the whole trimmed line) before the walk | UTF-8 checked per sequence inside `Sc::string` only | `stop` stops at bytes >= 128, `sstr` checks the sequence width (hand `utf8_width`), `valid` goes |
| 4.3b-V | `value` has one container loop that tests `obj` per element and calls `ws` twice after a comma | `array` and `object` are separate loops; `member` alone decides slots | split the loop into array and object helpers |
| 4.3b-C | `Cap` carries `lo`, `hi`, `root`, `rec`; consumers re-add `lo` | the caller trims, `rec` is scanner state, `Cap` holds the members | trim in the caller, pass the view, drop the four fields |
| 4.3b-D | `text_span`/`compact` `resize` `buf` to the encoded length (zero fill) and `dec` stores by index, byte by byte | `buf.clear()`, plain runs appended, decoded chars pushed | `buf = truncate(buf, 0)`; `dec` appends the plain run before each `\\` with `arr_append` and pushes decoded bytes |

New gap rows: VP (verifier cost of independent `if` runs, `../lib/RESHAPE.md` step 4.3); IW (4.3b).

## 3b. Codex structure review (hand vs Pruv, per function) and what was applied

One review (Codex gpt-6.2-astra, high) of `perf/hand_rpc.rs` against `rpc.pruv` and the perf rpc lane: 16 findings.

- Behaviour, fixed in the **hand** (the Pruv follows the reference and the repo's P11 note):
  1. A child that closed stdin before stage 26 and then answers within the deadline: the hand skipped the write and kept
     reading, Pruv reports read-failed (timeout when the timer fired). The hand now has `Rpc::before_send` with the same rule.
  2. A response line of more than 8 MiB whose LF arrives in a later chunk: the hand tested the cap only on the no-LF
     branch after appending. It now tests `line + piece > LINE_CAP` before every append, as `receive_grow`.
- Structure, fixed in Pruv: `space` `c < 194` early return; dead `lo == i` branch in `trim_bounds`; `not_handled_line`
  reserve + three appends (`put_lit` gone); `compact` inline closing-quote scan and key test bounded by the value end;
  `load_check` returns right after a failed `start` (no buffers made); `failure` takes `raw` owned.
- Structure, left open as 4.3b rows: U, V, C, D above.
- Not applied: the `[i64; 15]` accumulator of `rpc_lines` (measure harness, E2); `Fail` as a named variant (an `i64` code
  in `Check`, converted where the packet is made, E2); the pump cap counting events rather than lines and the drain
  counter (UL); `REQ_INIT` borrowed (L); the tagged `Span` (B family; gone in 4.3b, `text_of` returns a borrow).

## 3c. Step 4.3b: the four rpc rows and `query_range`

- **U** `stop` also stops at a byte >= 128; `sstr` takes the sequence width from `utf8_width` (hand `utf8_width`, the
  `parse::utf8_width` rules) and fails on 0. `valid` (`utf8_valid` over the whole line) is gone. Outside strings a byte
  >= 128 fails the scan anyway, so no line passes that failed before or the reverse.
- **V** `value` dispatches on the first byte (else-if chain) and calls `array` or `object`, two separate loops with
  `d - 1` (mutual recursion, `decreases 2 * d + 1` / `2 * d + 2`). `rec` is an `inout i64` threaded like the hand's
  `self.rec`.
- **C** `Cap` lost `lo`, `hi`, `root` and `rec`. The callers (`pump`, `drain`, `perf_root::rpc_line`) trim with the now
  public `trim_bounds` and pass the view `t`; `fast_kind(t, id, c)`, `line_kind`, `not_handled_line`, `on_reply`,
  `detail` and `compact` take positions in `t`. `scan` resets the fields one by one (gap IW).
- **D** `dec(b, s, e, inout out)` appends: `find_byte` for the next `\`, `arr_append` of the run, the decoded char
  pushed by `push_cp` (1-4 bytes). `text_of(t, c, which, inout buf) -> ref [u8] from(t, buf)` replaces `text_span`:
  the generated Rust is `fn text_of<'a>(t: &'a [u8], .., buf: &'a mut Vec<u8>) -> &'a [u8]` with `buf.truncate(0)`,
  as the hand. `compact` truncates `buf` and calls `dec`. `perf_root::fold_text` is gone.
- **`entry::query_range`** (history path item 3) returns a `crate::text::Span` (`lo == -1` for none) instead of a
  2-word `[i64]`, the hand's `Option<(usize, usize)>`; no allocation. The first test is one short-circuit `if`.
- **Canon** (history path item 4): Pruv keeps an offset into the line where the hand copies 24 bytes; Pruv is the
  cheaper side and also accepts numeric stamps (kept). No change.
- Codex structure review of 4.3b (one, gpt-6.2-astra high, per function): six findings, five applied:
  1. *Output bug in the perf lane*: `rpc_line` skipped `compact` when the `code` token was over 8 MiB, the hand does not
     (a line up to 16 MiB). `compact` now takes `e - s <= 16777216` and the perf guard is gone.
  2. `array` consumed the space after a comma and `value` tested it again; now only the comma (hand).
  3. `object` classified the key and reset captures before seeing the colon; now after it (hand).
  4. `sstr` computed the `\u` value with `hex4`; now it checks the four digits with `hexv`, stopping at the first bad one
     (hand `is_ascii_hexdigit`).
  5. `line_kind` with id 99 (drain, nothing pending) still compared the response id; now skipped (hand `on_line`).
  6. Not applied (open row 4.3b-R): the 4 KiB zero-filled line buffer of `load_check` (shared `pkgs/transport` contract).
- History path still open (`../lib/RESHAPE.md` "Next"): the `pkgs/turns` collector store and `read::assemble_store`
  (three allocations where the hand reserves the packet once). Both change `pkgs/turns` and need all its consumers run.
- Verification: the scratch root with `rpc`, `line` and `entry` verifies in 97 s on a warm proof cache (191 s first).

## 3d. Step 4.3c: the history packet, row 4.3b-R, review fixes

Against `perf/hand_history.rs` per function:

| Pruv | hand | 4.3c |
|---|---|---|
| `read::assemble_store` built `rows` (`10 * kept` words), `meta` (`packet::meta_new`), checked them with `packet::rows_ok` and copied both into a third, zero-filled packet | `Collector::packet`: `Vec::with_capacity(28 + nr + nx)`, `resize(28)`, rows pushed oldest turn first, text extended, header and meta written in place | same shape: `alloc(0, 0)` + one `arr_reserve(28 + nr + nx)`, 28 zero words, rows pushed oldest first (`arr_push` of 10 words), text pushed (`widen` / `widen_str` now append), header and meta (`p[4 + k]`, no mutable sub-slice) in place. One allocation, as the hand. `packet::rows_ok` is gone (no caller) |
| `assemble_store` stops at a draft whose Store offset is out of range | (cannot happen) | kept as one check per draft, now returning the InternalInvariant packet: `widen` needs the source range, which the Store contract does not carry (row VL) |
| `assemble_store` ends with `len(p) == 28 + nr + nx` | (by construction) | kept: `text_layout` gives `tl` but not the per-draft stamp sum the push loop would need to prove it |
| `event_step` re-checked `kd`, `lo`, `hi` at run time; `window` carried an event count `j` and a `bad` flag | `history_window` handles the borrowed line in the LF branch and returns on failure | the ranges are `event_step` preconditions; `window` returns the error packet at once; `j` is gone (`st[0] <= p` bounds the turn count) |
| `line_step` returned early on a line over 16 MiB | (the caller caps lines at 1 MiB) | `len(line) <= 1048576` is a precondition; the check is gone |
| `load_check` line buffer `alloc(4096, 0)` (row 4.3b-R) | `Vec::with_capacity(4096)` | `alloc(0, 0)` + `arr_reserve(4096)`. `transport::receive_grow` now takes and keeps `len(line) <= cap` (was `1 <= len(line)`); grok `step`/`pump`/`drain` and `mcp/root.pruv` drop `1 <= len(line)`. mcp still starts from `alloc(4096, 0)` (not part of this step) |

Not changed:

- **Collector store** (`pkgs/turns` `st`/`sl`/`Store`): the hand `Collector { slots: Vec<Slot> }` with owned prompt and
  reply buffers was probed again with this `pruvc-rs` (a struct with a `Vec<Slot>` field, `Slot { prompt: [u8], reply:
  [u8], .., created: At }`, `enum At { Absent, Canon(b: [u8; 24]) }`, `vec_at_mut` + `truncate` / `arr_append` on the
  element fields). rs/rust verifies and builds; rs/llvm rejects it ("has owned collection fields, which need the Rust
  backend"; as an `inout Vec<Slot>` parameter, "G6 foreign calls and collections are not implemented by the llvm
  codegen layer"). Gap VL (`../pkgs/RESHAPE.md` step 2.12) stands; `pkgs/turns` is unchanged.
- Codex structure review (one, gpt-6.2-astra high): 12 rows, packet words, order and reservation match the hand.
  Applied: the `event_step` / `window` and `line_step` rows above. Left open as 4.3c rows:

| row | Pruv | hand | change |
|---|---|---|---|
| 4.3c-T | `entry::feed` resolves the timestamp (numeric stamps format 24 bytes) before the role / synthetic / blank filters | `Entry::at()` is called only for an accepted user or assistant entry | resolve the stamp right before `begin2` / `answer2` |
| 4.3c-B | `user_view` / `assistant_view` test a 100000000-byte bound and the query char boundaries at run time; `user_view` has two `begin2` call sites | one call site after the prompt view is chosen | carry the bounds in the `query_range` / caller contracts and call `begin2` once |
| 4.3c-G | `receive_grow` copies up to the cap and then reports -3; the line is cleared lazily (`filled = 0`, truncate on the next append) | `Rpc::step` fails before copying an over-cap piece and clears after each line | compare the LF position with the room left before `arr_append` (shared `pkgs/transport`; all transport consumers) |

  Not a Pruv change: numeric timestamps (an integer or a digit string) give a 24-byte ISO stamp in Pruv (the reference
  behaviour) and none in the hand, whose benchmark inputs carry only canonical ISO strings (`hand_history.rs` header).
  The pump event cap counting non-line events is row UL.

## 3e. Step 4.4d: rows 4.3c-T, 4.3c-B, 4.3c-G and dead code

- 4.3c-T closed. `entry::at` is the hand `Entry::at` (timestamp, then `created_at` when the timestamp is missing or
  null): a canonical stamp returns its offset in the line, a numeric one formats 24 bytes into the caller's `iso`
  buffer, absent is -3, a defect -1. `feed` calls it only after the role, synthetic and blank filters (`user`) and
  for an assistant right before `answer2`. `at_view` lends the stamp (`ref str from(line, iso)`), so the canonical
  case copies nothing until `begin2` / `answer2` store it. `user_view` / `assistant_view` are gone.
- 4.3c-B partly closed. `user` picks the prompt span (query range or trimmed text), filters `<user_info>` and blank,
  then calls `begin2` once. Two run-time checks stay and are rows: 4.4d-CL (`feed` tests `len(e.content) <= 1e8`:
  `line::entry` does not ensure the content is bounded by the line) and 4.4d-CB (the query cut tests
  `is_char_boundary` on both ends: the verifier cannot derive a char boundary from the ASCII `<`/`>` byte next to
  it; `query_range` with that `ensures` fails).
- 4.3c-G closed in `pkgs/transport` `receive_grow`: when the LF is not in the chunk and the piece would pass the cap,
  the step fails (-3, the byte at the cap consumed) before any copy, as `Rpc::step` checks `LINE_CAP` before
  `extend`. Consumers (grok rpc, mcp, codex) stop on -3 as before. The dead `grow_line` is removed. The lazy line
  clear (`filled = 0`, truncate on the next append; the hand clears after `on_line`) stays: row 4.4d-LC.
- `pkgs/turns` dead code removed after a grep of every fragment: `begin3`, `answer3`, `put_nl_span`, `put_pad`,
  `write_span`. `finish2`, `finish_ref` and `empty_buffer` also have no caller, but comments and
  `../pkgs/RESHAPE.md` cite them as the reference forms; they stay as row 4.4d-TD.
- q mutants retargeted: M09 (`Kind::Other => { return 0; }`), M11 (tool count) and M08 (query trim) on the new code;
  every marker matches exactly once.

| row | Pruv | hand | change |
|---|---|---|---|
| 4.4d-CB | `user` checks `is_char_boundary` at `q.lo` / `q.hi` at run time | `&text[range]` on a range found at ASCII bytes | language: a char-boundary fact from an adjacent ASCII byte |
| 4.4d-CL | closed in 4.4z: `line::entry` ensures `len(e.content) <= len(s)`; the `feed` test is gone | no bound | done |
| 4.4d-LC | closed in 4.4v: `Reader.filled` removed; `pump` / `drain` clear `line` after the used line | `on_line` then `line.clear()` | done |
| 4.4d-TD | closed in 4.4v: `turns::finish2`, `finish_ref`, `empty_buffer` and the comments citing them removed | - | done |
| 4.4d-RC | closed in 4.4v: on -3 `transport::take` leaves `st.at` at the piece start | `Rpc::step` leaves the cursor at the piece start | done |
| 4.4d-PO | closed in 4.4v: `transport::receive` scans `chunk[at..end)` first and polls only when no line is complete; a fresh chunk is scanned in the same call | `Rpc::step` consumes the buffer, waits only when no line is complete | done |
| 4.4v-DR | closed in 4.4v (review): `drain` calls `transport::scan` (buffered bytes only), so a partial line ends it without a read or a poll | `drain` calls `step(false)` | done |
| 4.4v-PB | `pump` counts every `step` (partial chunks too) against 1000000 | `step(true)` waits inside one call | open; differs only past a million steps |
| 4.4v-TR | a ready chunk and the overall timer: `receive` sees the timer first | `recv_timeout` can take a queued chunk | open; only the race at the deadline |
| 4.4d-T0 | `feed` calls `turns::tool` only for `n > 0` | `self.tool(n)` for any `Some(n)` | same output (adds 0); needs `line::entry` to ensure `n >= 0` |

Codex structure comparison (one, gpt-6.2-astra high, Codex 2 account; the default account was at its limit). Applied:
`prompt_index` is read only in the no-query branch of `user` (hand `None =>` arm). Rows: 4.4d-RC, 4.4d-PO, 4.4d-T0 above.
Not changed: numeric timestamps (finding 1) are the reference behaviour, as in 3d.

Next (order kept in `../lib/RESHAPE.md` "Rows open after 4.4d"): Codex rows 4.3f-A, O, N, K, L, T, P, R, D, then
4.3f-B, E, F; then the OpenCode leftovers, 4.4b-SP first (checked against the TypeScript reference).

## 3f. Step 4.4z: row 4.4d-CL

- `line::entry` ensures `len(e.content) <= len(s)`: the content is decoded from the line (`decode_string` ensures
  the output grows by at most the bytes consumed) and joined inside it. `content`, `fields` and `part` carry
  `len(e.content) <= i` for the cursor `i`; `part_members` ensures its scratch is empty or `len(scratch) + 2 <= i - old(i)`,
  so the `"\n\n"` join is paid by the `{"k":` bytes before the text value.
- `entry::feed` requires `len(line) <= 1048576 && len(e.content) <= len(line)` (`read::line_step` already has both) and the
  `len(e.content) > 100000000` test is gone. The `part` proof guard (`len(e.content) + len(e.scratch) + 2 > 2^40`
  sets `ok = false`) is gone with it. No run-time branch was added.
- Codex structure comparison (gpt-6.2-astra, high, default account): the patch adds no branch; the three removed tests
  cannot fire on a line the scanner accepts. Older differences remain (members closure written as loops, the `ok`
  rebuild in `part`, 4.4d-T0, 4.4d-CB), so the structure difference is not zero and performance was not measured.
- Results (`rs/rust`, `rs/llvm`): `test_impl.py` 254/254 and 254/254 (verified, TypeScript reference); `q.py --cases 300`
  554 cases, 0 unexplained (450 exact, rest policy) on both; `q_mutants.py --check` 16/16 markers match.

## 4. Results

The tables below the 4.4d table are the 4.3c, 4.3b and 4.3 results.

4.4d, built from this worktree with `pruvc-rs` (`$PRUVC_RS`), verified (no `--no-verify`, 300 s cap per module),
after the last code change. `pkgs/transport` and `pkgs/turns` changed, so every consumer was rerun:

| test (4.4d) | rs/rust | rs/llvm |
|---|---|---|
| grok `test_impl.py --no-ts` | 254/254 | 254/254 |
| grok `q.py --cases 300` (554 cases with the fixtures) | 554, 0 unexplained (450 exact, rest policy) | 554, 0 unexplained (450 exact, rest policy) |
| grok `q_mutants.py --check` | 16/16 markers match | - |
| grok `perf/run_perf.py check` (output equality, no measurement) | 8/8 `pruv == hand` | 8/8 `pruv == hand` |
| mcp `test_impl.py` (transport) | 22/22 | 22/22 |
| codex `test_impl.py` (transport) | 136/136 | 136/136 |
| opencode `test_impl.py` (turns) | 199/199 | 199/199 |
| claude `test_impl.py` a0 / a1 / a2 / a3 / a4 (turns) | 272 / 2884 / 2443 / 134 / 270, all pass | same |

Performance was not remeasured.

4.3c, built from this worktree with `pruvc-rs` (`$PRUVC_RS`), verified (no `--no-verify`), after the last code change:

| test (4.3c) | rs/rust | rs/llvm |
|---|---|---|
| grok `test_impl.py --no-ts` | 254/254 (all fixtures passed on 2 pairs) | 254/254 |
| grok `q.py --cases 300` (554 cases with the fixtures) | 554, 0 unexplained (450 exact, rest policy) | 554, 0 unexplained (450 exact, rest policy) |
| grok `perf/run_perf.py check` (output equality, no measurement) | 8/8 `pruv == hand` | 8/8 `pruv == hand` |
| mcp `test_impl.py` (transport consumer) | 22/22 | 22/22 |
| codex `test_impl.py` (transport consumer; ts/oracle 136/136) | 136/136 | 136/136 |

`pkgs/turns` was not changed (gap VL), so opencode was not rerun. No q case needed a rerun. Performance was not remeasured.

4.3b, built from this worktree with `pruvc-rs` (`$PRUVC_RS`), verified (no `--no-verify`):

| test (4.3b) | rs/rust | rs/llvm |
|---|---|---|
| `test_impl.py --no-ts --jobs 6` | 254/254 | 254/254 |
| `q.py --cases 300 --jobs 4` (554 cases with the fixtures) | 554, 0 unexplained (449 exact, rest policy) | 554, 0 unexplained (450 exact, rest policy) |
| `perf/run_perf.py check` (output equality, no measurement) | 8/8 `pruv == hand` | 8/8 `pruv == hand` |

No q case needed a rerun. Performance was not remeasured.

Built from this worktree's `rpc.pruv` (verified, no `--no-verify`) with `pruvc-rs` (`$PRUVC_RS`); the full grok root
verified in 427 s for rs/rust and 115 s for rs/llvm (a shared `--proof-cache`, so the second run reused proofs).

| test | rs/rust | rs/llvm |
|---|---|---|
| `test_impl.py --no-ts --reuse-verified` | 254/254 | 254/254 |
| `q.py --cases 300` (554 cases with the fixtures, `--jobs 4`) | 551 explained, 3 unexplained | 551 explained, 3 unexplained |
| the 4 unexplained cases (q00198, q00200, q00207 load; q00220 copy) rerun one at a time, 3 times | 12/12 equal | 12/12 equal |
| `perf/run_perf.py check` (output equality, no measurement) | 8/8 `pruv == hand` | 8/8 `pruv == hand` |

The unexplained q cases were all "the fake ACP was started on one side only" or a `timeout` on a 3000 ms stall case, under
load average about 10 with four jobs; they differ between the pairs and all pass when run alone. `hand_rpc.rs` builds
without warnings (`rustc -O`). Performance was not remeasured.

Verification time: the first 4.3 form of `value` (sixteen separate `if slot == k` statements after the recursive call,
six before) did not finish in 150 minutes for the whole root (no output); the `rpc` module alone with that one function
changed to `else if` chains verifies in 15 s, and the whole module in 26 s. See gap VP.

## 4.4za: stderr

The hand `load_check` spawns with `.env("GROK_HOME", ..)` and `Stdio::null()`. Pruv has `proc_spawn_env` but no quiet
form of it, so `start` is unchanged and `transport::receive` keeps reading and dropping stderr (row 4.4za-EQ in
`../lib/RESHAPE.md`). The shared receive start is now `transport::begin` (no behaviour change).

L-gaps 16 closed 4.4za-EQ and 4.4za-QE: `start` spawns with `proc_spawn_env_quiet` (hand `.env(..).stderr(Stdio::null())`,
SPEC R-6l-4), and `transport::receive` no longer reads stream 2, which a quiet child has at end of file from the start.
The stderr drain is gone; like the hand, Pruv reads stdout only.
