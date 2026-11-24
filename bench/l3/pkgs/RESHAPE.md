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
| names | 0 (`project_name` reshaped in step 2.2b) | no |
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
| `project_name(data) -> [u8]` | `let mut out = Vec::with_capacity(n); for b in data { out.push(map(b)) }` | `[u8] -> [u8]` | owned result | one pass | 1 (`alloc(0,0)` + `arr_reserve(len(data))`, then `arr_push`) | **changed in 2.2b** (LB5 closed VC). = |
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

Step 2.2b (this revision) rewrote every **W** row toward **=**, **P**, **W-c** or a gap id. Two new gap ids
(**IF**, **SB**, `docs/RUST_SHAPE_RESET.md` §4) came out of it; each was checked with a minimal file against
`pruvc-rs` before the row was written. No performance was measured yet.

**Closed in 2.2c.** The 2.2b audit still read eight function-level differences as **W** (J4, P1, P3, P5, P6, X5,
T2 `answer`, T7 `begin2`/`answer2`). Step 2.2c reshaped each one at the function level. Where the hand Rust had the
worse shape, the hand Rust changed too:
- J4: the hand now copies escape-free runs.
- P5: the hand tape now grows inside the record append.

Four rows keep a residue that Pruv cannot express today. Each residue is a §4 gap id, not a **W**:
- **RU** (P6)
- **OS** (P6)
- **SX** (P3)
- **MC** (X5)

RU, OS and SX were checked with a minimal file against `pruvc-rs`, and MC against the std list in SPEC. Because gaps remain, no performance was measured.

## Status (function level)

| Package | = | Changed in 2.2 | W | W-c | Gap id | P |
|---|---|---|---|---|---|---|
| jstr | J2, J2b, J3, J4 | J3 `string_units`; 2.2b/2.2c: J4 | — | J5 copy variants | B (J1), SL/S (J6), U (J8) | J5 `adapter_probe`, J6/J7 `InternalInvariant` |
| parse | P0, P1, P2, P4, P5 | P0 (first pass); 2.2b: P1-P6; 2.2c: P1, P3, P5, P6 | — | P8 | B-LLVM (P7), SX (P3), RU/OS (P6) | P2 plain bit, P9 |
| turns | T3 (operation only), T4, T5, T8, T2 `put_nl`/`begin`/`answer` | 2.2b: T2 (part), T4, T5, T8; 2.2c: T2 `answer`, T7 | — | T1 `grow`, T7, T9, T10 | ABI / V (T0, T1 slabs, T6), IF (T6 growth), SB (T2 `put_str`/`write_piece`) | T1 `empty_buffer` |
| transport | X0, X3, X4 | X0 (first pass); 2.2b: X3, X4, X5; 2.2c: X5 growth | — | X1, X2, X6 | FM (X7), MC (X5 scan) | — |

## jstr (`jstr/jstr.pruv`)

