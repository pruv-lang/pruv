# Rust-shape reset step 2.1: shared packages

Seven packages (`json`, `lex`, `bytes`, `text`, `names`, `args`, `tail`) set against the hand Rust that does the
same job. Order of work: json, text, names, args, tail, then the lex and bytes checks. `lex` and `bytes` were
"keep" (structure check only). Gap ids refer to `docs/RUST_SHAPE_RESET.md` §4.

Both backends are a hard constraint: every consumer piece (mcp, qwen, codex, grok, opencode) also builds on LLVM,
where borrowed views and borrowing structs are `codegen/unsupported_type` (gap **B-LLVM**, debt D-LB3). So no package
here returns `ref str` / `ref [u8]` or stores one in a struct yet.

Columns: Pruv function, hand Rust function, types, ownership, loops, allocation sites. "=" means same as hand.

Rules (main correction, 2026-11-24): done means the Pruv code is structurally identical to the hand Rust, function by
function. A difference is allowed only when a proof removes a check hand Rust keeps. A function that cannot be written in
the hand shape because Pruv lacks a feature is left unchanged and gets a gap row; no workaround versions. Performance is
measured only for a package whose table shows zero differences.

## Status

| Package | Differences left | Measured |
|---|---|---|
| json | 2 (B-LLVM source copy, D32 publication checks; root tape copy TK) | no |
| text | 3 (`trim`, `clip_bytes`: B-LLVM; `one_line`: SR) | no |
| names | 1 (`project_name`: VC) | no |
| args | 1 (`build_args`: SR) | no |
| tail | whole package (hand is UTF-8 `Vec<u8>`; decoder pick: FM) | no |
| lex | 1 (`scan_string` block skip, kept by brief) | no |
| bytes | 0 | no (not run in this step) |

## json (`json/json.pruv`, hand `json/hand/hand_json.rs`)

| Pruv | Hand Rust | Types | Ownership | Loops | Allocations | Difference and reason |
|---|---|---|---|---|---|---|
| `kind_*()`, `err_*()`, `limit_*()` | `const` / literal kinds | `i64` | = | none | none | Pruv has no item `const`; each is a one-line fn with `ensures result == N`, inlined by both backends. Same code. |
| `struct JsonError { position, kind }` | `Err((usize, i64))` | `i64` pair | = | — | none | Named struct instead of tuple; same layout. |
| `struct JsonDocument { source: [u8], tape: [i64] }` | `Document<'a> { source: &'a [u8], tape: Vec<i64> }` | owned `[u8]` vs `&[u8]` | Pruv owns a copy of the source | — | Pruv: 1 extra (`copy(input)` in `parse::scan_document`) | **B-LLVM**: a borrowed field is Rust-backend only; consumers run on LLVM. Tape is moved (=). Source copy stays until LB3b borrowed views reach LLVM. |
| `enum JsonParse { Parsed, Failed }` | `Result<Vec<i64>, (usize,i64)>` | enum | = (moves) | — | none | = |
| `make_error`, `parse_failed` | `Err((pos, kind))` | — | = | none | none | Constructors with contracts; inlined. |
| `rec_ok`, `scan_ok`, `check_tape`, `check_subtrees` | none (hand trusts its parser) | `[i64]` read | borrow | per record | none | Proof-only publication checks (debt D32 in L3_RESTRUCTURE_SPEC §7.5). Not in the hand path; removal needs the parse-side proof (parse is step 2.2). Unchanged here. |

| root `Parsed(document) => copy(document.tape)` (`json/a2_root.pruv`) | `Ok(doc) => doc.tape` | `[i64]` | Pruv copies on LLVM | — | LLVM: 1 tape-sized copy | **TK**. The Rust backend already emits `return document.tape`. |

No source change in `json.pruv` this step. Not measured: differences remain (the earlier step 2.1 measurement was taken
before this rule and is withdrawn, along with its debt rows).

## text (`text/text.pruv`, hand: `str` methods)

