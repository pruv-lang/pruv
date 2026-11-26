# Grok ACP load check and RPC lines: Rust-shape reset (RUST_SHAPE_RESET §6 step 4.3)

Scope: the whole of `rpc.pruv` (the per-line scan of DB65, `RESHAPE_LINE.md`, plus everything DB65 left on the tape and the
load-check driver) against `perf/hand_rpc.rs`, which is extended here with the load check (`load_check`, `Rpc`, `Check`)
so that every Pruv function has a hand counterpart. The history path (`perf/hand_history.rs`) is not in this step (4.3b,
`../lib/RESHAPE.md` "Next"). pruvc/, pruvc-rs/, SPEC.md and tests/ were not touched; `bench/l3/lib` and `bench/l3/pkgs`
are unchanged (no shared-module consumer runs needed).

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
| per line: trim, scan | `trim_bounds`, `scan` into `Cap`; escaped key in a tracked object or `c.err == 6` → whole-line tape parse | `trim_bounds`, `scan` into `Cap` for every line; `key_slot` → `key_is` decodes escaped keys while comparing | `trim_bounds`, `Sc::value` → `member` → `key_is` | 0 / 0; same |
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
| `trim_bounds` :111 | `trim_bounds` :66 | = | ASCII fast path; the dead `lo == i` branch of the back walk removed after the review |
| `Sc::ws` :181 | `ws` :126 | = | |
| `Sc::string` :192 | `sstr` :183, `stop` :143 | = / E1 | Pruv returns the span as `Val` instead of moving `self.p`; the end-of-line guard is explicit |
| `Sc::number` :243 | `num_end` :229, `digits` :212 | = | |
| `Sc::lit` :286 | `lit_end` :290 | = | |
| `Sc::value` / `array` / `object` :296-450 | `value` :368 | **open (4.3b)** / E1 / P | one container loop tests `obj` per element where the hand has separate `array` and `object` loops (row 4.3b-V below); depth 128 is `decreases d` (P); record cap and span guards E1; the hand's `match` on the slot is an `else if` chain (gap VP, §3) |
| `Sc::member` :364 | `key_slot` :342 | = | same track → slot table; the reset of `caps`/`ls` at `result` and of `ls` at `agentCapabilities` is in `value` where the slot is used |
| `key_is` :454 | `key_is` :301 | = | plain keys compare bytes; escaped keys decode `\uXXXX` and the one-letter escapes while comparing |
| `hex4` :497 | `hex4` :169, `hexv` :159 | = | |
| `unescape` :506 | `dec` :521, `put_cp` :508 | **open (4.3b)** | same decoding (lone surrogate → U+FFFD); but the caller zero-fills `buf` to the encoded length with `resize` and `dec` stores by index, byte by byte, where the hand clears and appends plain runs (row 4.3b-D) |
| `text_of` :555 | `text_span` :566 | E2 (B) | returns a span tagged line/`buf` because a function cannot return a borrowed slice; the caller (`fold_text`) slices either side |
| `emit` :570, `detail` :576 | `detail` :587 | = / E1 | `emit` is inlined (two `arr_push` sites); the `kind != STR` and span tests are one guard |
| `compact` :620 | `compact` :634, `hexd` :623 | = / E1 | output bound `len(out) <= 6 * (e - s)` and `e - s <= 8388608` (the line cap) are contracts only, `perf_root` tests the same bound before the call; after the review the closing quote is the hand's inline escape-skipping scan and the key test stops at the value end (before: a second `sstr` validation and a `ws` that could pass `e`); the `buf` growth is row 4.3b-D |
| `scan` :674 | `scan` :451, `valid` :448 | **open (4.3b)** | Pruv runs `utf8_valid` over the whole line first (row 4.3b-U); `Cap` also carries `lo`/`hi`/`root`/`rec` (row 4.3b-C) |
| `line` :685 | `perf_root.pruv` `rpc_line` | = | the error branch folds `detail` then `compact` into one reused `out` |
| `rpc_lines` :738 | `perf_root.pruv` `grok_rpc_lines` | = | |
| `Check`, `Fail` :772-788 | `Check` :721 | = / E2 | `fail` is an `i64` code (0 = None); `slot`, `timer`, `budget`, `ready`, `n` hold the loop handles the hand keeps in `Rpc` |
| `REQ_INIT` :790 | `req_init` :787 | E2 | gap L: no borrowed `[u8]` of a literal for `send` |
| `push_escaped` :793 | `push_escaped` :793 | = | |
| `req_load` :805 | `req_load` :812 | = | |
| `Rpc::send` :834 | `transport::send` | E2 | the G10 transport takes the bytes owned and applies the overall budget as the write deadline; a write failure half-closes stdin in both |
| `Rpc::step` :844 | `step` :762 | E2 (TH) | `receive_grow` on the event loop vs `recv_timeout` on the channel; the 8 MiB line cap, timeout (fail 5) and read failure (fail 6) map one to one |
| `Rpc::on_line` :887 | `fast_kind` :697 + the `fk` arms of `pump`/`drain` | = | |
| `Rpc::pump` :913 | `pump` :858 | = / P | the event cap counts loop events, not lines (UL) |
| `Rpc::drain` :936 | `drain` :894 | = / P | `n < 70000` cap (gap UL) |
| `on_reply` :946 | `on_reply` :833 | = / E1 | span guards on the subview; M16 marker kept |
| `load_check` :962, `Rpc::before_send` :929 | `load_check` :954, `start` :725, `before_send` :922 | E2 (TH) | spawn + loop registration is `start`; a failed start now returns before any buffer is made (as the hand, after the review); `before_send` is in the hand too now (see §3) |
| `failure` :1031 | `failure` :935 | = / E2 | takes `raw` and `detail` owned (after the review; before, `raw` was borrowed from `Check`); `LoadError` is written as the mode 3 packet |

