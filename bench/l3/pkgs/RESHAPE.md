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
| json | 2 (B-LLVM source copy, D32 publication checks; root tape move reshaped in step 2.8) | no |
| text | 2 (`trim`, `clip_bytes`: B-LLVM; `one_line` reshaped in step 2.5) | no |
| names | 0 (`project_name` reshaped in step 2.2b) | no |
| args | 1 (`arg_spans`: span packet kept for codex `child_args` and the U8 check; `build_args` reshaped in step 2.8) | no |
| tail | whole package (hand is UTF-8 `Vec<u8>`; decoder pick: FM) | no |
| lex | 0 (cursor and return form done in step 2.7; `e?;` codegen row, not a source difference) | yes, `scan_string` M and L (step 2.7) |
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

| root `Parsed(document) => copy(document.tape)` (`json/a2_root.pruv`) | `Ok(doc) => doc.tape` | `[i64]` | Pruv copies on LLVM | — | LLVM: 1 tape-sized copy | **TK**. The Rust backend already emits `return document.tape`. **Changed in step 2.8**: `let tape: [i64] = move document.tape;` (LB9 R-6g8-4), no copy on either backend. |

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
| `one_line(s, max) -> str` | `qwen/handrust/src/lib.rs:325`: `for c in s.trim_matches(is_es_ws).chars()`, `out.push(c)` per scalar, `push(' ')` per whitespace run | `str -> String` | owned result (=) | one pass (=) | `out` growth only (=) | **changed in 2.5** (LB7, R-6g6-3): the scalar is pushed as `string_push_str(move out, subview(s, i, j))`, Rust `out.push_str(&s[i..j])`, with no temporary (was one `substr` per scalar). = See step 2.5 for the line-by-line comparison. |
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
| `arg_spans(argv) -> [i64]` | `argv.is_empty() \|\| argv.ends_with('\0')` check, no spans | `str -> [i64]` | owned packet | one pass | Pruv 1-2; hand 0 | Unchanged in shape (step 2.8 adds one `ensures` and one invariant). No longer needed by `build_args`; kept for codex `rpc.pruv` `child_args` (reads the spans, outside the pkgs bundle) and as the validation step of mcp `root.pruv`. Its runtime `is_char_boundary(argv, i + 1)` check is the **U8** gap (step 2.8). |
| `build_args(argv) -> ProcessArgs` | `for p in argv.split_terminator('\0') { cmd.arg(p) }` | `str -> ProcessArgs` | owned result | one pass | hand 1 per argument (inside `arg`); Pruv span packet + `substr` + `String::from` per argument | **Changed in step 2.8**: `find_byte` + `proc_args_push(move a, subview(argv, start, end))`, no span packet, no temporary (=). |

Callers (mcp `root.pruv`, codex `rpc.pruv` `child_args`) unchanged. Since step 2.8 `build_args` no longer calls
`arg_spans`; mcp `root.pruv` already checks `arg_spans` before it calls `build_args`.

## tail (`tail/tail.pruv`, hand: `supervisor/hand/core_hand.rs`)

Unchanged (HEAD). Hand keeps the tail as UTF-8 `Vec<u8>` (`Vec::with_capacity(6 * limit + 4)`, `trim` drops whole
scalars from the front by lead byte, `finish(self)` returns the same vector). Pruv keeps a ring of UTF-16 units in two
loose arrays (`tb`, 12-word `ts` packet), with `tail_bytes` re-encoding into a new buffer. Every function differs. The
rewrite to the hand shape is open; its decoder pick needs `let d = if s == 1 { &mut tl.d1 } else { &mut tl.d2 }`,
which Pruv cannot express (gap **FM**). A first attempt that grouped the state into `Tail` / `Decoder` structs but kept
UTF-16 units and copied the chosen decoder in and out was withdrawn as a non-hand shape.

## lex (`lex/lex.pruv`, hand: `json/hand/hand_json.rs` scanner) — `scan_string` loop reshaped in step 2.6, cursor form in step 2.7

| Pruv | Hand Rust | Types | Ownership | Loops | Allocations | Difference and reason |
|---|---|---|---|---|---|---|
| `struct Fault { position, kind }`, every `scan_*(input, inout pos, end, ..) -> Result<(), Fault>` | `ParseResult<()>` = `Result<(), (usize, i64)>`, cursor in `self.pos` | `i64` pair / `(usize, i64)` | `inout pos` ↔ `&mut self.pos` | — | none | = since step 2.7 (`Scan` is gone; RU closed by LB8). The error is a named struct where hand uses a tuple; same two values. |
| `validate_utf8(input, start, end) -> i64` | `std::str::from_utf8(&input[a..b])` | `[u8]` | borrow | 16-byte ASCII blocks, then per char | none | Same algorithm as std (ASCII fast path, then sequence check); returns the error offset like `valid_up_to`. = |
| `skip_whitespace` | `while matches!(b, b' '\|b'\t'\|b'\n'\|b'\r')` | `[u8]` | borrow | one pass | none | = |
| `scan_literal(input, inout pos, end, literal: str)` | `hand_json.rs:67` `literal(&mut self, literal: &[u8])`: `required()` per byte, error 12 at the first mismatch | `str` for `&[u8]` (no byte-string literal; `literal[k]` emits `as_bytes()` indexing) | borrow | one per expected byte | none | = since step 2.7: walks the given literal; the caller picks it from the first byte like hand `record` (`b't' => self.literal(b"true")?`). |
| `digits`, `scan_number` | `hand_json.rs:77` `digits()`, `:87` `number()` | `[u8]` | borrow | one pass | none | = since step 2.7: `digits` requires its first digit itself (error 5 / 13) and is called with `?` from `scan_number`, like hand. |
| `scalar_width(byte)` | `hand_json.rs:235` `utf8_width(byte: u8) -> usize` | `i64` → `i64` (SPEC §6b: no scalar `u8`/`usize`) | — | — | none | Same table (194-223/224-239/240-244/else 1). Named apart from `parse::utf8_width` only to keep the Python verifier stable (step 2.6). |
| `scan_string` | `hand_json.rs:113` `string()`: `while let Some(byte) = self.peek()` per byte | `[u8]` | borrow | per byte; a byte ≥ 128 checks one scalar `[i, i + width)` with `from_utf8` and jumps over it | none | Loop = since step 2.6 (the 16-byte block skip is gone). The scalar check passes `(input, i, i + width)` where hand passes `&input[i..i + width]`. Cursor and return form = since step 2.7. |