| Pruv | Hand Rust | Types | Ownership | Loops | Allocations | Difference and reason |
|---|---|---|---|---|---|---|
| `space_len(s, i)` | `is_es_ws(c)` (`qwen/handrust/src/lib.rs:312`, ECMAScript whitespace set) on the char at `i` | `str`, `i64` | borrow | none | none | = |
| `next`, `prev` (private) | `s[i..].chars().next().len_utf8()` | `str` | borrow | ≤4 steps | none | = |
| `is_blank(s)` | `s.trim().is_empty()` | `str -> bool` | borrow | one pass | none | = |
| `trim_span(s) -> Span` | `s.trim()` bounds | `str -> {lo,hi}` | borrow | two passes | none | = (span form of trim) |
| `trim(s) -> str` | `s.trim() -> &str` | `str -> str` | Pruv returns an owned copy | — | Pruv: 1 (`substr`) | **B-LLVM** (borrowed result). Callers that only need bounds use `trim_span`. |
| `same(a, b)` | `a == b` | `str` | borrow | one pass | none | = |
| `one_line(s, max) -> str` | `qwen/handrust/src/lib.rs:325`: `for c in s.trim_matches(is_es_ws).chars()`, `out.push(c)` per scalar, `push(' ')` per whitespace run | `str -> String` | owned result (=) | one pass (=) | hand: `out` growth only; Pruv: plus one `substr` temporary per scalar | Unchanged (HEAD). **SR**: Pruv has no scalar push, and neither `subview` nor a scalar can be the argument of `string_push_str`. A wrapper fn would differ from hand, so none is used. |
| `clip_bytes(s, max) -> str` | `&s[..floor_char_boundary(max)]` | `str -> str` | Pruv returns an owned copy | ≤3 steps back | Pruv: 1 (`substr`) | **B-LLVM** (borrowed result). |
| `engine_line(text, task_origin)` | `let t = text.trim(); t == "[Request…]" \|\| …` | `str -> bool` | borrow | `eq_at` per literal | **changed**: the literals are written inline at each comparison like hand (`t == "…"`), so no `let a: str = "…"` local (which emits `String::from`, gap SL) | = now. `! alloc` kept on the signature for caller stability. |
| `dropped_user` and helpers (`ws_at`, `skip_ws`, `lit_end`, `quoted_end`, `colon_value`, `kv`, `tool_result_after`, `content_hit`, `eq_at`) | byte scans over `as_bytes()` | `[u8]`/`str` | borrow | one pass | none (literal arguments are `&'static str`) | = . The `! alloc` marks are stale but kept for caller stability. |

## names (`names/names.pruv`, hand: Lair `projectName` / `validThread` in Rust)

| Pruv | Hand Rust | Types | Ownership | Loops | Allocations | Difference and reason |
|---|---|---|---|---|---|---|
| `alnum(x)` | `u8::is_ascii_alphanumeric` | `i64 -> bool` | — | none | none | = |
| `project_name(data) -> [u8]` | `let mut out = Vec::with_capacity(n); for b in data { out.push(map(b)) }` | `[u8] -> [u8]` | owned result | one pass + copy loop | hand 1; Pruv 2 (scratch + exact copy) | Unchanged (HEAD). **VC**: no reserve-then-push `[u8]` (`alloc` fills; `truncate` adds a conditional shrink realloc, which the confirm audit showed returns a 2x buffer on some inputs). |
| `valid_thread(s) -> bool` | `bytes().enumerate().all(..)` with early exit | `str -> bool` | borrow | one pass, early return | none | **changed**: early `return false` instead of an `ok` flag. = |

## args (`args/args.pruv`, hand: `s.split_terminator('\0')` into `Command::args`)

| Pruv | Hand Rust | Types | Ownership | Loops | Allocations | Difference and reason |
|---|---|---|---|---|---|---|
| `arg_spans(argv) -> [i64]` | `argv.is_empty() \|\| argv.ends_with('\0')` check, no spans | `str -> [i64]` | owned packet | one pass | Pruv 1-2; hand 0 | Unchanged (HEAD). Exists only because `build_args` cannot push from the blob (SR). |
| `build_args(argv) -> ProcessArgs` | `for p in argv.split_terminator('\0') { cmd.arg(p) }` | `str -> ProcessArgs` | owned result | one pass | hand 1 per argument (inside `arg`); Pruv span packet + `substr` + `String::from` per argument | Unchanged (HEAD). **SR**: `proc_args_push` takes no `subview`; a wrapper fn would differ from hand. |