| # | Pruv | Hand Rust | Types | Ownership | Loops | Allocations | Id and reason |
|---|---|---|---|---|---|---|---|
| J1 | `hex`, `step`, `scalar`, `width` | `units(raw)` iterator (`hand_json.rs:335`); `hex4` exists only in `claude/handrust:333` and `grok/perf:309` | `[u8]`, `i64` | borrow | one escape or scalar per call | none | **B** (§4): the hand-shape cursor is a struct holding the borrowed bytes (`struct Units<'a> { raw: &'a [u8], pos }`); a Pruv struct with a `ref [u8]` field is `type/borrow_signature` ("borrowed type or from-list is not allowed here"). Left unchanged (free helpers returning `Step`, same decode work per unit). |
| J2 | `ascii_equal(source,s,e,key)` | query mode 3: `text.is_ascii() && !body.contains('\\')` then slice `==`, else `units(raw).eq(text.encode_utf16())` | `[u8]`, `str` | borrow | one pass, early exit | none | **=** for the ASCII path. Precondition (not in the hand): `key` is ASCII; every caller checks `is_ascii(key)` first (`access.pruv:122`, `:212-217`, `:311`, `json/b_root.pruv:144`) and non-ASCII keys go through `units_equal`. Documented, not a behaviour bug (see note J2 below). |
| J2b | `units_equal(source,s,e,key,klen)` | `units(raw).eq(key)` | `[u8]`, `[i64]` | borrow | one pass, early exit | none | **=** (slow path of mode 3; key already UTF-16). |
| J3 | `string_units(source,s,e) -> [i64]` | mode 5: `units(raw).count()`, `Vec::with_capacity`, `extend(units(raw))` | `[u8] -> [i64]` | owned result | count pass, then decode + push pass | 1 exact (was 2: `alloc(e-s)` + exact copy) | **changed** (LB5 `arr_reserve` + `arr_push`). = now. |
| J4 | `string_utf8`, `string_utf8_plain`, `string_utf8_scanned` | mode 6: `decode_utf16(units(raw))` length sum, then `encode_utf8` per char into `Vec<i64>` | Pruv `[u8]`, hand `Vec<i64>` | owned result | count pass, then `alloc(0,0)` + `arr_reserve(need)` and a decode pass (one shared private `decode_utf8`). Both passes take each escape-free run (up to the next `\\`; in `_utf8` also up to the next byte >= 128) as one segment: the count adds its length, the decode does one `arr_append` (`extend_from_slice`). Escapes are decoded and pushed per encoded byte. `_plain`: `arr_reserve` + one `arr_append` | 1 each | **changed in 2.2c**. The hand changed to the same segments: `utf8_segments` hands each escape-free run to one `extend` and decodes each escape as it meets it, in both the count pass and the copy pass. Escapes: both sides decode one scalar per step (a high surrogate takes the next `\\u` when it is the low half, else U+FFFD) with no look-ahead scan; Pruv pushes its 1-4 bytes, the hand `encode_utf8`s into a 4-byte stack buffer. Neither side allocates (=; the hand's earlier escape-run pre-scan was removed after the 2.2c confirmation). **P**: `string_utf8` (unscanned input) also ends a run at a byte >= 128 and decodes that scalar with `step` (lenient: a lone lead byte keeps its value and continuation bits are not checked), because nothing proves its input is UTF-8. The hand only sees parser-validated input, so its counterpart is `string_utf8_scanned`, whose runs end only at `\\`. = The element type differs on purpose (**W-c** in the hand benchmark only): consumers take `[u8]` bytes, the hand benchmark prints `i64` per byte. `_plain` (no count pass, one `arr_append`) is selected by the parser's plain bit, which the hand does not keep: without escapes the hand still runs both segment passes, each one `position` search (**P**, as P2's plain bit). |
| J5 | `string_text`, `string_text_scanned`, `string_text_plain`, `utf8_to_string`, `adapter_probe` | `String::from_utf8(vec)` (move); adapter `str_adapter` `to_owned` | `[u8] -> Text` | **copy** into a new `String` | validate + copy | Pruv 2 (bytes, then `String`); hand 1 | **W-c**: the `*_owned` variants are the hand shape; the copy variants stay until consumers switch (consumer steps). `adapter_probe` is **P** (adapter conformance probe, no hand counterpart). |
| J6 | `string_text_owned`, `*_scanned_owned`, `*_plain_owned`, `owned_text(take data)` | `String::from_utf8(vec)` / adapter: empty `String` on invalid input | `[u8] -> str` / `Text` | move | validate pass | 1 | **=** for valid input. Two Pruv-only branches: `owned_text` returns `""` on invalid UTF-8 (the `str_adapter` contract, builds an empty `String`: **SL/S**), and `*_owned` return `Text::InternalInvariant` when `utf8_valid` fails (**P**: a returned value the caller handles, not a panic; unreachable on real paths, see note J6). |
| J7 | `Text` enum (`Plain`/`InternalInvariant`) | `Result<String, _>` / `unwrap` | enum | — | — | none | **P**: the verifier cannot yet prove the parse-validated bytes are UTF-8 at this call, so the branch is a value, not `unreachable!()`. |
| J8 | UTF-16 key views (`[i64]` units) | `&[u16]` / `encode_utf16()` | `[i64]` vs `[u16]` | — | — | — | **U** (§4): no first-class UTF-16 view; `[i64]` holds one unit per word. |

## parse (`parse/parse.pruv`)

| # | Pruv | Hand Rust | Types | Ownership | Loops | Allocations | Id and reason |
|---|---|---|---|---|---|---|---|
| P0 | `parse(input)` error path | `hand_json.rs:243`: on a grammar error (kind != 4) `from_utf8(&input[position..])` | `[u8]` | borrow | one rescan of the rest | none | **=** (changed in 2.2 first pass: was a full rescan on every failure). |
| P1 | `Lexed`/`token_at`, `string_token`, `record` dispatch | `Parser { pos }` + `Result<_, Error>` returned from each method | `enum Lexed { Value(kind, plain), Error(kind) }` + `inout pos` vs `&mut self` + `Result` | borrow | — | none | **changed in 2.2c**. The recursive path dispatches on the byte inside `record` like the hand. `string_token` returns `Result<bool, Fault>` and is propagated with `?`. Literal and number scans convert to `Fault` once, where the hand's `literal`/`number` return the error. There is one error type, `struct Fault { position, kind }`, and the `Descent` enum is gone. `string_token` still returns the `plain` flag (used by `token_at` for `scan_into`, P8; `record` ignores it), and its `start` argument is used only by its contracts (`requires`, loop invariant, `ensures`) (**P**). Literals go through the shared `lex::scan_literal(input, pos, end) -> Scan` (it picks the expected word from the first byte and returns the position), where the hand's `literal(&[u8])` walks the expected bytes on `self.pos`; `lex` is outside this step's packages (**W-c**, shared with other `lex` users). `token_at -> Lexed` stays only for `scan_into` (P8). = (`Parser<'a>`'s borrowed `input` field is a parameter, see P7). |
| P2 | `utf8_width`, `string_token` | `Parser::string`: per byte, `from_utf8` on the slice | `[u8]` | borrow | one byte loop with `inout pos` (block skip removed in 2.2b) | none | **changed in 2.2b**. = The plain bit is **P** (feeds `string_utf8_plain`). |
| P3 | `append_record`; `write_record`, `close_record` (scratch) | `Parser::record`: `tape.extend_from_slice(&[kind, start, pos, id + 1, parent, 0])` | `[i64]` | `inout` (= `&mut Vec`) | — | amortized tape growth inside the append (both sides, counted on capacity change); the scratch helpers write a fixed tape, none | The recursive path (`scan`, `scan_document`) pushes each record with `append_record`, then patches end/next/count after the container returns, like the hand. The append is six `arr_push`, where the hand does one `extend_from_slice` of a six-word stack slice: **SX** (§4). Per-record `arr_reserve(6)` would be `reserve_exact` (quadratic), so it was not used. `write_record`/`close_record` stay only for `scan_into` (P8, **W-c**). |
| P4 | `comma_state`, `initial_state`, `value_state`, `value_increment`, internal error 18 | control flow of the recursive methods | `i64` state | — | — | none | **changed in 2.2b**: gone from the recursive path (`record`/`container` control flow, as the hand). = The state helpers remain only in `scan_into` (P8, **W-c**). |
| P5 | `first_slots`, `widened`, `regrown` (removed) | `Vec::with_capacity(32*WIDTH)`, growth inside `extend_from_slice` | `[i64]` | owned | — (2.2b had a copy loop) | initial 192 words, then amortized growth inside the append (2.2b: new buffer + copy) | **changed in 2.2b/2.2c**: `first_slots`/`widened`/`regrown` removed. `scan_records` does `arr_reserve(32*6)`, the tape grows inside the record append (amortized), and the frame stack is the call stack. In 2.2c the hand dropped its explicit per-record `reserve`: `extend_from_slice` grows the tape, and an allocation is counted only when the capacity changes. = |
| P6 | `scan`, `scan_document` | `Parser::record` recursive descent (`container`/`string`/`number`/`literal`) | `scan`: `[u8] -> Checked` (the built tape moves into `Checked::Tape`), `scan_document`: `[u8] -> JsonParse` | borrow input | one recursion on both sides (2.2b: two flat loops with an explicit stack) | tape | **changed in 2.2c**: one recursive descent, `record` ↔ `container` (`decreases 2*(129-depth)` / `+1`), both entry points through `scan_records`. `container` returns `Result<i64, Fault>` (the child count), like the hand's `ParseResult<i64>`. The ghost `first` argument is gone: the contracts use `len(old(tape))`. Two residues are gaps. `record` returns `Ok(0)` because `Result<(), Fault>` is `type_argument`: **RU** (§4). The `initial` argument stays because `old(pos)` of a scalar `inout` is not available: **OS** (§4). Proof-only `pred fn local_records`/`parents_ok` carry the tape shape (**P**). `container` loops `while pos <= end` with an end-of-input `Err` after the loop, where the hand uses `loop` and gets that error from `required()?`: the bound is the loop's termination measure (`decreases end - pos + 1`), which an unbounded `loop` does not have (**P**). |
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
| T1 | `new_state`, `new_slots`, `new_buffer`, `empty_buffer`, `grow` | `VecDeque::new()`, per-turn `String::new()` | packets | owned | fill loops | up-front slabs; `grow` = new buffer + copy | Slabs: **ABI / V** (T0). `grow` is **W-c**: the in-place form (`take buf` + `resize(move buf, ..)`, or a private `inout` + `arr_reserve`, checked to verify) changes the `pub` signature its callers `opencode/convert.pruv:202` and grok `read::window` use (`buf = grow(buf, ..)`), so it moves with those consumers. `empty_buffer` is **P** (DB7 lazy first slab, no hand counterpart). |
| T2 | `put_str`, `put_nl`, `write_piece`, `begin`/`answer` clip | Grok `hand_history.rs`: `&[u8]` clip and `Vec<u8>::extend_from_slice` | `[u8]` | `inout` | Pruv per byte | `begin`, `answer`: none (2.2b/2.2c) | **changed in 2.2b/2.2c**: `put_nl` is one `copy_span` of `"\n\n"`. `begin` and (2.2c) `answer` clip with `clip_len` on the byte range, so there is no temporary. = `put_str`/`write_piece` stay per byte: **SB** (§4), `copy_range(inout dst, at, src, ..)` with `src: str` is `type_mismatch` ("expected [u8], found str"). |
| T3 | `skip_user`, `tool` | `skip_user`, `tool` methods (flag / count updates) | `inout st`, `sl` | — | — | none | **=** in operation; shape is ABI / V (T0). |
| T4 | `plan` (203-word offset array) | `turns.iter().rev()` with a running byte total | `[i64]` | owned | one pass | none (2.2b) | **changed in 2.2b**: `plan` returns `Layout { kept, total, text_len }` (two passes, no array); the `finish*` writers walk a cursor down from `text_len` (`finish_inplace` up). = |
| T5 | `finish` | `Collector::finish(self) -> Vec<Turn>` (moves) | `Collected` | **copy** of `turns`/`text` at both returns (`turns.pruv:356`, `:391`) | — | +2 copies | **changed in 2.2b**: returns `move turns`, `move text` (no copy). = No caller change was needed. |
| T6 | `new_store`, `new_store_hint`, `reserve`, `move_bytes` (`Store` arena) | per-turn `String` allocation | `Store` | `inout` | compaction loop | arena + compaction | **ABI / V** consequence: one byte arena instead of owned `String`s. Growth inside `reserve` is **IF** (§4): `arr_reserve(inout bk.data, n)` is `type/inout_argument` ("an inout argument must be the whole name of a local array or an inout parameter"). Unchanged. |
| T7 | `begin2`, `answer2`, `begin3`, `answer3`, `finish2` | same methods | packets + `Store` | `inout` | per byte (2) / span copy (3) | arena | **W-c**: store-backed variants for claude/qwen/grok. In 2.2c, `begin2`/`answer2` clip with `clip_len` and copy with `put_str` (no temporary string). The per-byte copy left in `put_str` is **SB** (as T2). |
| T8 | `clip_span`, `put_nl_span`, `put_pad`, `write_span` | `clip` + `push_str` on a borrowed slice | `[u8]` | borrow / `inout` | `copy_span` (2.2b) | none | **changed in 2.2b**: `put_nl_span`, `put_pad` (one `copy_span` of the 4-byte pad scalar), `write_span` copy in bulk. = |
| T9 | `new_refs`, `begin_ref`, `answer_ref` | none (hand owns `String`s) | `[i64]` refs | `inout` | — | one refs array | **W-c**: reference-only variant for a zero-copy consumer; no hand counterpart (hand copies). |
| T10 | `text_layout`, `finish_into`, `finish_ref`, `finish_inplace` | `finish` then the consumer serialises `Vec<Turn>` | `Placed` | `inout wire` | per turn | none / turns array | **W-c**: writes straight into the consumer's output; the hand builds `Vec<Turn>` first. Consumer API. |

## transport (`transport/transport.pruv`)

| # | Pruv | Hand Rust | Types | Ownership | Loops | Allocations | Id and reason |
|---|---|---|---|---|---|---|---|
| X0 | `receive_grow` chunk read | `read_pipe`: `pipe.read(&mut buf)` into `[0; 65536]`, `buf[..n]` | `[u8]` | `inout chunk` | copy | none | **=** (changed in 2.2 first pass: per-byte copy → one `copy_range`). |
| X1 | `live`, `stdin_closed`, `send` | `child.stdin.write_all` on a writer thread | `Loop` moved through | move | — | none | **W-c**: event-loop handle instead of threads. `send` takes a deadline `ms` with no hand counterpart (hand blocks). Shared API of mcp, codex, grok. |
| X2 | `close`, `poll`, `expired` | `child.kill`/`wait`, `recv_timeout` | `Loop`, `inout st` | move | `expired`: at most 8 polls | none | **W-c** (same API as X1). The first table's `=` was wrong: the hand has no poll/expiry loop. |
| X3 | `grow_line(line, cap)` | `read_line` into a reused `String` (`clear`, capacity kept) | `[u8]` | `take line` (2.2b) | — | `let mut b = move line; b = resize(b, n, 0)`: one in-place `reserve_exact` + `resize` (Vec growth) | **changed in 2.2b**: `grow_line(take line, cap)`, the one caller `grok/rpc.pruv:750` passes `move aux` and assigns it back. = for the growth itself. The caller is not a line reader: `aux` holds the error-reply detail and the code bytes already written, which `resize` keeps (**W-c**: error-reply storage API; the hand Grok borrows the code range). |
| X4 | `receive_grow` event bytes | `line.as_bytes().to_vec()` once | `[u8]` | `ev_proc_read_into(move q, slot, 1, inout chunk, inout got)` | — | 1 (=) | **changed in 2.2b**: reads the event bytes straight into `chunk`. = |
| X5 | `receive_grow` framing | `BufReader::read_line` (newline kept, partial last line returned at EOF) | `[u8]` | `inout line` | scan to the newline (or cap), then `truncate` to the filled length and one `arr_append` of the chunk | growth only (amortized inside `arr_append`, no zero fill) | **changed in 2.2c**: growth is the hand's append (the 2.2b `resize` zero fill is gone). The newline scan stays byte by byte, where `read_line` uses `memchr`: **MC** (§4). Framing semantics differ on purpose (**W-c**): Pruv appends raw bytes with no UTF-8 check (consumers parse the line as JSON, which checks it), where `read_line` into a `String` validates and returns its error with `?`; Pruv excludes the newline and drops/flags a partial last line at EOF per the `st` status (-3 at cap kept); consumers rely on that. |
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

Step 2.2 added no gap ids. Step 2.2b added **IF** and **SB** and cites **B** for J1. VC is closed by LB5; **C** no
longer blocks X3 (the caller reuses one buffer). Gap ids that remain:

| Id | Hand Rust form | What Pruv cannot express | Functions left unchanged |
|---|---|---|---|
| B-LLVM | `Document<'a> { source: &'a [u8] }` | borrowed fields on LLVM | `parse` source copy (P7) |
| B | `struct Units<'a> { raw: &'a [u8], pos }` iterator | a struct field of borrowed type (`type/borrow_signature`) | `jstr` decode helpers (J1) |
| IF | `self.data.reserve(n)` on a struct field | `inout` argument that is a field of an `inout` struct (`type/inout_argument`) | `turns::reserve` growth (T6) |
| SB | `dst[at..at+n].copy_from_slice(&s.as_bytes()[..n])` | a `str` as the byte source of `copy_range` (`type_mismatch`) | `turns::put_str`, `write_piece` (T2) |
| ABI / V | `Collector { turns: VecDeque<Turn> }` returning `Vec<Turn>` | owned struct collections across the host boundary | `turns` package API (T0, T6) |
| U | `&[u16]` keys | first-class UTF-16 view | `jstr` key arrays (J8) |
| SL / S | `""` literal never allocated | `""` in a `str` result emits `String::from` | `jstr::owned_text` invalid-input branch (J6) |
| FM | ring tail picked by stream | `inout` binding to a run-time chosen field | `transport` stderr tail (X7) |

## Step 2.3: access, lines

Function-level table for `pkgs/access` and `pkgs/lines`, same columns as step 2.2. Hand references:
`json/hand/hand_json.rs` `Document` (`value`, `array_at`, `field` last-wins, `units`, query modes 3/5/6, `raw_output`),
`claude/handrust` `get_str` (`Cow::Borrowed` for an escape-free string), and the hand line readers
(`claude/handrust` `edge_lines`, `qwen/handrust` `window_import`: `split_inclusive(b'\n')` + `strip_suffix`).
Call paths run from the product entry to the owner of the result. No hand Rust changed: every rewrite below was
already the hand shape.

| # | Pruv | Hand Rust | Types | Ownership | Loops | Allocations | Id and reason |
|---|---|---|---|---|---|---|---|
| A0 | `root`, `valid`, `kind`, `raw_span`, `array_length` | `Document::value`, `tape[i].kind`, `span`, record count | `JsonDocument` read, `i64`/`Span` | borrowed doc | none | none | **=** |
| A1 | `array_at(doc, id, index)` | `Document::array_at`: `index >= count` is a miss, then `index` steps along `next` | `i64` record id | borrowed doc | `index` link steps | none | **changed in 2.3**: was a walk over the whole array (`j < stop`) with a per-element parent check; now the count decides the miss and the loop runs `index` steps, the parent/valid check once on the hit. Same results on every tape that passes `check_tape` (the count equals the direct children reached along `next`), as the hand trusts the count. = Callers: `json/a4_root.pruv:22`, `json/q_root.pruv:85`, `mcp/perf/perf_root.pruv:35` (`b_root` has its own `array_at`) |
| A2 | `field_units(doc, id, key: [i64])`, `find` | mode 3 non-ASCII matcher `units(raw).eq(text.encode_utf16())` | key as `[i64]` UTF-16 units | borrowed | members of the object, last match wins | none | **U**: the key is a caller-built `[i64]`; the hand compares against `encode_utf16()` of a `&str` iterator. Callers: `json/q_root`, `a4_root` |
| A3 | `field(doc, id, key: str)`, `find_ascii`, `may_match`, `is_ascii` | `Document::field` + mode 3 fast path `text.is_ascii() && !body.contains('\\')` | `str` key | borrowed | members, last match wins | ASCII key: none (=). Non-ASCII key: `alloc(len(key))` units | **=** for the ASCII path (every product key is ASCII). The non-ASCII path builds a key unit array: **U** |
| A4 | `fields_into(doc, id, n, k0..k7, inout out)` + `hit`, `slot_error`, `fill_error`, `field_if_used`, `last_match` | one `get(key)` per key on a `BTreeMap` | 8 `str` keys, `inout [i64]` | `inout out` | one pass over members | none | **W-c**: multi-key one-pass API used by `claude/import.pruv:450` and `claude/entry.pruv:220,248` (owner: the claude import record); the hand does a lookup per key. Consumer step |
| A5 | `string_units(doc, id) -> Units` | query mode 5 `units(raw).collect::<Vec<u16>>()` | `Units { status, units: [i64] }` | result owns the units | one decode | one (the units) | **changed in 2.3**: miss paths built an empty array and returned `copy(empty)`, the hit returned `copy(u)`; now `alloc(0, 0)` and `move u`. = Callers: `codex/rpc.pruv:418,430`, `codex/root.pruv:276`, `grok/rpc.pruv:706`, `grok/path.pruv:328`, `mcp/perf/perf_root.pruv:37` |
| A6 | `string_text`, `string_text_scanned` | `get_str` → `String` | `jstr::Text` | result owns | decode | one | **W-c** through `jstr` J5 (status-carrying text). Callers: `qwen` import, `grok/util.pruv:106` (`_scanned_owned`) |
| A7 | `string_is(doc, id, lit)` | `get_str(..) == Some(lit)`: `Cow::Borrowed` for an escape-free string, decode only with escapes | `i64` | borrowed | ASCII: compare; plain: compare in place; escaped: decode then compare | ASCII and plain: none. Escaped: one decode buffer (= `Cow::Owned`) | **changed in 2.3**: a non-ASCII literal against an escape-free string (tape plain bit) decoded into a fresh `[u8]` before; now compares the source bytes in place. = for documents from `parse::scan_into` (`build_span`: `grok/read.pruv:360`, `qwen` `parse_span`). Documents from the owned parser (`parse::parse`, `parse_owned`/`parse_copy`) always carry plain bit 0 (`parse.pruv:222`), so a non-ASCII literal still decodes there; writing the bit is a `parse` row, outside this step. Static callers pass ASCII literals (`claude/entry.pruv:238,339` through `field_is`, `claude/import.pruv:356..541`, `codex/item.pruv:142..166`), which never reach the decode |
| A8 | `raw_is`, `view_root_type(data, lo, tape, records)` | mode 3 fast path on the raw bytes | scratch tape view | borrowed | members of the root | none | **W-c**: reads a scratch tape before a document exists (`grok/read.pruv:359`); the hand builds the `Document` first. Consumer step |
| A9 | `boolean`, `number` | `raw_output`: `parse::<i64>` within ±(2^53-1), `-0` kept | `i64`, `Number` | borrowed | digits | none | **=** Callers: `codex/rpc.pruv:416`, `grok/rpc.pruv:668`, `grok/summary.pruv:154` |
| A10 | `string_text_owned`, `string_text_scanned_owned` | `String::from_utf8` of the decoded bytes | `Text` | result owns | decode | one | **=** |
| L0 | `jsonl_next(data, offset, is_final)` | `split_inclusive(b'\n')` + `strip_suffix(b"\n")`/`b"\r"` | `JsonlStep` | borrowed | scan to newline | none | **=** Caller: `claude/framing.pruv:28` |
| L1 | `parse_range(data, lo, hi)` | `parse(&data[lo..hi])`, borrowed `Document` | `JsonParse` | document owns a copy of the range | UTF-8 check + scan (in `parse_owned`) | one exact copy of the range, the tape | **changed in 2.3**: was a zeroed `alloc(n, 0)`, a byte loop, a scan, then a second `copy(owned)`; now `copy(subview(data, lo, hi))` moved into `parse_owned` (as `parse_copy`). The remaining copy is **B-LLVM** (owned source on LLVM). Callers: `qwen/entry.pruv:12`, `grok/read.pruv:362` (fallback) |
| L2 | `parse_owned(take data, hi)` | `parse(&data)` on an owned buffer | `JsonParse` | takes the buffer; `truncate` in place | UTF-8 check + scan | the tape | **=** Caller: `grok/rpc.pruv:628` |
| L3 | `parse_copy(data, lo, hi)` | `parse(&line[lo..hi])` | `JsonParse` | one exact copy | as L2 | copy + tape | **B-LLVM** (copy kept for the owned source). Callers: `codex/rpc.pruv:82`, `mcp/root.pruv:118` |
| L4 | `scan_span`, `parse_span` | none (hand parses each line into a fresh `Document`) | caller scratch `inout tape`, `inout stack` | `inout` | scan | none in the scan | **W-c**: scratch API like P8. Callers: `qwen/entry.pruv:32`, `grok/read.pruv:570` |
| L5 | `build_span(data, lo, hi, tape, records)` | `Document { source: data[lo..hi].to_vec(), tape: tape[..m].to_vec() }` | `JsonDocument` | result owns both | tape prefix: element loop | source: one exact copy (=); tape: zeroed `alloc(m, 0)` + loop | **changed in 2.3**: the result took `copy(source)` and `copy(exact)` of locals it owned; now both are moved. The tape prefix loop stays: **ZP** (new in §4). Callers: `grok/read.pruv:360`, `parse_span` |
| L6 | `absolute_offset(base, start, position)` | `base.checked_add(start)?.checked_add(position)` | `i64` | — | none | none | **=** |

Status count: = 9 rows (A0, A1, A5, A7, A9, A10, L0, L2, L6) plus the ASCII path of A3 and the source half of L5;
W-c 4 (A4, A6, A8, L4); U 2 (A2, A3 non-ASCII key); B-LLVM 2 (L1, L3); ZP 1 (L5 tape). No W rows remain. No measurement: the structure difference is not zero (U, B-LLVM, ZP, W-c).

### Gap rows found in step 2.3

| Id | Hand Rust form | What Pruv cannot express | Functions left unchanged |
|---|---|---|---|
| ZP (new) | `tape[..m].to_vec()` | bulk copy of an `[i64]` prefix: `copy(subview(tape, 0, m))` and `arr_append(inout exact, tape, 0, m)` fail with "z3 error 6: invalid pattern" (G-DB21-1) | `lines::build_span` tape prefix (L5) |
| U | `text.encode_utf16()` compared to `units(raw)` | first-class UTF-16 key view | `access::field_units`, non-ASCII `field` (A2, A3) |
| B-LLVM | `Document<'a> { source: &'a [u8] }` | borrowed document source on LLVM | `lines::parse_range`, `parse_copy` copy (L1, L3) |