## 3. Differences that remain

| difference | reason | row |
|---|---|---|
| the child is read on the host event loop, not by a reader thread over a channel | no threads or channels in the language | TH (open, `../lib/RESHAPE.md`) |
| `transport::send` takes the request bytes owned (`req_init()` allocates the literal) | `send` and the loop API take `[u8]` by value; no borrowed `[u8]` of a `str` literal | L (open) |
| pump/drain loops carry event caps (`1000000`, `70000`) | no unbounded loop | UL (open) |
| `load_check` returns the `[i64]` result packet | host boundary (§3 keep list) | — |
| `failure` takes `ck.detail` by moving the field out and refilling it with an empty array on the next statement | no struct destructuring; a moved field must be reassigned at once (an empty Vec does not allocate) | none (same cost) |
| `text_span` returns a tagged span (line or `buf`) instead of a slice | a function cannot return a borrowed slice into one of two arrays | B family (no new row) |
| the slot dispatch in `value` is an `else if` chain where the hand has `match` arms | an `i64` scrutinee takes no literal arms (R-6u-6 literals are payload positions only), and sixteen separate `if slot == k` statements (the first 4.3 form) made verification of `value` run for hours | VP (new, `../lib/RESHAPE.md`) |

Open structure rows (not language gaps; they can be written in the hand shape and are left for 4.3b, `../lib/RESHAPE.md`):

| row | Pruv | hand | change |
|---|---|---|---|
| 4.3b-U | `scan` runs `valid` (`utf8_valid` over the whole trimmed line) before the walk | UTF-8 checked per sequence inside `Sc::string` only | `stop` stops at bytes >= 128, `sstr` checks the sequence width (hand `utf8_width`), `valid` goes |
| 4.3b-V | `value` has one container loop that tests `obj` per element and calls `ws` twice after a comma | `array` and `object` are separate loops; `member` alone decides slots | split the loop into array and object helpers |
| 4.3b-C | `Cap` carries `lo`, `hi`, `root`, `rec`; consumers re-add `lo` | the caller trims, `rec` is scanner state, `Cap` holds the members | trim in the caller, pass the view, drop the four fields |
| 4.3b-D | `text_span`/`compact` `resize` `buf` to the encoded length (zero fill) and `dec` stores by index, byte by byte | `buf.clear()`, plain runs appended, decoded chars pushed | `buf = truncate(buf, 0)`; `dec` appends the plain run before each `\\` with `arr_append` and pushes decoded bytes |

New gap row: VP (verifier cost of independent `if` runs, `../lib/RESHAPE.md` step 4.3). No other new rows.

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
  counter (UL); `REQ_INIT` borrowed (L); the tagged `Span` (B family).

## 4. Results

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