Callers (mcp `root.pruv`, codex `rpc.pruv` `child_args`) unchanged.

## tail (`tail/tail.pruv`, hand: `supervisor/hand/core_hand.rs`)

Unchanged (HEAD). Hand keeps the tail as UTF-8 `Vec<u8>` (`Vec::with_capacity(6 * limit + 4)`, `trim` drops whole
scalars from the front by lead byte, `finish(self)` returns the same vector). Pruv keeps a ring of UTF-16 units in two
loose arrays (`tb`, 12-word `ts` packet), with `tail_bytes` re-encoding into a new buffer. Every function differs. The
rewrite to the hand shape is open; its decoder pick needs `let d = if s == 1 { &mut tl.d1 } else { &mut tl.d2 }`,
which Pruv cannot express (gap **FM**). A first attempt that grouped the state into `Tail` / `Decoder` structs but kept
UTF-16 units and copied the chosen decoder in and out was withdrawn as a non-hand shape.

## lex (`lex/lex.pruv`, hand: `json/hand/hand_json.rs` scanner) — keep

| Pruv | Hand Rust | Types | Ownership | Loops | Allocations | Difference and reason |
|---|---|---|---|---|---|---|
| `struct Scan { position, kind }` | `Result<usize, (usize,i64)>` | `i64` pair | = | — | none | Named struct; same layout. |
| `validate_utf8(input, start, end) -> i64` | `std::str::from_utf8(&input[a..b])` | `[u8]` | borrow | 16-byte ASCII blocks, then per char | none | Same algorithm as std (ASCII fast path, then sequence check); returns the error offset like `valid_up_to`. = |
| `skip_whitespace` | `while matches!(b, b' '\|b'\t'\|b'\n'\|b'\r')` | `[u8]` | borrow | one pass | none | = |
| `scan_literal` | `input[p..].starts_with(b"true")` etc. | `[u8]` | borrow | ≤5 steps | none | Blames the first mismatching byte (A0 error rule), = otherwise. |
| `digits`, `scan_number` | hand number scanner | `[u8]` | borrow | one pass | none | = |
| `scan_string` | `hand_json.rs:113` `string()`: `while let Some(byte) = self.peek()` per byte | `[u8]` | borrow | Pruv: 16-byte plain-byte blocks, then per byte; hand: per byte | none | **Differs** (loop shape, not a gap): Pruv has an extra block skip. Kept because lex is a keep package in this step; not a proof-only difference. |

No source change (keep package). One loop difference left (`scan_string`).

## bytes (`bytes/bytes.pruv`) — keep, one fix

| Pruv | Hand Rust | Types | Ownership | Loops | Allocations | Difference and reason |
|---|---|---|---|---|---|---|
| `copy_span(inout dst, at, src, lo, hi)` | `dst[at..at+n].copy_from_slice(&src[lo..hi])` | `inout [u8]`, `[u8]` | `dst` borrowed mutably (=) | **changed**: was a per-byte loop with three quantified invariants; now one `copy_range` (Rust backend: `copy_from_slice`; LLVM: `llvm.memcpy`) | none | = now. Also removes the loop invariant that timed out in the Python verifier (G-DB55-2). |

## Gap rows found in this step

Added to `docs/RUST_SHAPE_RESET.md` §4. Each function hit by a gap is left unchanged.