Step 2.6 rewrote the `scan_string` loop to the hand shape; step 2.7 moved the whole package to the hand cursor and return form.

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

Four rows kept a residue that Pruv could not express at step 2.2c (all but RU closed by LB6 and rewritten in step 2.4; RU closed by LB8 and rewritten in step 2.7). Each residue is a §4 gap id, not a **W**:
- **RU** (P6)
- **OS** (P6)
- **SX** (P3)
- **MC** (X5)

RU, OS and SX were checked with a minimal file against `pruvc-rs`, and MC against the std list in SPEC. Because gaps remain, no performance was measured.

## Status (function level)

| Package | = | Changed in | W | W-c | Gap id | P |
|---|---|---|---|---|---|---|
| jstr | J2, J2b, J3, J4 | J3 `string_units`; 2.2b/2.2c: J4 | — | J5 copy variants | B (J1), SL/S (J6), U (J8) | J5 `adapter_probe`, J6/J7 `InternalInvariant` |
| parse | P0, P1, P2, P3, P4, P5, P6 | P0 (first pass); 2.2b: P1-P6; 2.2c: P1, P3, P5, P6; 2.4: P3, P6; 2.7: P1 literals, P6 `record` | — | P8 | B-LLVM (P7) | P2 plain bit, P9 |
| turns | T2, T3 (operation only), T4, T5, T8 | 2.2b: T2 (part), T4, T5, T8; 2.2c: T2 `answer`, T7; 2.4: T2 `put_str`/`write_piece`, T7 copies | — | T1 `grow`, T7, T9, T10 | ABI / V (T0, T1 slabs, T6 incl. growth; T2 writes the slot buffer `buf`, T7 writes `bk.data`, neither a per-turn `Vec`) | T1 `empty_buffer` |
| transport | X0, X3, X4, X5 (scan, growth) | X0 (first pass); 2.2b: X3, X4, X5; 2.2c: X5 growth; 2.4: X5 scan | — | X1, X2, X6, X5 framing | FM (X7) | — |

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
| P1 | `Lexed`/`token_at`, `string_token`, `record` dispatch | `Parser { pos }` + `Result<_, Error>` returned from each method | `enum Lexed { Value(kind, plain), Error(kind) }` + `inout pos` vs `&mut self` + `Result` | borrow | — | none | **changed in 2.2c**. The recursive path dispatches on the byte inside `record` like the hand. `string_token` returns `Result<bool, Fault>` and is propagated with `?`. Literal and number scans return `lex::Fault` and `record` propagates it with `?`, like the hand's `literal`/`number` (2.2c converted a `Scan` once; 2.7 removed that). The `Descent` enum is gone. `string_token` still returns the `plain` flag (used by `token_at` for `scan_into`, P8; `record` ignores it), and its `start` argument is used only by its contracts (`requires`, loop invariant, `ensures`) (**P**). Literals go through the shared `lex::scan_literal(input, inout pos, end, "true")?` with the literal picked from the first byte, like the hand `self.literal(b"true")?` (**changed in 2.7**; before, `scan_literal` picked the word itself and returned a `Scan`). There is one error type, `lex::Fault { position, kind }` (parse's private copy is gone in 2.7). `token_at -> Lexed` stays only for `scan_into` (P8). = (`Parser<'a>`'s borrowed `input` field is a parameter, see P7). |
| P2 | `utf8_width`, `string_token` | `Parser::string`: per byte, `from_utf8` on the slice | `[u8]` | borrow | one byte loop with `inout pos` (block skip removed in 2.2b) | none | **changed in 2.2b**. = The plain bit is **P** (feeds `string_utf8_plain`). |
| P3 | `append_record`; `write_record`, `close_record` (scratch) | `Parser::record`: `tape.extend_from_slice(&[kind, start, pos, id + 1, parent, 0])` | `[i64]` | `inout` (= `&mut Vec`) | — | amortized tape growth inside the append (both sides, counted on capacity change); the scratch helpers write a fixed tape, none | The recursive path (`scan`, `scan_document`) pushes each record with `append_record`, then patches end/next/count after the container returns, like the hand. **changed in 2.4**: the append is one `arr_push(inout tape, k, s, e, r + 1, parent, count)` (R-6g5-5), emitted as `tape.extend_from_slice(&[k, s, e, (r + 1), parent, count])`, as `json/hand/hand_json.rs:182-184`. **=** (2.2c residue **SX** closed by LB6). Per-record `arr_reserve(6)` would be `reserve_exact` (quadratic), so it was not used. `write_record`/`close_record` stay only for `scan_into` (P8, **W-c**). |
| P4 | `comma_state`, `initial_state`, `value_state`, `value_increment`, internal error 18 | control flow of the recursive methods | `i64` state | — | — | none | **changed in 2.2b**: gone from the recursive path (`record`/`container` control flow, as the hand). = The state helpers remain only in `scan_into` (P8, **W-c**). |
| P5 | `first_slots`, `widened`, `regrown` (removed) | `Vec::with_capacity(32*WIDTH)`, growth inside `extend_from_slice` | `[i64]` | owned | — (2.2b had a copy loop) | initial 192 words, then amortized growth inside the append (2.2b: new buffer + copy) | **changed in 2.2b/2.2c**: `first_slots`/`widened`/`regrown` removed. `scan_records` does `arr_reserve(32*6)`, the tape grows inside the record append (amortized), and the frame stack is the call stack. In 2.2c the hand dropped its explicit per-record `reserve`: `extend_from_slice` grows the tape, and an allocation is counted only when the capacity changes. = |
| P6 | `scan`, `scan_document` | `Parser::record` recursive descent (`container`/`string`/`number`/`literal`) | `scan`: `[u8] -> Checked` (the built tape moves into `Checked::Tape`), `scan_document`: `[u8] -> JsonParse` | borrow input | one recursion on both sides (2.2b: two flat loops with an explicit stack) | tape | **changed in 2.2c**: one recursive descent, `record` ↔ `container` (`decreases 2*(129-depth)` / `+1`), both entry points through `scan_records`. `container` returns `Result<i64, Fault>` (the child count), like the hand's `ParseResult<i64>`. The ghost `first` argument is gone: the contracts use `len(old(tape))`. **changed in 2.4**: the `initial` argument is gone; the contracts and the `container` loop invariant use `old(pos)` (R-6g5-4), so `record(input, inout pos, inout tape, parent, depth, key)` matches `hand_json.rs:155` `record(&mut self, parent, depth, key)` and `container(input, inout pos, inout tape, id, depth, object)` matches `:197` (`pos`/`tape` are the `self` fields). **changed in 2.7**: `record` returns `Result<(), lex::Fault>` like the hand `ParseResult<()>` (`Ok(0)` is gone; RU closed by LB8), and `scan_records` propagates it the same way. Proof-only `pred fn local_records`/`parents_ok` carry the tape shape (**P**). `container` loops `while pos <= end` with an end-of-input `Err` after the loop, where the hand uses `loop` and gets that error from `required()?`: the bound is the loop's termination measure (`decreases end - pos + 1`), which an unbounded `loop` does not have (**P**). |
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
| T2 | `put_str`, `put_nl`, `write_piece`, `begin`/`answer` clip | Grok `hand_history.rs`: `&[u8]` clip and `Vec<u8>::extend_from_slice` | `[u8]` | `inout` | one bulk copy each (2.4); `clip_len` scans only for the UTF-8 cut | `begin`, `answer`: none (2.2b/2.2c) | **changed in 2.2b/2.2c**: `put_nl` is one `copy_span` of `"\n\n"`. `begin` and (2.2c) `answer` clip with `clip_len` on the byte range, so there is no temporary. **changed in 2.4**: `put_str` is one `copy_range(inout dst, at, src, 0, n)` from the `str` (R-6g5-1, Rust `copy_from_slice` of `src.as_bytes()`), so `write_piece` is `put_nl` + one bulk copy, as `grok/perf/hand_history.rs:769-770` (`extend_from_slice(&b"\n\n"[..s2])`, `extend_from_slice(&text[..take])`) and `begin` as `:724`. **=** at the function level (2.2b residue **SB** closed by LB6). Destinations: `begin`/`answer` write the fixed slot buffer `buf`; `begin2`/`answer2` (T7) write `bk.data` of the `Store` arena. Neither is the hand's per-turn `Vec`: **ABI / V** (T0, T6). |
| T3 | `skip_user`, `tool` | `skip_user`, `tool` methods (flag / count updates) | `inout st`, `sl` | — | — | none | **=** in operation; shape is ABI / V (T0). |
| T4 | `plan` (203-word offset array) | `turns.iter().rev()` with a running byte total | `[i64]` | owned | one pass | none (2.2b) | **changed in 2.2b**: `plan` returns `Layout { kept, total, text_len }` (two passes, no array); the `finish*` writers walk a cursor down from `text_len` (`finish_inplace` up). = |
| T5 | `finish` | `Collector::finish(self) -> Vec<Turn>` (moves) | `Collected` | **copy** of `turns`/`text` at both returns (`turns.pruv:356`, `:391`) | — | +2 copies | **changed in 2.2b**: returns `move turns`, `move text` (no copy). = No caller change was needed. |
| T6 | `new_store`, `new_store_hint`, `reserve`, `move_bytes` (`Store` arena) | per-turn `String` allocation | `Store` | `inout` | compaction loop | arena + compaction | **ABI / V** consequence: one byte arena instead of owned `String`s. Growth inside `reserve` was **IF** (§4), closed by LB6 (`arr_reserve(inout bk.data, n)` now emits `bk.data.reserve_exact(n)`). **Unchanged in 2.4**: `Store` keeps `len(data)` as its capacity and writes by index at `bk.end` into zeroed bytes (`begin2`/`answer2`/`begin3`/`answer3`, and the `bk.end <= len(bk.data)` contracts in `qwen/import.pruv`, `grok/entry.pruv`, `grok/line.pruv`, `grok/read.pruv`). A capacity-only `arr_reserve` does not give that room; using it needs the arena turned Vec-shaped (`len == end`, appends), and the hand has no arena at all (per-turn `String`). Remaining reason: **ABI / V**. |
| T7 | `begin2`, `answer2`, `begin3`, `answer3`, `finish2` | same methods | packets + `Store` | `inout` | bulk copy (2, since 2.4; `clip_len` scans for the UTF-8 cut) / span copy (3) | arena | **W-c**: store-backed variants for claude/qwen/grok. In 2.2c, `begin2`/`answer2` clip with `clip_len` and copy with `put_str` (no temporary string). **changed in 2.4** through T2: the copies are one bulk copy each (`hand_history.rs:724`, `:769-770`); `begin3`/`answer3` already copy spans. Remaining: **W-c** (store-backed variants) and **ABI / V** (arena). |
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
| X5 | `receive_grow` framing | `BufReader::read_line` (newline kept, partial last line returned at EOF) | `[u8]` | `inout line` | scan to the newline (or cap), then `truncate` to the filled length and one `arr_append` of the chunk | growth only (amortized inside `arr_append`, no zero fill) | **changed in 2.2c**: growth is the hand's append (the 2.2b `resize` zero fill is gone). **changed in 2.4**: the newline scan is one `find_byte(chunk, 10, begin, stop)` (R-6g5-3; Rust `position`, LLVM `memchr`) with `stop` the buffered end bounded by the room under cap, as `read_line` (`codex/hand/reader.rs:30`, `memchr` in `read_until`). **=** for scan and growth (2.2c residue **MC** closed by LB6). Framing semantics differ on purpose (**W-c**): Pruv appends raw bytes with no UTF-8 check (consumers parse the line as JSON, which checks it), where `read_line` into a `String` validates and returns its error with `?`; Pruv excludes the newline and drops/flags a partial last line at EOF per the `st` status (-3 at cap kept); consumers rely on that. |
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
longer blocks X3 (the caller reuses one buffer). Gap ids as of step 2.2 (IF and SB closed by LB6, see the rows):

| Id | Hand Rust form | What Pruv cannot express | Functions left unchanged |
|---|---|---|---|
| B-LLVM | `Document<'a> { source: &'a [u8] }` | borrowed fields on LLVM | `parse` source copy (P7) |
| B | `struct Units<'a> { raw: &'a [u8], pos }` iterator | a struct field of borrowed type (`type/borrow_signature`) | `jstr` decode helpers (J1) |
| IF | `self.data.reserve(n)` on a struct field | closed by LB6 | — (T6 growth now waits on ABI / V, see T6) |
| SB | `dst[at..at+n].copy_from_slice(&s.as_bytes()[..n])` | closed by LB6 | — (T2 rewritten in 2.4) |
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
| L5 | `build_span(data, lo, hi, tape, records)` | `Document { source: data[lo..hi].to_vec(), tape: tape[..m].to_vec() }` | `JsonDocument` | result owns both | tape prefix: element loop | source: one exact copy (=); tape: one exact copy (2.5) | **changed in 2.3**: the result took `copy(source)` and `copy(exact)` of locals it owned; now both are moved. **changed in 2.5**: the tape prefix is `copy(subview(tape, 0, m))` (ZP closed by LB7, R-6g6-2), Rust `tape[..m].to_vec()`. = Callers: `grok/read.pruv:360`, `parse_span` |
| L6 | `absolute_offset(base, start, position)` | `base.checked_add(start)?.checked_add(position)` | `i64` | — | none | none | **=** |

Status count: = 9 rows (A0, A1, A5, A7, A9, A10, L0, L2, L6) plus the ASCII path of A3 and the source half of L5;
W-c 4 (A4, A6, A8, L4); U 2 (A2, A3 non-ASCII key); B-LLVM 2 (L1, L3); ZP 1 (L5 tape). No W rows remain. No measurement: the structure difference is not zero (U, B-LLVM, ZP, W-c).

### Gap rows found in step 2.3

| Id | Hand Rust form | What Pruv cannot express | Functions left unchanged |
|---|---|---|---|
| ZP (new) | `tape[..m].to_vec()` | bulk copy of an `[i64]` prefix: `copy(subview(tape, 0, m))` and `arr_append(inout exact, tape, 0, m)` fail with "z3 error 6: invalid pattern" (G-DB21-1) | `lines::build_span` tape prefix (L5) |
| U | `text.encode_utf16()` compared to `units(raw)` | first-class UTF-16 key view | `access::field_units`, non-ASCII `field` (A2, A3) |
| B-LLVM | `Document<'a> { source: &'a [u8] }` | borrowed document source on LLVM | `lines::parse_range`, `parse_copy` copy (L1, L3) |

## Step 2.4: rows freed by LB6 (SB, IF, MC, OS, SX)

LB6 (SPEC R-6g5-1..5) closed SB, IF, MC, OS and SX. This step rewrites the rows that waited on them; RU and ZP stay open, so the rows that wait on them keep their gap.

| Row | Function | Status | Hand line |
| --- | --- | --- | --- |
| P3 | `parse::append_record` | **=**: one `arr_push` of six values, Rust `tape.extend_from_slice(&[..])` | `json/hand/hand_json.rs:182-184` |
| P6 | `parse::record`, `container` | `initial` gone, contracts use `old(pos)`; `Ok(0)` stays: **RU** | `hand_json.rs:155`, `:197` |
| T2 | `turns::put_str`, `write_piece` | **=**: one `copy_range` from the `str`; destination is `buf` (`begin`/`answer`) or `bk.data` (T7), not a per-turn `Vec`: **ABI / V** | `grok/perf/hand_history.rs:724`, `:769-770` |
| T6 | `turns::reserve` | unchanged: IF closed, but `Store` keeps `len(data)` as capacity, so `arr_reserve` needs the Vec-shaped arena: **ABI / V** | hand has no arena (per-turn `String`) |
| T7 | `turns::begin2`, `answer2` | bulk copy through T2; **W-c** and **ABI / V** remain | `hand_history.rs:724`, `:769-770` |
| X5 | `transport::receive_grow` | **=** for the scan: one `find_byte` (LLVM `memchr`), as `read_line` | `codex/hand/reader.rs:30` |

`bench/l3/lib/access.pruv` was removed in this step. Every root that has `mod access;` (claude, codex, qwen, mcp, grok, opencode, json, `pkgs/time`) takes it from `pkgs/access` through its `pruv.json` dependency, and no `--lib bench/l3/lib` user (budget, req, entry) refers to `crate::access`. The mutant text in `json/run_q_mutants.py` for P3 moved to the one-`arr_push` form.

With P3 and P6 together (either one alone was fine), the Python verifier timed out on `claude/time.pruv` `canonical_copy` for the claude a2 and a4 roots. That function does not use `parse`, so the cause is solver instability from the changed declarations, not the new code. The Rust compiler proved it unchanged. The fix adds one loop invariant to `canonical_copy` that states the digit property for `out[0..j]` directly. Its contracts are unchanged.

## Step 2.6: lex string scanner in the hand shape

**Split.** `lex::scan_string` is the hand `Parser::string` (`json/hand/hand_json.rs:113-153`). It is one per-byte loop. `"` ends the string. A byte below 32 is error 16. A byte of 128 or more checks one scalar: `scalar_width` (hand `utf8_width`, `:235`) gives the width, and `validate_utf8` (std `from_utf8`) checks `[i, i + width)`. A short or bad scalar is error 4 at its first byte. `\` reads one escape (`\u` with four hex digits). `validate_utf8` stays as the separate std `from_utf8` counterpart. The hand never validates the whole input first, so there is no proof to take the 16-byte plain-byte skip away. The skip was an extra loop, so it was removed. The hand baseline is unchanged because it already had this shape. `parse::string_token` has the same shape, so lex and parse now scan strings the same way.

| Pruv | Hand Rust | Types | Ownership | Loops | Allocations | Difference |
|---|---|---|---|---|---|---|
| `scan_string` | `Parser::string` | `[u8]`, `i64` positions / `&[u8]`, `usize` | borrow | one per-byte loop; scalar jump on ≥ 128 | none | Loop, error kinds and positions = (hand tests `\\` before ≥ 128, Pruv the other way; the cases do not overlap). Cursor by value and `Scan` return vs `&mut self.pos` and `Result<(), E>` (RU). Scalar check gets `(input, i, e)`, hand a sub-slice `&input[i..e]` |
| `scalar_width` | `utf8_width` | `i64` / `u8 → usize` | — | — | none | Same table; `i64` for `u8`/`usize` (SPEC §6b); name differs |
| `validate_utf8` | `std::str::from_utf8` | `[u8]` | borrow | ASCII blocks, then per sequence | none | = (unchanged) |

The loop difference is gone. The structure difference is not zero: the cursor and return form stays. The `Result<(), E>` half is the open gap **RU** (RUST_SHAPE_RESET §4). The `inout pos` half is expressible since OS closed. It is a change to the `scan_*` API as a whole and to callers outside this step's files (`parse`, `claude/entry`), so it is listed here, not worked around. Integer types (`i64` for `u8`/`usize`, SPEC §6b) and `(array, lo, hi)` arguments for a sub-slice are the convention of every `=` row in this file and are not counted. No new gap row. The helper is named `scalar_width`, not `utf8_width`, for one reason. With a second `utf8_width` (next to `parse`'s private one, same body), the Python verifier timed out on `claude/time.pruv` `iso_of` → `canonical_copy` in the claude a2/a4 roots, every time, with either visibility. This is the solver instability noted in step 2.4. The two other names tried (`lex_width_probe`, `scalar_width`) pass.

**Fixtures.** A1 has 13 new cases (26 fixtures with their offset-range copies) plus one fixed-range fixture, 351 in all: valid strings with 2-, 3- and 4-byte scalars, and error-4 positions for a lone continuation byte, `\xff`, an overlong lead, a surrogate encoding, scalars cut by the end or by `"`, a bad byte after a good scalar, a string that ends after a full 4-byte scalar (error 17), and `\\` followed by `\xff` (error 14 at the `ff`: the escape rule comes first). One more fixture puts `end` inside a scalar whose bytes go on in the buffer (`22 c2 a2 22`, `end` 2: error 4 at 1), so a check that read past `end` would fail it.

**Consumers** (all four compiler/backend pairs, rc 0):

| Runner | Fixtures per pair |
|---|---|
| json A0 / A1 / A2 / A3 / A4 / A5 | 37 / 351 / 114 / 454 / 6344 / 83 |
| claude A0 / A1 / A2 / A3 / A4 | 272 / 2884 / 2443 / 134 / 270 |

`bench/l3/lib/lex.pruv` is untouched, so its `--lib` users were not run. The lex mutant strings in `json/run_q_mutants.py` still appear exactly once each.

**Measurement (loop only).** The structure difference is not zero, so this is not the gate measurement of RUST_SHAPE_RESET; it compares the reshaped loop and is to be repeated when the cursor form is done. The harness is not tracked in Git. It runs hand `Parser::string`, copied verbatim, against Pruv `string` (a1 root, i.e. `lex::scan_string` behind its `requires` checks) over every string in the buffer. Runs alternate hand / Pruv-rust / Pruv-LLVM, 5 rounds each. Results are ns per byte, median and range:

| Size | Profile | Hand | Pruv (rust) | Pruv (LLVM) | Max RSS |
|---|---|---|---|---|---|
| M 1 MiB | mixed (escapes, Latin-1, Hangul, emoji, CJK) | 0.974 (0.77-1.59) | 0.790 (0.73-0.93) | 0.859 (0.80-1.75) | 2.5 MiB all three |
| M 1 MiB | long ASCII | 0.690 (0.69-0.85) | 0.666 (0.65-0.77) | 0.660 (0.66-0.77) | 2.5 MiB all three |
| L 15 MiB | mixed | 0.850 (0.74-0.90) | 0.762 (0.73-0.85) | 0.828 (0.81-0.94) | 16.5 MiB all three |
| L 15 MiB | long ASCII | 0.837 (0.73-0.85) | 0.727 (0.71-0.74) | 0.747 (0.66-0.90) | 16.5 MiB all three |

L is 15 MiB because the A1 root requires `end <= 16777216`, and the buffer runs slightly past its target size. The Pruv median is at or under the hand median in every cell, and the ranges overlap. Five runs do not show a difference either way. Max RSS is the same for all three, and neither scanner allocates. A first 1 MiB ASCII smoke run read 2.65 vs 0.78 ns/byte. It did not repeat in three back-to-back pairs (0.87/0.95, 1.12/1.04, 0.80/0.91) or in the alternated runs above. Its cause is not known.

## Step 2.5: rows freed by LB7 (ZP, SR sub-range)

LB7 closed ZP (SPEC R-6g6-2) and the sub-range part of SR (R-6g6-3: `string_push_str` / `string_from` take
`subview(t, lo, hi)` with no temporary). This step rewrites the rows that waited only on those. The scalar push and
`proc_args_push` parts of SR, and RU, FM, TK, SL, B-LLVM, ABI / V, stay open; their rows are unchanged.

| Row | Function | Status | Hand line |
| --- | --- | --- | --- |
| L5 | `lines::build_span` | **=**: tape prefix is `copy(subview(tape, 0, m))`, Rust `tape[..m].to_vec()` | `Document { source: data[lo..hi].to_vec(), tape: tape[..m].to_vec() }` |
| text | `text::one_line` | **=**: each kept scalar is `string_push_str(move out, subview(s, i, j))`, Rust `out.push_str(&s[i..j])`; no per-scalar temporary | `qwen/handrust/src/lib.rs:325` |
| args | `args::build_args`, `arg_spans` | unchanged: **SR rest** (`proc_args_push` takes no `subview`) | `for p in argv.split_terminator('\0') { cmd.arg(p) }` |

### Line-by-line structure

`lines::build_span(data, lo, hi, tape, records)` against the hand `Document` construction:

| Item | Hand Rust | Pruv (2.5) | Same? |
| --- | --- | --- | --- |
| Types | `&[u8]`, `&[i64]` in; `Document { source: Vec<u8>, tape: Vec<i64> }` out | `[u8]`, `[i64]` borrowed in; `JsonParse::Parsed(JsonDocument { source: [u8], tape: [i64] })` | = (the `JsonParse` wrapper is the `Result` of the row in step 2.1) |
| Source bytes | `data[lo..hi].to_vec()` | `copy(subview(data, lo, hi))` | = (one exact allocate-and-copy) |
| Tape prefix | `tape[..m].to_vec()` | `copy(subview(tape, 0, m))` | = (was zeroed `alloc(m, 0)` + element loop) |
| Ownership | both vectors moved into the document | `move source`, `move exact` | = |
| Loops | none | none (`check_tape` is the D32 publication check, **P**, step 2.1 json row) | = apart from **P** |
| Allocation sites | 2 exact | 2 exact | = |

`text::one_line(s, max)` against `qwen/handrust/src/lib.rs:325`:

| Item | Hand Rust | Pruv (2.5) | Same? |
| --- | --- | --- | --- |
| Types | `&str, usize -> String` | `str, i64 -> str` (owned `String` built, returned as `str`) | = |
| Loop | `for c in s.chars()`, one pass | `while i < n` stepping `next(s, i)` scalar by scalar, one pass | = |
| Whitespace run | `pending = true` | `p = 1` when `out` is non-empty | = |
| Pending space | `out.push(' ')` | `string_push_str(move out, " ")` (Rust `push_str(" ")`, `&'static str`) | = (one byte copied, no allocation) |
| Kept scalar | `out.push(c)` (encodes `c`, 1-4 bytes) | `string_push_str(move out, subview(s, i, j))` (copies the same 1-4 source bytes) | = in bytes copied and allocations; was one `substr` `String` per scalar |
| Allocation sites | `out` growth only | `out` growth only | = |

No performance was measured: the packages still have open gap rows (B-LLVM in `text`, SR rest in `args`, B-LLVM and U
in `lines`/`access`).

Emitted Rust (qwen `rs_rust`, both compilers alike): `build_span` is `data[lo..hi].to_vec()` and `tape[0..m].to_vec()`
(`get_unchecked` ranges) moved into the document; `one_line` pushes `get_unchecked(i..j)` of `s` with `push_str`.

### Consumer tests (step 2.5)

| Test | Result |
| --- | --- |
| json `run_a0` .. `run_a5` | pass (6/6) |
| mcp `test_impl` | pass (22/22 per pair) |
| qwen `test_impl` | pass (26/26 on py/rust, py/llvm, rs/rust, rs/llvm) |
| supervisor `test_a0` .. `test_a3` | pass (4/4; `test_a3` 35/35 per pair) |
| claude `test_a0` .. `test_a4` | pass (5/5) |
| opencode `test_impl` | pass (199/199 on 4 pairs) |
| codex `test_impl` | pass (136/136 on 4 pairs; rs/llvm rerun alone after the first run was killed) |
| grok `test_impl` (`GROK_COMPILE_WALL=3600`) | pass (254/254 on 4 pairs) |

## Step 2.7: lex cursor form, `parse::record` unit result (RU closed by LB8)

**Split.** LB8 (SPEC R-6g7-1) closed **RU**, so the two forms that waited on it are rewritten. Every `lex` scanner moves the caller's cursor and returns the hand `ParseResult<()>`: `scan_*(input, inout pos, end, ..) -> Result<(), Fault>` (the hand `&mut self.pos`). `Scan` is gone. `Fault { position, kind }` is the one error type of `lex` and `parse` (parse's private copy is gone). `scan_literal` takes the literal like the hand `literal(&[u8])`, and the caller picks it from the first byte. `digits` requires its first digit and is called with `?`, like the hand. `parse::record` returns `Result<(), lex::Fault>` (no `Ok(0)`). `container` keeps `Result<i64, lex::Fault>` because the hand is `ParseResult<i64>`. Callers changed with the API: `parse` (`record`, `scan_records`, `token_at`, `scan`, `scan_document`), `json/a1_root.pruv` (host ABI unchanged; it maps `Ok(())` / `Err(fault)` back to the old return code) and `claude/entry.pruv` (`scan_number` with a `match`, because `is_ok` is contract-only).