| Id | Hand Rust form | What Pruv cannot express | Functions left unchanged |
|---|---|---|---|
| B-LLVM | `Document<'a> { source: &'a [u8] }`, `fn trim(&str) -> &str` | Borrowed fields and borrowed results are Rust-backend only (`codegen/unsupported_type` on LLVM, debt D-LB3); every consumer also builds on LLVM. | `json` source copy, `text::trim`, `text::clip_bytes` |
| SR | `out.push(c)`, `out.push_str(&s[i..j])`, `cmd.arg(&s[i..j])` | No scalar push; `subview(s,lo,hi)` cannot be the direct argument of `string_push_str` / `proc_args_push` (only of a plain `str` parameter of a Pruv fn, R-6g-10). | `text::one_line`, `args::build_args` (+ `arg_spans`) |
| VC | `Vec::with_capacity(n)` then `push` | No reserve-without-fill `[u8]`; `alloc` fills and `truncate` may shrink-realloc (LB5 non-filling growth). | `names::project_name` |
| FM | `let d = if c { &mut x.a } else { &mut x.b };` | No `inout` binding to one of two fields chosen at run time. | `tail` rewrite (blocked part: decoder pick) |
| TK | `Ok(doc) => doc.tape` | The tape cannot be moved out of a match binder: `let mut` clones, `move binder` of a struct is `type/foreign_move`, a `take` record parameter is refused for a struct with an `[i64]` field. The Rust backend emits a move anyway; LLVM emits `@pruv_copy`. | `json/a2_root.pruv` root |
| SL | `let a = "lit";` (`&'static str`) | `let a: str = "lit"` emits `String::from`. Not needed in `text::engine_line` (hand writes the literal inline, and so does Pruv now). | — (other pieces: codex, claude, grok `let none: str = ""`) |

# Step 2.2: jstr, parse, turns, transport

Same rule and columns as step 2.1. Hand counterparts: `json/hand/hand_json.rs` (`parse`, `Parser`, `units` at 335,
query modes 3/5/6 at 397-430) for `parse` and `jstr`; the `Collector` in `qwen/handrust/src/lib.rs:158`,
`claude/handrust/src/lib.rs:618` and `grok/perf/hand_history.rs:705` for `turns`; `supervisor/hand/supervise_hand.rs`
`read_pipe` (64 KiB chunk, `buf[..n]`) and `codex/hand/reader.rs` `Rpc` (`BufReader::read_line` into a reused
`String`) for `transport`; `str_adapter/src/lib.rs:2-3` (`to_owned`, empty `String` on invalid UTF-8) for the
`jstr` text adapters. No performance was measured.

Revised after an independent review against the hand files (delegation `5ecff70e`): the first version of this
table marked 21 of 23 rows wrongly (`=` where the shapes differ, hand functions that do not exist, VC claimed after
LB5 closed it). Every row below now carries an id.

Status ids used here, besides the §4 gap ids:

- **=**: same shape as the hand function named in the row.
- **W**: writable in hand shape with current Pruv (LB5 `arr_reserve`/`arr_push`/`arr_append`, recursion with
  `decreases`), rewrite pending. Not a gap; listed so the next step picks it up.
- **W-c**: writable, but the change is in the consumer pieces' API (mcp, codex, grok, opencode), so it is done in a
  consumer step, not in this package.
- **P**: Pruv-only proof or API support with no hand counterpart (kept on purpose, reason in the row).

VC is closed: LB5 added non-filling growth (SPEC R-6g4-1/2/3/6). The step 2.2 rows that blamed VC are W.

## Status (function level)

| Package | = | Changed in 2.2 | W | W-c | Gap id | P |
|---|---|---|---|---|---|---|
| jstr | J2, J2b, J3 (valid input of J6) | J3 `string_units` | J1, J4 | J5 copy variants | SL/S (J6), U (J8) | J5 `adapter_probe`, J6/J7 `InternalInvariant` |
| parse | P0 | P0 (first pass) | P1-P6 | P8 | B-LLVM (P7) | P2 plain bit, P9 |
| turns | T3 (operation only) | — | T1, T2, T4, T5, T6 growth, T8 | T7, T9, T10 | ABI / V (T0, T6) | — |
| transport | X0 | X0 (first pass) | X3, X4, X5 | X1, X2, X6 | C (X3), FM (X7) | — |

## jstr (`jstr/jstr.pruv`)