| Pruv | Hand Rust | Difference |
|---|---|---|
| `skip_whitespace(input, inout pos, end) -> ()` | `whitespace(&mut self)` | = |
| `scan_literal(input, inout pos, end, literal: str) -> Result<(), Fault>` | `literal(&mut self, literal: &[u8]) -> ParseResult<()>` | = (`str` for `&[u8]`: no byte-string literal; `literal[k]` emits `as_bytes()` indexing) |
| `digits(input, inout pos, end) -> Result<(), Fault>` | `digits(&mut self) -> ParseResult<()>` | = |
| `scan_number(input, inout pos, end) -> Result<(), Fault>` | `number(&mut self) -> ParseResult<()>` | = (`digits(..)?;` ↔ `self.digits()?`) |
| `scan_string(input, inout pos, end) -> Result<(), Fault>` | `string(&mut self) -> ParseResult<()>` | = (loop since step 2.6) |
| `scalar_width`, `validate_utf8` | `utf8_width`, `from_utf8` | = (unchanged) |
| `parse::record(..) -> Result<(), lex::Fault>` | `record(..) -> ParseResult<()>` | = (`Ok(0)` gone; literals and numbers with `?`) |
| `parse::scan_records(input, inout tape) -> Result<(), lex::Fault>` | `record(..)?` + trailing check in `parse` | = |
| `parse::token_at` literal branch | — (Pruv-only, `scan_into`, P8) | picks the literal from the first byte and matches the `Result` once |

Not counted, as in every `=` row of this file: `i64` for `u8`/`usize` (SPEC §6b), `(input, lo, hi)` for a sub-slice, `if` chains and `while` for `match` and `for`. The lex structure difference is 0.

**Codegen difference (not a source difference).** A `f(..)?;` statement on `Result<(), E>` emits `match f(..) { Ok(()) => {} Err(__pruv_try_eN) => { return Err::<(), E>(__pruv_try_eN); } }`, where the hand `?` goes through `Try` and `From` (identity here). Both lower to the same branch on the discriminant. This is recorded, not worked around. No new gap row.

**Mutants.** The lex/parse strings in `json/run_q_mutants.py` follow the new text (`Err::<(), Fault>(..)`, `digits(input, inout pos, end)?;`, `literal[k]`, `crate::lex::Fault`). Each appears exactly once. Run result (`json/mutants.json`): 11 proof-rejected, 14 behavior-detected, no errors (was 10 / 15). The one change is `wrong_literal_position` (`position: pos - k`): it is now rejected by the proof, because every scanner's `ensures` states that an error's position is the cursor.

**Measurement (gate, `scan_string`).** The lex structure difference is 0, so this is the RUST_SHAPE_RESET measurement for the package's hot scanner. The harness is the one of step 2.6 (not tracked in Git): hand `Parser::string`, copied verbatim, against Pruv `string` (the a1 root over `lex::scan_string(input, inout pos, end)`, rebuilt from this step's sources) on every string in the buffer. Runs alternate hand / Pruv-rust / Pruv-LLVM, 5 rounds each (M 20 passes, L 3 passes per run). Results are ns per byte, median and range; Max RSS from `/usr/bin/time -l`:

| Size | Profile | Hand | Pruv (rust) | Pruv (LLVM) | Max RSS |
|---|---|---|---|---|---|
| M 1 MiB | mixed (escapes, Latin-1, Hangul, emoji, CJK) | 1.520 (1.50-2.78) | 1.466 (1.43-1.49) | 1.455 (1.45-1.50) | 2.7 MiB all three |
| M 1 MiB | long ASCII | 0.663 (0.64-0.68) | 0.653 (0.64-0.66) | 0.649 (0.64-0.67) | 2.5-2.6 MiB |
| L 15 MiB | mixed | 1.540 (1.54-1.54) | 1.475 (1.47-1.48) | 1.492 (1.49-1.50) | 17.7 MiB all three |
| L 15 MiB | long ASCII | 0.649 (0.64-0.65) | 0.646 (0.64-0.65) | 0.648 (0.65-0.65) | 16.6 MiB all three |

The Pruv median is at or under the hand median in every cell (0.96-1.00x hand), so the 1.0x floor holds on both backends. Max RSS is the same; no scanner allocates. The machine was not idle (load average about 6 on 10 cores from other worktrees' tests), which is why the mixed numbers are higher than in step 2.6 for all three alike; compare within this table only.

**Consumers** (all four compiler/backend pairs, rc 0):

| Runner | Fixtures per pair |
|---|---|
| json A0 / A1 / A2 / A3 / A4 / A5 | 37 / 351 / 114 / 454 / 6344 / 83 |
| claude A0 / A1 / A2 / A3 / A4 | 272 / 2884 / 2443 / 134 / 270 |
| grok (`--no-ts`) | 254 |
| opencode | 199 |
| codex | 136 |
| mcp | 22 |
| qwen | 26 |

## Step 2.8: rows freed by LB9 (SR `proc_args_push`, TK)

LB9 closed the `proc_args_push` part of SR (SPEC R-6g8-1: `subview(t, lo, hi)` is a direct argument) and TK
(R-6g8-4: `let t = move binder.f;` in a consuming match). This step rewrites the two rows that waited on them. SL
(R-6g8-3) has no row in these files (no `let x: str = "lit"` in `args.pruv` or `json/a2_root.pruv`). FM, RU, B-LLVM,
ABI / V stay open.

| Row | Function | Status | Hand line |
| --- | --- | --- | --- |
| args | `args::build_args` | **=**: `find_byte` for the next NUL, `proc_args_push(move a, subview(argv, start, end))`; Rust `a.arg(&argv[start..end])` after `position` | `command.args(argv.split_terminator('\0'))` (`supervisor/hand/supervise_hand.rs:70`) |
| args | `args::arg_spans` | unchanged in shape: kept for codex `child_args` (spans) and mcp validation; carries the **U8** fact as a contract | `argv.len() > 1048576 \|\| (!argv.is_empty() && !argv.ends_with('\0'))` (`supervise_hand.rs:54-55`) |
| json TK | root `parse_tape` (`json/a2_root.pruv`) | **=**: `let tape: [i64] = move document.tape; return tape;`; Rust `let tape = document.tape;`, LLVM no `@pruv_copy` | `Ok(doc) => doc.tape` |

### Gap U8: a `str` is not known to be UTF-8

The verifier models a `str` as any byte sequence (the `utf8_boundary` fix text says a counterexample str may not be
valid UTF-8). So after a NUL byte at `end`, `is_char_boundary(argv, end + 1)` is not provable, though it holds for every
valid `str` (the byte after an ASCII byte starts a scalar or is the end). Without it, `subview(argv, end + 1, ..)` on the
next turn is refused (`invariant_preserved` for `is_char_boundary(argv, start)`; counterexample `start = end + 1`).

`build_args` does not add a runtime check for it. It states the fact as a contract instead:
`requires forall k in 0..len(argv): argv[k] == 0 ==> is_char_boundary(argv, k + 1)`. `arg_spans` already had the
runtime check (`if !is_char_boundary(argv, i + 1) { return bad; }`, HEAD) and now also `ensures` the same fact when it
returns `[0, ..]`, so mcp `root.pruv` (checks `sp[0]` first) proves it with no source change. The cost of U8 is this
check in `arg_spans`, which the hand does not have. Closing U8 needs a verifier axiom that `str` values are UTF-8, which
is outside this bundle; `docs/RUST_SHAPE_RESET.md` §4 has no U8 row yet.

### Line-by-line structure

`args::build_args(argv)` against `command.args(argv.split_terminator('\0'))`:

| Item | Hand Rust | Pruv (2.8) | Same? |
| --- | --- | --- | --- |
| Types | `&str` in, `Command` args | `str` borrowed in, `ProcessArgs` out (`Command` args in the host) | = |
| Validation | before the loop: size cap and trailing NUL | before the call: caller checks `arg_spans` (`requires len(argv) <= 1048576` + the U8 fact) | = for `build_args`; the U8 runtime check lives in `arg_spans` |
| Split | `split_terminator('\0')` (memchr per piece) | `find_byte(argv, 0, start, n)` (Rust `position`, LLVM `memchr`) | = |
| Push | `cmd.arg(p)` with `p: &str` | `proc_args_push(move a, subview(argv, start, end))` (Rust `&argv[start..end]`) | = (was span packet + `substr` `String` per argument) |
| Last piece | `split_terminator` drops the empty tail | `if end == n { return move a; }` | = on a validated blob (ends with NUL, so the last `find_byte` is `n` only when `start == n`, which the loop test stops) |
| Allocation sites | one per argument inside `arg` | one per argument inside `proc_args_push` | = (was 1-2 span packets + 1 `String` per argument) |

Behaviour on a malformed blob changes: `build_args` used to return no arguments when `arg_spans` failed; now it pushes
the `split_terminator` pieces, as the hand loop would. Its only caller (mcp `root.pruv`) returns before the call on a
malformed blob; supervisor keeps its own `supervisor/args.pruv`.

Root `parse_tape` against `Ok(doc) => doc.tape`:

| Item | Hand Rust | Pruv (2.8) | Same? |
| --- | --- | --- | --- |
| Match | `match parse(input)` by value | `match parsed` on an owned local (consuming) | = |
| Tape | `doc.tape` (partial move) | `move document.tape` (Rust `let tape = document.tape;`) | = (was `copy(document.tape)`; LLVM tape-sized `@pruv_copy`, now none) |
| Error | `[-1, pos, kind]` | `alloc(3, 0)` + three writes | = (unchanged) |

No performance was measured: `args` keeps the `arg_spans` row (U8, codex spans) and `json` keeps B-LLVM and D32.

### Consumer tests (step 2.8)

| Test | Result |
| --- | --- |
| json `run_a0` .. `run_a5` | pass (6/6 on py/rust, py/llvm, rs/rust, rs/llvm) |
| mcp `test_impl` | pass (22/22 per pair) |
| codex `test_impl` | pass (136/136 on 4 pairs; TypeScript oracle 136/136) |
| grok `test_impl` (`GROK_COMPILE_WALL=3600`) | pass (254/254 on 4 pairs; `GROK_TS_TRANSPILER` set for the reference) |

Grok declares `mod args;` but calls nothing from it; it still compiles the package. Supervisor, claude and qwen do not
use `pkgs/args` or `json/a2_root.pruv` and were not run.