| # | Pruv | Hand Rust | Types | Ownership | Loops | Allocations | Id and reason |
|---|---|---|---|---|---|---|---|
| J1 | `hex`, `step`, `scalar`, `width` | `units(raw)` iterator (`hand_json.rs:335`); `hex4` exists only in `claude/handrust:333` and `grok/perf:309` | `[u8]`, `i64` | borrow | one escape or scalar per call | none | **W**: the hand has one stateful iterator; Pruv has free decode helpers returning a `Step` record. Same decode work per unit; an iterator-shaped `Step` cursor is writable. No hand function is one-to-one with these four. |
| J2 | `ascii_equal(source,s,e,key)` | query mode 3: `text.is_ascii() && !body.contains('\\')` then slice `==`, else `units(raw).eq(text.encode_utf16())` | `[u8]`, `str` | borrow | one pass, early exit | none | **=** for the ASCII path. Precondition (not in the hand): `key` is ASCII; every caller checks `is_ascii(key)` first (`access.pruv:122`, `:212-217`, `:311`, `json/b_root.pruv:144`) and non-ASCII keys go through `units_equal`. Documented, not a behaviour bug (see note J2 below). |
| J2b | `units_equal(source,s,e,key,klen)` | `units(raw).eq(key)` | `[u8]`, `[i64]` | borrow | one pass, early exit | none | **=** (slow path of mode 3; key already UTF-16). |
| J3 | `string_units(source,s,e) -> [i64]` | mode 5: `units(raw).count()`, `Vec::with_capacity`, `extend(units(raw))` | `[u8] -> [i64]` | owned result | count pass, then decode + push pass | 1 exact (was 2: `alloc(e-s)` + exact copy) | **changed** (LB5 `arr_reserve` + `arr_push`). = now. |
| J4 | `string_utf8`, `string_utf8_plain`, `string_utf8_scanned` | mode 6: `decode_utf16(units(raw))` length sum, then `encode_utf8` per char into `Vec<i64>` | Pruv `[u8]`, hand `Vec<i64>` | owned result | Pruv: 16-byte block scan, count pass, fill pass; hand: two decode passes | 1 each | **W**: reserve-then-push is writable now (was VC). The element type differs on purpose: consumers take `[u8]` bytes, the hand benchmark prints `i64` per byte; the plain-body `copy_range` matches the hand `extend_from_slice`. Rewrite pending (grok consumes `_scanned`, tested in the grok step). |
| J5 | `string_text`, `string_text_scanned`, `string_text_plain`, `utf8_to_string`, `adapter_probe` | `String::from_utf8(vec)` (move); adapter `str_adapter` `to_owned` | `[u8] -> Text` | **copy** into a new `String` | validate + copy | Pruv 2 (bytes, then `String`); hand 1 | **W-c**: the `*_owned` variants are the hand shape; the copy variants stay until consumers switch (consumer steps). `adapter_probe` is **P** (adapter conformance probe, no hand counterpart). |
| J6 | `string_text_owned`, `*_scanned_owned`, `*_plain_owned`, `owned_text(take data)` | `String::from_utf8(vec)` / adapter: empty `String` on invalid input | `[u8] -> str` / `Text` | move | validate pass | 1 | **=** for valid input. Two Pruv-only branches: `owned_text` returns `""` on invalid UTF-8 (the `str_adapter` contract, builds an empty `String`: **SL/S**), and `*_owned` return `Text::InternalInvariant` when `utf8_valid` fails (**P**: a returned value the caller handles, not a panic; unreachable on real paths, see note J6). |
| J7 | `Text` enum (`Plain`/`InternalInvariant`) | `Result<String, _>` / `unwrap` | enum | — | — | none | **P**: the verifier cannot yet prove the parse-validated bytes are UTF-8 at this call, so the branch is a value, not `unreachable!()`. |
| J8 | UTF-16 key views (`[i64]` units) | `&[u16]` / `encode_utf16()` | `[i64]` vs `[u16]` | — | — | — | **U** (§4): no first-class UTF-16 view; `[i64]` holds one unit per word. |

## parse (`parse/parse.pruv`)

| # | Pruv | Hand Rust | Types | Ownership | Loops | Allocations | Id and reason |
|---|---|---|---|---|---|---|---|
| P0 | `parse(input)` error path | `hand_json.rs:243`: on a grammar error (kind != 4) `from_utf8(&input[position..])` | `[u8]` | borrow | one rescan of the rest | none | **=** (changed in 2.2 first pass: was a full rescan on every failure). |
| P1 | `Token`/`token_result`/`token_at`/`token_at_span` packets | `Parser { pos }` + `Result<_, Error>` returned from each method | record vs `&mut self` + `Result` | borrow | — | none | **W**: a recursive descent with `inout pos: i64` and a `Result`-like enum is writable (recursion needs `decreases`, SPEC §6e). Pending with P6. |
| P2 | `utf8_width`, `string_token` | `Parser::string`: per byte, `from_utf8` on the slice | `[u8]` | borrow | Pruv: 16-byte block skip, each block read up to 4 times, plain bit tracked; hand: one byte loop | none | **W** (loop shape): the block skip is a Pruv speed choice, same as `lex::scan_string` (step 2.1); a single-read block loop is writable. The plain bit is **P** (feeds `string_utf8_plain`). |
| P3 | `write_record`, `close_record` | `Parser::record`: `tape.extend_from_slice(&[k, s, e, parent, 0, 0])` | `[i64]` | `inout` (= `&mut Vec`) | — | none | **W**: Pruv writes 6 words by record id into a pre-sized tape; the hand pushes. Push form is `arr_push`/`arr_append` now. The sixth word's initial value differs by path (0, a negative frame number, or 1) and is closed later; no hand counterpart for that encoding. |
| P4 | `comma_state`, `initial_state`, `value_state`, `value_increment`, internal error 18 | control flow of the recursive methods | `i64` state | — | — | none | **W** (follows from P6): explicit state integers stand in for the call stack; error 18 is an internal-state error the hand cannot reach. Disappears with a recursive rewrite. |
| P5 | `first_slots`, `widened`, `regrown` | `Vec::with_capacity(32*WIDTH)`, `tape.reserve` (realloc growth) | `[i64]` | owned | copy loop | new buffer + copy | **W** (was VC): `arr_reserve` growth is writable. `regrown` grows the frame stack, not the tape (the first table said tape); the hand stack is the Rust call stack. |
| P6 | `scan`, `scan_document` | `Parser::record` recursive descent (`container`/`string`/`number`/`literal`) | `[u8] -> JsonParse` | borrow input | Pruv: two flat loops (validate-only `scan`, tape-building `scan_document`), explicit stack starting at 8 frames; hand: one recursion | tape (+ stack) | **W**: iterative vs recursive descent. Recursion is allowed with `decreases` on every function in the cycle; nothing blocks the hand shape except the borrowed `Parser<'a>` field (B-LLVM, P7 below), which an `inout pos` / `input` parameter replaces. Rewrite pending (largest item of this step). |
| P7 | `Parsed(JsonDocument{ source: copy(input), .. })` | `Document<'a>{ source: &'a [u8] }` | `[u8]` | **copy** | — | +1 | **B-LLVM** (same row as json step 2.1). Unchanged. |
| P8 | `scan_into`, `span_reject`, `done` (`Pass`) | none (hand parses each line with `parse`) | `[i64]` scratch | `inout tape`, `inout stack` | — | none | **W-c**: reusable-scratch API used by `lines`/claude entries; the hand allocates per call. Kept until those consumers are reshaped. |
| P9 | `failure`, `publication_ok` (`json::check_subtrees`) | none | `[i64]` | borrow | per record, after a successful parse | none | **P**: D32 proof-only publication check; `scan_document` does not yet prove the tape shape. Suffix-UTF-8 policy on failure matches the hand (P0). |

## turns (`turns/turns.pruv`)

The hand `Collector` keeps `VecDeque<Turn>` with `Turn { prompt: String, reply: String, created_at: Option<String>, .. }`
and returns `Vec<Turn>`. The whole package stays on word packets because of **ABI / V** (owned struct collections
across the host boundary on both backends). Rows below list what differs inside that constraint.

| # | Pruv | Hand Rust | Types | Ownership | Loops | Allocations | Id and reason |
|---|---|---|---|---|---|---|---|
| T0 | package API: `st[4]`, working slots `sl[200*8]`, `buf` 65584 bytes per slot, `Collected{turns:[i64],text:[u8],..}` | `Collector { turns: VecDeque<Turn> }`, `finish -> Vec<Turn>` | packets vs `Vec<struct>` | — | — | slabs vs per-turn `String` | **ABI / V**. The 200×8 table is the working slot array `sl` (first table said `Collected`). The modular ring (`% 200`) matches the Grok collector's bounded deque. |
| T1 | `new_state`, `new_slots`, `new_buffer`, `empty_buffer`, `grow` | `VecDeque::new()`, per-turn `String::new()` | packets | owned | fill loops | up-front slabs; `grow` = new buffer + copy | **W**: `grow` can be `arr_reserve` (was refill + copy). The up-front slabs follow from T0. |
| T2 | `put_str`, `put_nl`, `write_piece`, `begin` clip | `prompt.push_str(text)`, `clip` on the `&str` | `[u8]` | `inout` | Pruv per byte; `begin` clips through a temporary string | temp string in `begin` | **W**: bulk copy (`copy_range`/`arr_append`) and clipping on the byte range without the temporary are writable. |
| T3 | `skip_user`, `tool` | `skip_user`, `tool` methods (flag / count updates) | `inout st`, `sl` | — | — | none | **=** in operation; shape is ABI / V (T0). |
| T4 | `plan` (203-word offset array) | `turns.iter().rev()` with a running byte total | `[i64]` | owned | one pass | one 203-word array | **W**: the offset array is an extra allocation; a two-pass sum without it is writable. |
| T5 | `finish` | `Collector::finish(self) -> Vec<Turn>` (moves) | `Collected` | **copy** of `turns`/`text` at both returns (`turns.pruv:356`, `:391`) | — | +2 copies | **W**: `move turns, move text` as `finish2` does. Only caller `opencode/convert.pruv:211`; not changed in this step so the running opencode tests stay valid. |
| T6 | `new_store`, `new_store_hint`, `reserve`, `move_bytes` (`Store` arena) | per-turn `String` allocation | `Store` | `inout` | compaction loop | arena + compaction | **ABI / V** consequence: one byte arena instead of owned `String`s. Growth inside `reserve` is **W** (`arr_reserve`). |
| T7 | `begin2`, `answer2`, `begin3`, `answer3`, `finish2` | same methods | packets + `Store` | `inout` | per byte (2) / span copy (3) | arena | **W-c**: store-backed variants for claude/qwen/grok; the per-byte copies in `*2` are W (as T2). |
| T8 | `clip_span`, `put_nl_span`, `put_pad`, `write_span` | `clip` + `push_str` on a borrowed slice | `[u8]` | borrow / `inout` | per byte | none | **W**: bulk copy writable (as T2). |
| T9 | `new_refs`, `begin_ref`, `answer_ref` | none (hand owns `String`s) | `[i64]` refs | `inout` | — | one refs array | **W-c**: reference-only variant for a zero-copy consumer; no hand counterpart (hand copies). |
| T10 | `text_layout`, `finish_into`, `finish_ref`, `finish_inplace` | `finish` then the consumer serialises `Vec<Turn>` | `Placed` | `inout wire` | per turn | none / turns array | **W-c**: writes straight into the consumer's output; the hand builds `Vec<Turn>` first. Consumer API. |

## transport (`transport/transport.pruv`)

| # | Pruv | Hand Rust | Types | Ownership | Loops | Allocations | Id and reason |
|---|---|---|---|---|---|---|---|
| X0 | `receive_grow` chunk read | `read_pipe`: `pipe.read(&mut buf)` into `[0; 65536]`, `buf[..n]` | `[u8]` | `inout chunk` | copy | none | **=** (changed in 2.2 first pass: per-byte copy → one `copy_range`). |
| X1 | `live`, `stdin_closed`, `send` | `child.stdin.write_all` on a writer thread | `Loop` moved through | move | — | none | **W-c**: event-loop handle instead of threads. `send` takes a deadline `ms` with no hand counterpart (hand blocks). Shared API of mcp, codex, grok. |
| X2 | `close`, `poll`, `expired` | `child.kill`/`wait`, `recv_timeout` | `Loop`, `inout st` | move | `expired`: at most 8 polls | none | **W-c** (same API as X1). The first table's `=` was wrong: the hand has no poll/expiry loop. |
| X3 | `grow_line(line, cap)` | `read_line` into a reused `String` (`clear`, capacity kept) | `[u8]` | owned | copy | new buffer + `copy_range` | **W** (was VC): `arr_reserve` on the line. The first table's `line.reserve(1)` is not in the hand; the hand reuses one `String`. Keeping capacity across lines is **C** (§4). |
| X4 | `receive_grow` event bytes | `line.as_bytes().to_vec()` once | `[u8]` | owned copy of `ev_bytes`, then `copy_range` | — | Pruv 2, hand 1 | **W**: one copy is writable. |
| X5 | `receive_grow` framing | `BufReader::read_line` (newline kept, partial last line returned at EOF) | `[u8]` | `inout line` | Pruv per byte; hand bulk | growth only | **W** (loop shape; bulk scan + `arr_append` writable). Framing semantics differ on purpose: Pruv excludes the newline and drops/flags a partial last line at EOF per the `st` status; consumers rely on that (W-c to change). The first table's `memchr`/`Reader` struct are not in the hand. |
| X6 | `st: [i64]` 5-slot state | locals of the reader thread | packet | `inout` | — | none | **W-c**: API of mcp/codex/grok; consumer step. |
| X7 | stderr `tail_push(inout tb, inout ts, ..)` | ring tail per stream | packets | `inout` | — | none | **FM** (step 2.1 `tail` row). Unchanged. |

## Notes on review findings judged not to be bugs

- J2: the false result for a quoted `"é"` against key `é` needs a non-ASCII key in `ascii_equal`; no caller passes one
  (all check `is_ascii` first, non-ASCII keys go to `units_equal`). Not reached; recorded as a precondition, no fix.
- J6: `owned_text([0xff])` returning `""` is the `str_adapter` contract (empty `String` on invalid UTF-8), not a
  wrong branch. `*_owned` can return `InternalInvariant` only on invalid UTF-8, but every `JsonDocument` is built by
  `parse`/`scan_into` (`parse.pruv:428`, `lines.pruv:57,86,154`, `claude/entry.pruv:100`), whose `string_token`
  validates UTF-8 inside strings. Not reached; the first table's "same effect class as a hand panic" was wrong (it is a
  returned value) and is corrected in J6/J7.
- Row "`string_utf8` count-then-fill is VC": wrong after LB5; now J4 **W**.
- Rows citing `hex4`, `line.reserve(1)`, `struct Reader`, `memchr`: those hand functions do not exist in the hand files
  named; rows J1, X3, X5 now cite the real hand code.

## Gap rows found in step 2.2

No new gap ids. One new status (W / W-c, defined above). VC is closed by LB5. Gap ids that remain:

| Id | Hand Rust form | What Pruv cannot express | Functions left unchanged |
|---|---|---|---|
| B-LLVM | `Document<'a> { source: &'a [u8] }` | borrowed fields on LLVM | `parse` source copy (P7) |
| ABI / V | `Collector { turns: VecDeque<Turn> }` returning `Vec<Turn>` | owned struct collections across the host boundary | `turns` package API (T0, T6) |
| U | `&[u16]` keys | first-class UTF-16 view | `jstr` key arrays (J8) |
| SL / S | `""` literal never allocated | `""` in a `str` result emits `String::from` | `jstr::owned_text` invalid-input branch (J6) |
| C | reused `String` keeps capacity | capacity-keeping clear | `transport` line reuse (X3) |
| FM | ring tail picked by stream | `inout` binding to a run-time chosen field | `transport` stderr tail (X7) |
