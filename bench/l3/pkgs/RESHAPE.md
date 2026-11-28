# Rust-shape reset step 2.1: shared packages

Seven packages (`json`, `lex`, `bytes`, `text`, `names`, `args`, `tail`) set against the hand Rust that does the
same job. Order of work: json, text, names, args, tail, then the lex and bytes checks. `lex` and `bytes` were
"keep" (structure check only). Gap ids refer to `docs/RUST_SHAPE_RESET.md` §4.

Both backends are a hard constraint: every consumer piece (mcp, qwen, codex, grok, opencode) also builds on LLVM,
where borrowing structs are `codegen/unsupported_type` (gap **B-LLVM**, debt D-LB3). Borrowed views reach LLVM in
bundle B1 (R-6g9-1), so `text` returns `ref str`; no package stores a view in a struct yet.

Columns: Pruv function, hand Rust function, types, ownership, loops, allocation sites. "=" means same as hand.

Rules (main correction, 2026-11-24): done means the Pruv code is structurally identical to the hand Rust, function by
function. A difference is allowed only when a proof removes a check hand Rust keeps. A function that cannot be written in
the hand shape because Pruv lacks a feature is left unchanged and gets a gap row; no workaround versions. Performance is
measured only for a package whose table shows zero differences.

## Status

| Package | Differences left | Measured |
|---|---|---|
| json | 2 (B-LLVM source copy, D32 publication checks; root tape move reshaped in step 2.8) | no |
| text | 0 (`trim`, `clip_bytes` reshaped in B1; `one_line` reshaped in step 2.5) | no |
| names | 0 (`project_name` reshaped in step 2.2b) | no |
| args | 1 (`arg_spans`: span packet kept for codex `child_args` and the U8 check; `build_args` reshaped in step 2.8) | no |
| tail | 0 shape rows (rewritten to the hand `Tail` in step 2.10); per-line differences CW, BV, U8, P7-1 in the step 2.10 table | no |
| lex | 0 (cursor and return form done in step 2.7; `e?;` emits the hand `f(..)?;` since LB-Q) | yes, `scan_string` M and L (step 2.7) |
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
| `trim(s) -> ref str from(s)` | `s.trim() -> &str` | `&str -> &str` | borrow (=) | two passes (=) | none (=) | = since B1 (R-6g9-1: the view lowers to `{ ptr, i64 }` on LLVM). Was an owned `substr` copy. Callers that only need bounds use `trim_span`. |
| `same(a, b)` | `a == b` | `str` | borrow | one pass | none | = |
| `one_line(s, max) -> str` | `qwen/handrust/src/lib.rs:325`: `for c in s.trim_matches(is_es_ws).chars()`, `out.push(c)` per scalar, `push(' ')` per whitespace run | `str -> String` | owned result (=) | one pass (=) | `out` growth only (=) | **changed in 2.5** (LB7, R-6g6-3): the scalar is pushed as `string_push_str(move out, subview(s, i, j))`, Rust `out.push_str(&s[i..j])`, with no temporary (was one `substr` per scalar). = See step 2.5 for the line-by-line comparison. |
| `clip_bytes(s, max) -> ref str from(s)` | `&s[..floor_char_boundary(max)]` | `&str -> &str` | borrow (=) | ≤3 steps back (=) | none (=) | = since B1 (R-6g9-1). Was an owned `substr` copy. |
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

**Rewritten in step 2.10** to the hand shape (see the step 2.10 section). Until then Pruv kept a ring of UTF-16 units
in two loose arrays (`tb`, 12-word `ts` packet) with `tail_bytes` re-encoding into a new buffer, and the decoder pick
`let d = if s == 1 { &mut tl.d1 } else { &mut tl.d2 }` was gap **FM** (language part closed by B5, R-6g9-5).

## lex (`lex/lex.pruv`, hand: `json/hand/hand_json.rs` scanner) — `scan_string` loop reshaped in step 2.6, cursor form in step 2.7

| Pruv | Hand Rust | Types | Ownership | Loops | Allocations | Difference and reason |
|---|---|---|---|---|---|---|
| `struct Fault { position, kind }`, every `scan_*(input, inout pos, end, ..) -> Result<(), Fault>` | `ParseResult<()>` = `Result<(), (usize, i64)>`, cursor in `self.pos` | `i64` pair / `(usize, i64)` | `inout pos` ↔ `&mut self.pos` | — | none | = since step 2.7 (`Scan` is gone; RU closed by LB8). The error is a named struct where hand uses a tuple; same two values. |
| `validate_utf8(input, start, end) -> i64` | `std::str::from_utf8(&input[a..b])` | `[u8]` | borrow | 16-byte ASCII blocks, then per char | none | Same algorithm as std (ASCII fast path, then sequence check); returns the error offset like `valid_up_to`. = |
| `skip_whitespace` | `while matches!(b, b' '\|b'\t'\|b'\n'\|b'\r')` | `[u8]` | borrow | one pass | none | = |
| `scan_literal(input, inout pos, end, literal: [u8])` | `hand_json.rs:67` `literal(&mut self, literal: &[u8])`: `for &expected in literal`, `required()` per byte, error 12 at the first mismatch | `[u8]` (step 2.10; callers pass `"true"`, a static byte view, R-6g4-5, Rust `"true".as_bytes()`) | borrow | `for expected in literal` (P4, step 2.10) | none | = since step 2.10 (was a `str` literal and a `while k < len(literal)` index loop). The caller picks the literal from the first byte like hand `record` (`b't' => self.literal(b"true")?`). |
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
| B-LLVM | `Document<'a> { source: &'a [u8] }` | Borrowed results and locals reach LLVM in bundle B1 (R-6g9-1); borrowing structs stay Rust-backend only (`codegen/unsupported_type` on LLVM) until bundle B2. | `json` source copy (`text::trim`, `text::clip_bytes` closed in B1) |
| SR | `out.push(c)`, `out.push_str(&s[i..j])`, `cmd.arg(&s[i..j])` | No scalar push; `subview(s,lo,hi)` cannot be the direct argument of `string_push_str` / `proc_args_push` (only of a plain `str` parameter of a Pruv fn, R-6g-10). | `text::one_line`, `args::build_args` (+ `arg_spans`) |
| VC | `Vec::with_capacity(n)` then `push` | No reserve-without-fill `[u8]`; `alloc` fills and `truncate` may shrink-realloc (LB5 non-filling growth). | `names::project_name` |
| FM | `let d = if c { &mut x.a } else { &mut x.b };` | No `inout` binding to one of two fields chosen at run time. Closed by B5 (R-6g9-5); `tail` rewritten in step 2.10. | — |
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
| turns | T2, T3 (operation only), T4, T5, T8 | 2.2b: T2 (part), T4, T5, T8; 2.2c: T2 `answer`, T7; 2.4: T2 `put_str`/`write_piece`, T7 copies | — | T1 `grow`, T7, T9, T10 | ABI / V (T0, T1 slabs, T6 incl. growth; T2 writes the slot buffer `buf`, T7 writes `bk.data`, neither a per-turn `Vec`) | — (T1 `empty_buffer` removed in 4.4v, no caller) |
| transport | X0, X3, X4, X5 (scan, growth) | X0 (first pass); 2.2b: X3, X4, X5; 2.2c: X5 growth; 2.4: X5 scan | — | X1, X2, X6, X5 framing | FM (X7) | — |

## jstr (`jstr/jstr.pruv`)

| # | Pruv | Hand Rust | Types | Ownership | Loops | Allocations | Id and reason |
|---|---|---|---|---|---|---|---|
| J1 | `hex`, `step`, `scalar`, `width` | `units(raw)` iterator (`hand_json.rs:335`); `hex4` exists only in `claude/handrust:333` and `grok/perf:309` | `[u8]`, `i64` | borrow | one escape or scalar per call | none | **B** (§4): the hand-shape cursor is a struct holding the borrowed bytes (`struct Units<'a> { raw: &'a [u8], pos }`); a Pruv struct with a `ref [u8]` field is `type/borrow_signature` ("borrowed type or from-list is not allowed here"). Left unchanged (free helpers returning `Step`, same decode work per unit). |
| J2 | `ascii_equal(source,s,e,key)` | query mode 3: `text.is_ascii() && !body.contains('\\')` then slice `==`, else `units(raw).eq(text.encode_utf16())` | `[u8]`, `str` | borrow | one pass, early exit | none | **=** for the ASCII path. Precondition (not in the hand): `key` is ASCII; every caller checks `is_ascii(key)` first (`access.pruv:122`, `:212-217`, `:311`, `json/b_root.pruv:144`) and non-ASCII keys go through `units_equal`. Documented, not a behaviour bug (see note J2 below). |
| J2b | `units_equal(source,s,e,key,klen)` | `units(raw).eq(key)` | `[u8]`, `[i64]` | borrow | one pass, early exit | none | **=** (slow path of mode 3; key already UTF-16). |
| J3 | `string_units(source,s,e) -> [i64]` | mode 5: `units(raw).count()`, `Vec::with_capacity`, `extend(units(raw))` | `[u8] -> [i64]` | owned result | count pass, then decode + push pass | 1 exact (was 2: `alloc(e-s)` + exact copy) | **changed** (LB5 `arr_reserve` + `arr_push`). = now. |
| J4 | `string_utf8`, `string_utf8_plain`, `string_utf8_scanned` | mode 6: `decode_utf16(units(raw))` length sum, then `encode_utf8` per char into `Vec<i64>` | Pruv `[u8]`, hand `Vec<i64>` | owned result | count pass, then `alloc(0,0)` + `arr_reserve(need)` and a decode pass (one shared private `decode_utf8`). Both passes take each escape-free run (up to the next `\\`; in `_utf8` also up to the next byte >= 128) as one segment: the count adds its length, the decode does one `arr_append` (`extend_from_slice`). Escapes are decoded and pushed per encoded byte. `_plain`: `arr_reserve` + one `arr_append` | 1 each | **changed in 2.2c**. The hand changed to the same segments: `utf8_segments` hands each escape-free run to one `extend` and decodes each escape as it meets it, in both the count pass and the copy pass. Escapes: both sides decode one scalar per step (a high surrogate takes the next `\\u` when it is the low half, else U+FFFD) with no look-ahead scan; Pruv pushes its 1-4 bytes, the hand `encode_utf8`s into a 4-byte stack buffer. Neither side allocates (=; the hand's earlier escape-run pre-scan was removed after the 2.2c confirmation). **P**: `string_utf8` (unscanned input) also ends a run at a byte >= 128 and decodes that scalar with `step` (lenient: a lone lead byte keeps its value and continuation bits are not checked), because nothing proves its input is UTF-8. The hand only sees parser-validated input, so its counterpart is `string_utf8_scanned`, whose runs end only at `\\`. = The element type differs on purpose (**W-c** in the hand benchmark only): consumers take `[u8]` bytes, the hand benchmark prints `i64` per byte. `_plain` (no count pass, one `arr_append`) is selected by the parser's plain bit, which the hand does not keep: without escapes the hand still runs both segment passes, each one `position` search (**P**, as P2's plain bit). |
| J5 | `string_text`, `string_text_scanned`, `string_text_plain`, `utf8_to_string`, `adapter_probe` | `String::from_utf8(vec)` (move); adapter `str_adapter` `to_owned` | `[u8] -> Text` | **copy** into a new `String` | validate + copy | Pruv 2 (bytes, then `String`); hand 1 | **=** (step 2.9): the copy variants `string_text`, `string_text_scanned`, `string_text_plain` are removed; every caller uses a `*_owned` variant (`String::from_utf8`, a move). The decode itself stays **P** (two passes, always owned; the hand borrows an unescaped string, gap B). `utf8_to_string` stays for the adapter. `adapter_probe` is **P** (adapter conformance probe, no hand counterpart). |
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
| T1 | `new_state`, `new_slots`, `new_buffer`, `grow` | `VecDeque::new()`, per-turn `String::new()` | packets | owned | fill loops | up-front slabs; `grow` = new buffer + copy | Slabs: **ABI / V** (T0). `grow` is **=** (step 2.9): `take buf` + `resize(move b, ..)` (`Vec::resize`, one realloc, no new buffer); caller `opencode/convert.pruv:202` passes `move buf`. `empty_buffer` (DB7) was removed in 4.4v (no caller). |
| T2 | `put_str`, `put_nl`, `write_piece`, `begin`/`answer` clip | Grok `hand_history.rs`: `&[u8]` clip and `Vec<u8>::extend_from_slice` | `[u8]` | `inout` | one bulk copy each (2.4); `clip_len` scans only for the UTF-8 cut | `begin`, `answer`: none (2.2b/2.2c) | **changed in 2.2b/2.2c**: `put_nl` is one `copy_span` of `"\n\n"`. `begin` and (2.2c) `answer` clip with `clip_len` on the byte range, so there is no temporary. **changed in 2.4**: `put_str` is one `copy_range(inout dst, at, src, 0, n)` from the `str` (R-6g5-1, Rust `copy_from_slice` of `src.as_bytes()`), so `write_piece` is `put_nl` + one bulk copy, as `grok/perf/hand_history.rs:769-770` (`extend_from_slice(&b"\n\n"[..s2])`, `extend_from_slice(&text[..take])`) and `begin` as `:724`. **=** at the function level (2.2b residue **SB** closed by LB6). Destinations: `begin`/`answer` write the fixed slot buffer `buf`; `begin2`/`answer2` (T7) write `bk.data` of the `Store` arena. Neither is the hand's per-turn `Vec`: **ABI / V** (T0, T6). |
| T3 | `skip_user`, `tool` | `skip_user`, `tool` methods (flag / count updates) | `inout st`, `sl` | — | — | none | **=** in operation; shape is ABI / V (T0). |
| T4 | `plan` (203-word offset array) | `turns.iter().rev()` with a running byte total | `[i64]` | owned | one pass | none (2.2b) | **changed in 2.2b**: `plan` returns `Layout { kept, total, text_len }` (two passes, no array); the `finish*` writers walk a cursor down from `text_len` (`finish_inplace` up). = |
| T5 | `finish` | `Collector::finish(self) -> Vec<Turn>` (moves) | `Collected` | **copy** of `turns`/`text` at both returns (`turns.pruv:356`, `:391`) | — | +2 copies | **changed in 2.2b**: returns `move turns`, `move text` (no copy). = No caller change was needed. |
| T6 | `new_store`, `new_store_hint`, `reserve`, `move_bytes` (`Store` arena) | per-turn `String` allocation | `Store` | `inout` | compaction loop | arena + compaction | **ABI / V** consequence: one byte arena instead of owned `String`s. Growth inside `reserve` was **IF** (§4), closed by LB6 (`arr_reserve(inout bk.data, n)` now emits `bk.data.reserve_exact(n)`). **Unchanged in 2.4**: `Store` keeps `len(data)` as its capacity and writes by index at `bk.end` into zeroed bytes (`begin2`/`answer2`/`begin3`/`answer3`, and the `bk.end <= len(bk.data)` contracts in `qwen/import.pruv`, `grok/entry.pruv`, `grok/line.pruv`, `grok/read.pruv`). A capacity-only `arr_reserve` does not give that room; using it needs the arena turned Vec-shaped (`len == end`, appends), and the hand has no arena at all (per-turn `String`). Remaining reason: **ABI / V**. |
| T7 | `begin2`, `answer2` (`begin3`, `answer3`, `finish2` removed: no caller) | same methods | packets + `Store` | `inout` | bulk copy (2, since 2.4; `clip_len` scans for the UTF-8 cut) / span copy (3) | arena | **W-c**: store-backed variants for claude/qwen/grok. In 2.2c, `begin2`/`answer2` clip with `clip_len` and copy with `put_str` (no temporary string). **changed in 2.4** through T2: the copies are one bulk copy each (`hand_history.rs:724`, `:769-770`); `begin3`/`answer3` already copy spans. Remaining: **W-c** (store-backed variants) and **ABI / V** (arena). |
| T8 | `clip_span`, `put_nl_span`, `put_pad`, `write_span` | `clip` + `push_str` on a borrowed slice | `[u8]` | borrow / `inout` | `copy_span` (2.2b) | none | **changed in 2.2b**: `put_nl_span`, `put_pad` (one `copy_span` of the 4-byte pad scalar), `write_span` copy in bulk. = |
| T9 | `new_refs`, `begin_ref`, `answer_ref` | none (hand owns `String`s) | `[i64]` refs | `inout` | — | one refs array | **W-c**: reference-only variant for a zero-copy consumer; no hand counterpart (hand copies). |
| T10 | `text_layout`, `finish_into`, `finish_inplace` (`finish_ref` removed in 4.4v: no caller) | `finish` then the consumer serialises `Vec<Turn>` | `Placed` | `inout wire` | per turn | none / turns array | **W-c**: writes straight into the consumer's output; the hand builds `Vec<Turn>` first. Consumer API. |

## transport (`transport/transport.pruv`)

| # | Pruv | Hand Rust | Types | Ownership | Loops | Allocations | Id and reason |
|---|---|---|---|---|---|---|---|
| X0 | `receive_grow` chunk read | `read_pipe`: `pipe.read(&mut buf)` into `[0; 65536]`, `buf[..n]` | `[u8]` | `inout chunk` | copy | none | **=** (changed in 2.2 first pass: per-byte copy → one `copy_range`). |
| X1 | `live`, `stdin_closed`, `send` | `child.stdin.write_all` on a writer thread | `Loop` moved through | move | — | none | **W-c**: event-loop handle instead of threads. `send` takes a deadline `ms` with no hand counterpart (hand blocks). Shared API of mcp, codex, grok. |
| X2 | `close`, `poll`, `expired` | `child.kill`/`wait`, `recv_timeout` | `Loop`, `inout st` | move | `expired`: at most 8 polls | none | **W-c** (same API as X1). The first table's `=` was wrong: the hand has no poll/expiry loop. |
| X3 | `grow_line(line, cap)` | `read_line` into a reused `String` (`clear`, capacity kept) | `[u8]` | `take line` (2.2b) | — | `let mut b = move line; b = resize(b, n, 0)`: one in-place `reserve_exact` + `resize` (Vec growth) | **changed in 2.2b**: `grow_line(take line, cap)`, the one caller `grok/rpc.pruv:750` passes `move aux` and assigns it back. = for the growth itself. The caller is not a line reader: `aux` holds the error-reply detail and the code bytes already written, which `resize` keeps (**W-c**: error-reply storage API; the hand Grok borrows the code range). |
| X4 | `receive_grow` event bytes | `line.as_bytes().to_vec()` once | `[u8]` | `ev_proc_read_into(move q, slot, 1, inout chunk, inout got)` | — | 1 (=) | **changed in 2.2b**: reads the event bytes straight into `chunk`. = |
| X5 | `receive_grow` framing | `BufReader::read_line` (newline kept, partial last line returned at EOF) | `[u8]` | `inout line` | scan to the newline (or cap), then `truncate` to the filled length and one `arr_append` of the chunk | growth only (amortized inside `arr_append`, no zero fill) | **changed in 2.2c**: growth is the hand's append (the 2.2b `resize` zero fill is gone). **changed in 2.4**: the newline scan is one `find_byte(chunk, 10, begin, stop)` (R-6g5-3; Rust `position`, LLVM `memchr`) with `stop` the buffered end bounded by the room under cap, as `read_line` (`codex/hand/reader.rs:30`, `memchr` in `read_until`). **=** for scan and growth (2.2c residue **MC** closed by LB6). Framing semantics differ on purpose (**W-c**): Pruv appends raw bytes with no UTF-8 check (consumers parse the line as JSON, which checks it), where `read_line` into a `String` validates and returns its error with `?`; Pruv excludes the newline and drops/flags a partial last line at EOF per the `st` status (-3 at cap kept); consumers rely on that. |
| X6 | `Reader { status, filled, at, exit, end }` (was `st: [i64]` 5-slot state) | locals of the reader (`codex/hand/reader.rs` `Rpc { output, line }`, blocking `read_line`) | named struct | `inout` | — | none | **changed in 2.12**: the packet is a named struct built by `transport::reader_new`; every consumer (mcp, codex, grok) uses the field names. The chunk cursor `at..end` and `status` stay as fields because the event loop returns between reads (**W-c**, see X1). |
| X7 | stderr `crate::tail::push(inout tl, 1, err)` | `tail.push(1, &bytes)` | `tail::Tail` | `inout tl` (= `&mut tail`) | — | none | = since step 2.10 (was `tail_push(inout tb, inout ts, 2, ..)`, FM). |

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
| FM | ring tail picked by stream | `inout` binding to a run-time chosen field (closed by B5; X7 rewritten in step 2.10) | — |

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
| A1 | `array_at(doc, id, index)` | `Document::array_at`: `index >= count` is a miss, then `index` steps along `next` | `i64` record id | borrowed doc | `index` link steps | none | **changed in 2.3**: was a walk over the whole array (`j < stop`) with a per-element parent check; now the count decides the miss and the loop runs `index` steps, the parent/valid check once on the hit; step 2.9 writes the walk as `for n in 0..index` (P4). Same results on every tape that passes `check_tape` (the count equals the direct children reached along `next`), as the hand trusts the count. = Callers: `json/a4_root.pruv:22`, `json/q_root.pruv:85`, `mcp/perf/perf_root.pruv:35` (`b_root` has its own `array_at`) |
| A2 | `field_units(doc, id, key: [i64])`, `find` | mode 3 non-ASCII matcher `units(raw).eq(text.encode_utf16())` | key as `[i64]` UTF-16 units | borrowed | members of the object, last match wins | none | **U**: the key is a caller-built `[i64]`; the hand compares against `encode_utf16()` of a `&str` iterator. Callers: `json/q_root`, `a4_root` |
| A3 | `field(doc, id, key: str)`, `find_ascii`, `may_match`, `is_ascii` | `Document::field` + mode 3 fast path `text.is_ascii() && !body.contains('\\')` | `str` key | borrowed | members, last match wins | ASCII key: none (=). Non-ASCII key: `alloc(len(key))` units | **=** for the ASCII path (every product key is ASCII). The non-ASCII path builds a key unit array: **U** |
| A4 | (removed in step 2.10) `fields_into(doc, id, n, k0..k7, inout out)` + `hit`, `slot_error`, `fill_error`, `field_if_used`, `last_match` | one `get(key)` per key | — | — | — | — | = since step 2.10: the three claude callers (`entry::root_of`, `entry::block_of`, `import.pruv` assistant `message`) call `field(doc, id, key)` once per key, as hand `get_str(e, key)` / `is_true(e, key)`; the multi-key API and its helpers are gone |
| A5 | `string_units(doc, id) -> Units` | query mode 5 `units(raw).collect::<Vec<u16>>()` | `Units { status, units: [i64] }` | result owns the units | one decode | one (the units) | **changed in 2.3**: miss paths built an empty array and returned `copy(empty)`, the hit returned `copy(u)`; now `alloc(0, 0)` and `move u`. = Callers: `codex/rpc.pruv:418,430`, `codex/root.pruv:276`, `grok/rpc.pruv:706`, `grok/path.pruv:328`, `mcp/perf/perf_root.pruv:37` |
| A6 | `string_text`, `string_text_scanned` | `get_str` → `String` | `jstr::Text` | result owns | decode | one | **=** (step 2.9): `string_text` / `string_text_scanned` removed; callers use `string_text_owned` / `string_text_scanned_owned` (`get_str` then `String::from_utf8`, no copy). Callers: `qwen/entry.pruv`, `qwen/chain.pruv`, `json/{a4,b,q}_root.pruv`, `grok/util.pruv:106` |
| A7 | `string_is(doc, id, lit)` | `get_str(..) == Some(lit)`: `Cow::Borrowed` for an escape-free string, decode only with escapes | `i64` | borrowed | ASCII: compare; plain: compare in place; escaped: decode then compare | ASCII and plain: none. Escaped: one decode buffer (= `Cow::Owned`) | **changed in 2.3**: a non-ASCII literal against an escape-free string (tape plain bit) decoded into a fresh `[u8]` before; now compares the source bytes in place. = for documents from `parse::scan_into` (`build_span`: `grok/read.pruv:360`, `qwen` `parse_span`). Documents from the owned parser (`parse::parse`, `parse_owned`/`parse_copy`) always carry plain bit 0 (`parse.pruv:222`), so a non-ASCII literal still decodes there; writing the bit is a `parse` row, outside this step. Static callers pass ASCII literals (`claude/entry.pruv:238,339` through `field_is`, `claude/import.pruv:356..541`, `codex/item.pruv:142..166`), which never reach the decode |
| A8 | `raw_is`, `view_root_type(data, lo, tape, records)` | mode 3 fast path on the raw bytes | scratch tape view | borrowed | members of the root | none | **W-c**: reads a scratch tape before a document exists (`grok/read.pruv:359`); the hand builds the `Document` first. Consumer step |
| A9 | `boolean`, `number` | `raw_output`: `parse::<i64>` within ±(2^53-1), `-0` kept | `i64`, `Number` | borrowed | digits | none | **=** Callers: `codex/rpc.pruv:416`, `grok/rpc.pruv:668`, `grok/summary.pruv:154` |
| A10 | `string_text_owned`, `string_text_scanned_owned` | `String::from_utf8` of the decoded bytes | `Text` | result owns | decode | one | **=** |
| L0 | `jsonl_next(data, offset, is_final)` | `split_inclusive(b'\n')` + `strip_suffix(b"\n")`/`b"\r"` | `JsonlStep` | borrowed | scan to newline | none | **=** Caller: `claude/framing.pruv:28` |
| L1 | `parse_range(data, lo, hi)` | `parse(&data[lo..hi])`, borrowed `Document` | `JsonParse` | document owns a copy of the range | UTF-8 check + scan (in `parse_owned`) | one exact copy of the range, the tape | **changed in 2.3**: was a zeroed `alloc(n, 0)`, a byte loop, a scan, then a second `copy(owned)`; now `copy(subview(data, lo, hi))` moved into `parse_owned` (as `parse_copy`). The remaining copy is **B-LLVM** (owned source on LLVM). Callers: `qwen/entry.pruv:12`, `grok/read.pruv:362` (fallback) |
| L2 | `parse_owned(take data, hi)` | `parse(&data)` on an owned buffer | `JsonParse` | takes the buffer; `truncate` in place | UTF-8 check + scan | the tape | **=** Caller: `grok/rpc.pruv:628` |
| L3 | `parse_copy(data, lo, hi)` | `parse(&line[lo..hi])` | `JsonParse` | one exact copy | as L2 | copy + tape | **B-LLVM** (copy kept for the owned source). Callers: `codex/rpc.pruv:82`, `mcp/root.pruv:118` |
| L4 | `scan_span`, `parse_span` | none (hand parses each line into a fresh `Document`) | caller scratch `inout tape`, `inout stack` | `inout` | scan | none in the scan | **=** (step 2.9): `scan_span` / `parse_span` removed; qwen `window_wire` parses each line with `entry::parse_line` (a fresh document per line, as the hand). grok never called them (its tape path is `build_span` after `parse::scan_into`, a P8 row). |
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

**Codegen difference: closed.** A `f(..)?;` statement on `Result<(), E>` once emitted `match f(..) { Ok(()) => {} Err(__pruv_try_eN) => { return Err::<(), E>(__pruv_try_eN); } }`. Both Rust backends now emit the hand `f(..)?;` (and `let x: T = f(..)?;` for a value), so `parse::record` and `parse::container` read like the hand parser. No gap row.

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
malformed blob; the supervisor (since step 4.2) validates with `arg_spans` before the call as well.

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

## Bundle B1: borrowed views on LLVM (`text::trim`, `text::clip_bytes`)

SPEC R-6g9-1 lowers `ref str` / `ref [u8]` locals, arguments and `from` results to `{ ptr, i64 }` on LLVM, never
freed. The two `text` rows held only by B-LLVM are rewritten in the hand shape: `trim(s: str) -> ref str from(s)` and
`clip_bytes(s: str, max: i64) -> ref str from(s)` return `subview(s, ..)` (hand `&s[..]`), with no `substr` copy and no
`! alloc`. Contracts are unchanged.

Callers bind the result to a `ref str` local, the hand `let t = s.trim();`. Where the argument was a temporary
(`trim(str_at(..))`), it is bound to a local first, as the hand borrow needs a named owner. Where the hand keeps an
owned string (`.to_string()`), the caller writes `copy(..)`. Callers: `pkgs/time/time.pruv`, `codex/item.pruv`, `codex/list.pruv`, `codex/plan.pruv`,
`codex/root.pruv`, `grok/summary.pruv`, `opencode/path.pruv`, `opencode/plan.pruv`. Each removes one allocation per
call. (claude keeps its own `claude/text.pruv` copy and is not a consumer of `pkgs/text`.) Still open: the `json` source copy (borrowing struct, bundle B2).


The codex list title is a helper `title(d, t)` that returns early, so the name path builds one `one_line` string like the
hand. The grok mutant M08 (`grok/entry.pruv`) now reads `copy(crate::text::trim(inner))`, because `trim` returns a view.

**Consumers** (all four compiler/backend pairs, rc 0):

| Runner | Fixtures per pair |
|---|---|
| codex | 136 |
| grok (`--no-ts`) | 254 |
| opencode | 199 |
| qwen | 26 |
| claude A0 / A1 / A2 / A3 / A4 (own `claude/text.pruv`, unchanged) | 272 / 2884 / 2443 / 134 / 270 |

## Step 2.9: W-c rows whose consumers can move, and P4 range loops

There are no plain **W** rows left in this file; every open row is **W-c** (waits on a consumer), **P**, or a gap. This
step takes the W-c rows whose consumers can switch now, and the two counted loops that P4 (`for i in a..b`, SPEC §6q)
writes in the hand form. No performance was measured.

| Row | Function | Status | Hand line |
| --- | --- | --- | --- |
| J5 | `jstr::string_text`, `string_text_scanned`, `string_text_plain` | **removed**: callers use the `*_owned` variants (`utf8_valid` then `str_from_utf8(bytes)`, which consumes the buffer) | `String::from_utf8(bytes)` on the unescaped `Vec<u8>` (`claude/handrust/src/lib.rs:383-407`) |
| A6 | `access::string_text`, `string_text_scanned` | **removed**: callers use `string_text_owned` / `string_text_scanned_owned` | `get_str` (`claude/handrust/src/lib.rs:485-488`) |
| L4 | `lines::scan_span`, `parse_span` (and qwen `entry::parse_line_span`) | **removed**: qwen `window_wire` calls `entry::parse_line(data, start, end)`; the caller scratch `tape` (24576 words) and `stack` (516 words) are gone | the hand parses each line with `serde_json::from_slice::<Value>` (`qwen/handrust/src/lib.rs:690-723`) |
| T1 | `turns::grow` | **=**: `take buf`, `b = resize(b, 13116800, 0)`; caller `buf = grow(move buf, ..)` | `buf.resize(n, 0)` (`Vec::resize`, at most one realloc) |
| lex | `scan_string` `\u` digits | **=**: `for k in 0..4` | `for _ in 0..4` (`json/hand/hand_json.rs:127`) |
| A1 | `access::array_at` walk | **=** for the loop: `for n in 0..index` (the body keeps its 2.3 tape guards) | `for _ in 0..index { child = self.record(child)[3]; }` (`hand_json.rs:311`) |

### Line-by-line structure

`jstr::string_text_owned` / `access::string_text_owned` against `String::from_utf8`:

| Item | Hand Rust | Pruv (2.9) | Same? |
| --- | --- | --- | --- |
| No escape | borrows the source slice (no allocation) | always builds an owned `[u8]` | **P**, unchanged by this step: a borrowed result from the document needs gap **B** |
| Decode | one pass, `Vec<u8>` started at the first escape | two passes (count, reserve once, then write) in `string_utf8*` (`jstr.pruv:202-310`) | **P**, unchanged by this step |
| To text | `String::from_utf8(v)` moves `v` | `utf8_valid(b)` then `str_from_utf8(b)` (consumes `b`) | = (the removed variants called `utf8_to_string`, a second buffer and a copy) |
| Allocation sites (escaped string) | one (the `Vec`) | one (the `[u8]`) | = (was two) |

qwen `window_wire` per line against the hand `parse_line`:

| Item | Hand Rust | Pruv (2.9) | Same? |
| --- | --- | --- | --- |
| Filter | parses only when the first non-blank byte is `{` | parses every entry, then checks the root kind | **P**, unchanged by this step |
| Parse | `serde_json::from_slice::<Value>(line)` on the borrowed slice, a fresh value | `entry::parse_line` -> `lines::parse_range`: copies the line once, then a fresh tape | **P**: the line copy is gap **B** (borrowed source in the document); tape vs `Value` is **V** |
| Scratch | `scratch: &mut Vec<u8>` reused only by the slow rewrite path (`parse_slow`) | none (was caller `tape` + `stack` passed `inout` to `parse_span`) | = on the common path (the rewrite path is a separate qwen row) |

`turns::grow` against `Vec::resize`:

| Item | Hand Rust | Pruv (2.9) | Same? |
| --- | --- | --- | --- |
| Signature | `&mut Vec<u8>` (or by value) | `take buf: [u8]` -> `[u8]` | = (ownership moves in and out; no borrow kept) |
| Body | `buf.resize(n, 0)` | `b = resize(b, 13116800, 0)` | = (was `alloc` of a new buffer plus a copy of the old bytes) |
| Allocation sites | at most one realloc | at most one realloc | = (was alloc + copy + free) |

`lex::scan_string` `\u` and `access::array_at`:

| Item | Hand Rust | Pruv (2.9) | Same? |
| --- | --- | --- | --- |
| `\u` loop | `for _ in 0..4` | `for k in 0..4`, invariant `pos >= hex_start` (feeds the outer loop's `decreases end - pos`) | = (was `while k < 4` with a manual counter and `decreases 4 - k`) |
| `array_at` loop | `for _ in 0..index` | `for n in 0..index` | = (was `while` with a counter) |
| `array_at` body | `child = self.record(child)[3]` | `j = doc.tape[6 * j + 3]` after `j >= stop` and `next <= j` guards | **P** (guards unchanged from 2.3: they keep the tape index in range without a `check_tape` fact; the hand trusts the tape) |
| Loop variable | `_` | named (`k`, `n`); unused in the body | = in code; Rust emits `for k in 0i64..4i64` |

### Gap rows freed since step 2.8, and what this step did with them

| Gap / feature | Freed by | This step |
| --- | --- | --- |
| P4 range `for` | P4 | used for the two counted loops above; the other `while` loops in the touched modules are cursor loops (the cursor moves by data), like the hand `while self.pos < ..`. `tail::tail_push` steps by one and is left for the `tail` bundle |
| U (`[u16]` keys) | P2 (owned and borrowed `[u16]`) | partly freed; not taken: `jstr` J8 and `access::field_units` (A2) take or return `[i64]` unit arrays that consumers build or read (A3 `field` takes a `str` and builds the units inside), so moving them to `[u16]` is a consumer API change |
| FM | B5 (place bindings `let d = inout P`) | language part closed; the `tail` decoder-pick rewrite is a package rewrite left to a later bundle |
| §6b integer widths | P1-P3 | not taken: changing `i64` record fields to narrower types touches every tape consumer |
| SX | LB6 (step 2.4) | nothing left |

### Consumer tests (step 2.9)

Pairs are rs/rust and rs/llvm (pruvc-rs). The Python compiler `pruvc/` is frozen (PF) and has no P4 `for`
(`parse/reserved_name` on `for`), so py pairs are not run; the runners now take `--vs-py` for them.

| Runner | Fixtures per pair (rs/rust, rs/llvm) |
| --- | --- |
| json `run_a0` .. `run_a5` | 37 / 351 / 114 / 454 / 6344 / 83, all pass |
| claude `test_a0` .. `test_a4` | 272 / 2884 / 2443 / 134 / 270, all pass |
| grok `test_impl --no-ts` (`GROK_COMPILE_WALL=3600`) | 254, all pass |
| opencode `test_impl` | 199, all pass (TypeScript `expected` 199/199) |
| codex `test_impl` | 136, all pass (TypeScript oracle 136/136) |
| mcp `test_impl` | 22, all pass |
| qwen `test_impl` | 26, all pass |

Supervisor does not use the changed packages and was not run. Nothing was measured.

### Rows still open after step 2.9

- `access` A4 (claude multi-key lookup to per-key `field`): consumer change in claude.
- grok `read::window` scratch `tape` / `stack` and the `build_span` path (P8, A8): needs the grok read rewrite.
- `lex::scan_literal` taking a `[u8]` literal, `tail_push` as a `for`: small, not in this step.
- `turns` T7, T9, T10 and `transport` X1-X6: consumer API and behaviour, unchanged.
- Gap rows B, B-LLVM, V, ABI, U, U8, S: unchanged.

## Step 2.10: `tail` in the hand shape, `scan_literal` on `[u8]`, claude per-key lookups

Hand counterparts: `supervisor/hand/core_hand.rs` (`Decoder`, `Tail`) for `tail`; `json/hand/hand_json.rs:67`
`literal` for `lex::scan_literal`; `claude/handrust/src/lib.rs` `get_str` / `is_true` (one lookup per key) for `access`
A4. Language used: P4 `for`, P5 `continue`, P7 char literals and `char(u32(..))`, B5 place bindings
(`let d = inout if ..`, R-6g9-5), field move-and-restore (R-6e-16).

### tail (`tail/tail.pruv`) against `core_hand.rs`

| Hand Rust | Pruv | Types | Ownership / borrow | Loops | Alloc / copy | Difference and reason |
|---|---|---|---|---|---|---|
| `struct Decoder { remaining: u8, cp: u32, lower: u8, upper: u8 }` | `struct Decoder { remaining, cp, lower, upper: i64 }` | `i64` for `u8`/`u32` | value | — | — | Field widths: `Decoder` is reachable from the `pub` `Tail` and `push`, and shifts (`<<`) are on `i64` only. Same values |
| `struct Tail { decoders: [Decoder; 2], bytes: Vec<u8>, units: usize, limit: usize }` | `struct Tail { d0, d1: Decoder, bytes: [u8], units, limit: i64 }` | `[u8]` = `Vec<u8>` | owned | — | — | **P7-1**: no `[T; N]`, so the two decoders are two fields; `decoders[stream]` becomes `inout if stream == 0 { tl.d0 } else { tl.d1 }` |
| — (unchecked) | `pred fn dec_ok`, `tail_ok` | — | — | — | — | Contracts only. `dec_ok` lists the decoder states the hand reaches, which proves `char::from_u32(cp)` succeeds |
| `Decoder::new()` | `decoder_new()` | — | value | — | — | = |
| `Tail::new(limit)`: `Vec::with_capacity(6 * limit + 4)` | `new(limit)`: `alloc(0, 0)` + `arr_reserve(inout bytes, 6 * limit + 4)` | — | result owns | — | one reserve | = (Rust: `vec![]` + `reserve_exact`) |
| `trim`: `while units > limit \|\| len - start > 3 * limit`, width from the lead byte (`0..=0x7f` 1, `..=0xdf` 2, `..=0xef` 3, else 4), `units -= 2` for 4 else 1 | `trim`: same condition and table as an `if` chain | `i64` | `inout tl` (= `&mut self`) | one `while` | none | **U8**: the loop also stops at `start == len`, clamps `start` to `len` and `units` to 0, and sets `units = 0` when everything is dropped. On valid UTF-8 bytes with `units` their UTF-16 length (what hand trusts) none of these fires; the verifier knows neither fact |
| `copy_within(start.., 0)` + `truncate(len - start)` on `self.bytes` | `let bytes = move tl.bytes; tl.bytes = shift_front(move bytes, start);` with `copy_within(inout b, ..)` + `truncate` inside | `[u8]` | field moved out and restored in the next statement (R-6e-16); Rust `mem::take` + call | — | none (`ptr::copy` in place) | **CW** (new): `copy_within` / `inout` builtins take a whole array name, not a field place (`type/inout_argument`), and `tl.bytes = f(move tl.bytes)` is `type/array_alias`. The buffer moves through a `take` helper and back, no copy |
| `append(ch)`: `ch.encode_utf8(&mut [0; 4])`, `extend_from_slice`, `units += ch.len_utf16()`; trim past `2 * limit` units or `6 * limit` bytes | `append(inout tl, ch: char)`: 1-4 `arr_push` by `u32(ch)` range, `units += 1` or 2; same trim test | `char` | `inout tl` | — | pushes into reserved capacity | **P7-1**: no fixed scratch array for `encode_utf8`, so the bytes are pushed one by one (same bytes) |
| `push(stream, bytes)`: `for &byte in bytes`, `let decoder = &mut self.decoders[stream];` | `push(inout tl, stream, bytes: [u8])`: `for byte in bytes`, `let decoder = inout if stream == 0 { tl.d0 } else { tl.d1 };` | `[u8]` borrowed | place binding (B5) | P4 `for` | none | = (stream index 0/1 like hand; was 1/2) |
| continuation: `cp = (cp << 6) \| (byte & 0x3f)`, `char::from_u32(cp).unwrap()` | `cp = (cp << 6) + (byte & 63)`, `char(u32(cp))` proved | `i64` | — | — | — | **BV** (new): with `\|` the solver gives up on the char range of the finished code point (`verify_unknown`); the low six bits of `cp << 6` are zero, so `+` gives the same value. The `unwrap` check is gone (proved) |
| mismatch: `*decoder = Decoder::new(); self.append(U+FFFD)`, then the byte again as a lead | same, `decoder = decoder_new(); append(inout tl, '\u{FFFD}')` | — | place binding | — | — | = |
| lead `match byte { 0..=0x7f, 0xc2..=0xdf, 0xe0, 0xed, 0xe1..=0xef, 0xf0, 0xf4, 0xf1..=0xf3, _ }` | `if` chain in the same order; ASCII `append(char(u8(byte)))`, others set `remaining`, `mask`, `lower`, `upper`, then `slot = Decoder { .. }` through a second place binding | — | — | — | — | No integer range patterns in `match`; same arms in the same order |
| `finish(mut self) -> Vec<u8>`: `for stream in 0..2 { if remaining != 0 { append(U+FFFD) } }`, `trim`, return `self.bytes` | `finish(take tl) -> [u8]`: `let mut t = move tl;`, `for stream in 0..2`, `trim(inout t)`, `let bytes = move t.bytes; return move bytes;` | — | consumes the tail | P4 `for` | none | = (`take` + `let mut` rebinding is `mut self`; the remaining count is read by an `if` on `stream` instead of `decoders[stream]`, P7-1) |

Checked outside the runners: `work`-local root calling `new` / `push` (alternating streams per chunk) / `finish`,
compiled with pruvc-rs (Rust), run against `core_hand.rs` `core()` on 200,000 random inputs (lengths 0-63 mixing
valid, truncated and invalid UTF-8; limits 1-8; chunks 1-9): 0 differences.

Callers rewritten (the old `tail_buf` / `tail_state` / `tail_push(.., 2, ..)` / `tail_flush` + `tail_bytes` API is
gone): `transport::receive_grow` (X7), `codex/root.pruv` + `codex/rpc.pruv`, `grok/rpc.pruv`, `mcp/root.pruv`. Each
holds `let mut tl: crate::tail::Tail = crate::tail::new(1024);`, passes `inout tl`, and states
`crate::tail::tail_ok(tl) && tl.limit == 1024` where the packet bounds were. mcp reads the tail once at the end:
`let err: [u8] = crate::tail::finish(move tl);` (hand `tail.finish()`), replacing `tail_flush` + `tail_bytes`. The
old `tail_bytes` kept the last `limit` UTF-16 units and dropped a low surrogate cut from its pair; hand `trim` drops
whole characters until at most `limit` units remain, which is the same text.

### lex and access

| Hand Rust | Pruv | Difference |
|---|---|---|
| `literal(&mut self, literal: &[u8])`: `for &expected in literal` | `scan_literal(input, inout pos, end, literal: [u8])`: `for expected in literal`, invariant `pos - old(pos) == index(expected)`; per byte `if pos == end` (error 5, hand `required()?`), `if input[pos] != expected` (error 12), `pos += 1` | = (callers in `parse.pruv` unchanged: a string literal argument is a static `[u8]` view; `end` is the P7 borrowed-input bound; `requires 1 <= len(literal)` is a contract only, every caller passes a 4- or 5-byte word) |
| claude `get_str(e, "type")`, `is_true(e, "isSidechain")`, `msg.and_then(\|m\| get_str(m, "model"))`, block `get_str(b, "type")` / `"text"` | `crate::access::field(doc, id, key)` per key at the three sites; `entry::root_of` still returns its 8-slot array, now filled by seven `field` calls | = per key. `root_of` keeps the array its callers (`import::consume_entry_of`, `user_text_of`) take; each slot equals the old one-pass slot. `fields_into`, `hit`, `slot_error`, `fill_error`, `field_if_used`, `last_match` removed (no other caller) |

### Gap rows found in this step

| Id | Hand Rust form | What Pruv cannot express | Where it shows |
|---|---|---|---|
| CW | `self.bytes.copy_within(start.., 0)` on a field | `copy_within` and the other `inout` array builtins take a whole array name, not a field place; `tl.bytes = f(move tl.bytes)` is `type/array_alias` | `tail::trim` (moves the field through `shift_front`, no copy) |
| BV | `(cp << 6) \| (byte & 0x3f)` feeding `char::from_u32` | with `\|`, the char-range obligation of the result times out (`verify_unknown`); `+` proves | `tail::push` |

### Consumer tests (step 2.10)

Pairs are rs/rust and rs/llvm (pruvc-rs), no `--vs-py`.

| Runner | Fixtures per pair (rs/rust, rs/llvm) |
| --- | --- |
| json `run_a0` .. `run_a5` | 37 / 351 / 114 / 454 / 6344 / 83, all pass |
| claude `test_a0` .. `test_a4` | 272 / 2884 / 2443 / 134 / 270, all pass |
| grok `test_impl --no-ts` (`GROK_COMPILE_WALL=3600`) | 254, all pass |
| opencode `test_impl` | 199, all pass |
| codex `test_impl` | 136, all pass (TypeScript oracle 136/136) |
| mcp `test_impl` | 22, all pass |
| qwen `test_impl` | 26, all pass |

Supervisor does not use the changed packages and was not run. Nothing was measured.

### Independent structure comparison

Before the commit a separate read of the same files against the same hand sources (Codex gpt-6.2-astra, high) found
no difference beyond the tables above. Points it raised and where they stand:

- `tail_ok` bounds bytes and units separately and does not tie `units` to the UTF-16 length of `bytes`: that is U8
  (the `trim` clamps above).
- `push` takes `stream` 0 or 1 as a contract; the hand indexes `decoders[stream]` and panics on anything else.
  Contract for a run-time check, as everywhere in Pruv.
- The process read in `transport::receive_grow` / codex `receive` still asks for stderr as fd 2; only the decoder
  number passed to `push` moved from 2 to 1.
- mcp `finish` versus the old `tail_flush` + `tail_bytes`: same final `last_line` bytes for the same stderr input
  (decoder output keeps bytes at most 3 x units, so only the unit bound cuts).
- `supervisor/core_root.pruv`, `supervise.pruv` and `a1_root.pruv` used their own `supervisor/btail.pruv` until
  step 4.2 moved them onto this package (`supervisor/RESHAPE.md`); `lib/tail.pruv` is the old library copy.

### Rows still open after step 2.10

- grok `read::window` scratch `tape` / `stack` and the `build_span` path (P8, A8, and W-c `scan_into`). The hand
  (`grok/perf/hand_history.rs:912-936`) reuses one `Entry`, resets it per line and parses the line straight into it
  (`e.ty` from the `"type"` value, then `col.feed(&entry)`); it builds no `Document` and has no tape fallback. Pruv has
  the DB45 one-pass reader (`grok/line.pruv`, scratch `f` / `d`) with the tape path behind it (`read::line_step`:
  `parse::scan_into`, `access::view_root_type`, `lines::build_span`, `entry::feed`). Matching the hand means one
  `Entry` parser replacing both `line.pruv` and the tape path, and `entry::feed` taking that `Entry`: a grok read
  rewrite, not a row-sized change. Not started.
- `turns` T7, T9, T10 and `transport` X1-X6: not started in this step.
- Gap rows B, B-LLVM, V, ABI, U, U8, S, P7-1 and the new CW, BV: open.

## Step 2.11: the grok read path in the hand shape (rows P8, A8, W-c `scan_into`)

Hand counterpart: `grok/perf/hand_history.rs` — `Kind`, `Stamp`, `Entry` (`new`, `reset`, `at`, lines 194-260), the
line scanner `P` (`hex4` .. `entry`, lines 309-691), `Collector::feed` (776-805) and the `history_window` loop body
(904-944). The DB45 one-pass reader (old `grok/line.pruv`, scratch `f` / `d`) and the tape path behind it
(`read::line_step`: `parse::scan_into`, `access::view_root_type`, `lines::build_span` / `parse_range`, the
`JsonDocument` `entry::feed` with `feed_user` / `feed_assistant` / `entry_at` / `content` / `copy_text_range`) are gone.
`grok/line.pruv` is now one `Entry` scanner, one Pruv function per hand function; `entry::feed` takes that `Entry`.
`parse`, `access` and `lines` are untouched shared packages (still used by grok `path`, `rpc`, `summary`, `util` and
the perf root), so no other consumer changed.

### Line-by-line structure

| Hand Rust | Pruv | Types | Ownership / borrow | Loops | Alloc / copy | Difference and reason |
|---|---|---|---|---|---|---|
| `enum Kind { Missing, Null, Other, Str, Arr }` | `line::Kind`, same variants | — | value | — | — | = |
| `enum Stamp { Missing, Null, Other, Canon([u8; 24]) }` | `line::Stamp { Missing, Null, Other, Canon(at: i64), Num(value: i64) }` | offset for `[u8; 24]` | `Canon` borrows from the line | — | none (hand copies 24 bytes) | **P7-1**: no `[u8; 24]` value, so `Canon` keeps the offset of the 24 bytes in the line and `feed` borrows them. **NUM** (scope): `Num` is the integer timestamp of `time::iso_of` (a number `-?digits` or an unescaped digit string, trimmed, at most 2^53 - 1) that the fixtures test and the hand leaves out |
| `struct Entry { is_obj, ty, synthetic, prompt_index, content_kind, content: Vec<u8>, scratch: Vec<u8>, timestamp, created_at, tool_calls: Option<i64> }` | `line::Entry`, same fields | `[u8]` = `Vec<u8>`, `i64` for `u8` `ty` | owned by `read::window` | — | — | = |
| `Entry::new()` | `new_entry()` | — | result owns | — | two empty buffers | = |
| `Entry::reset(&mut self)`: all but `scratch`, `content.clear()` | `reset(inout e)`: same, `e.content = truncate(e.content, 0)` | — | `inout` | — | none, capacity kept | = |
| `Entry::at()`: `timestamp`, or `created_at` when `Missing \| Null`; `Canon` → `Some` | first lines of `entry::feed`: two `match`es pick the stamp | — | borrows the line | — | `Num` only: `time::format_iso` builds the 24 bytes | = for `Canon`; **ORD**: `at` is resolved before the user skips (hand: inside `begin`/`answer` calls, after them). Only a `Num` stamp can fail (`format_iso` returning 0 bytes, `-1`), and only then can a skipped user line report it; `format_iso`'s contract does not rule the defect out, no fixture reaches it |
| `hex4(s, at) -> Option<u32>` | `hex4(s, at) -> Result<i64, i64>` | **R**: `Result` for `Option` | borrowed | `for k in 0..4` | none | = (`?` works only on `Result`, LG11) |
| `P::ws(&mut self)` | `ws(s, inout i)` | cursor `inout i` = `self.i` | borrowed line | one `while` | none | = |
| `P::lit(&mut self, w: &[u8])` | `lit(s, inout i, w: [u8])`: `eq_range(s, i, w, 0, len(w))` | **R** | literal is a static `[u8]` view | — | none | = |
| `P::skip_string() -> Option<(usize, usize)>` | `skip_string(s, inout i) -> Result<Raw, i64>` | **T**: `struct Raw { lo, hi }` for the tuple | borrowed | one `while`, escapes `\"\\/bfnrt` and `\u`+`hex4` | none | = (no tuples in a module) |
| `P::decode_string(&mut self, out: &mut Vec<u8>)`: runs to `"`, `\` or `< 0x20` with `extend_from_slice`; simple escapes push one byte; `\u` + surrogate pair; `char::from_u32(..).unwrap_or('\u{FFFD}')` + `encode_utf8` | `decode_string(s, inout i, inout out: [u8])`: same runs with `arr_append` (LG30: the run lies between ASCII bytes of a valid line), simple escapes `arr_push`, same pair combination; `push_char` encodes the scalar | **R** | `inout out` = `&mut Vec<u8>` | outer `while` + run `while` | appends into `out`'s capacity | **CH**: no `char::from_u32` / `encode_utf8` on `i64`, so `push_char` writes the 1-4 UTF-8 bytes arithmetically (surrogates → U+FFFD); same bytes, proved UTF-8 |
| `P::number()` | `number(s, inout i)` | **R** | borrowed | three digit `while`s | none | = |
| `P::skip_value(depth) -> Option<Kind>`: `depth > 128` → `None`; `{` → `members(skip_value(depth + 1))` | `skip_value(s, inout i, depth) -> Result<Kind, i64>`; the object arm writes `members` out | **R** | borrowed | members `while` inline | none | **CL**: no closures, so `members(f)` is written out in each caller (`skip_value`, `part_members`, `fields`). Mutual recursion with `skip_array` carries `decreases 2 * (129 - depth) + 1` / `2 * (129 - depth)` |
| `P::skip_array(depth) -> Option<i64>` | `skip_array(s, inout i, depth) -> Result<i64, i64>` | **R** | borrowed | one `while` | none | = |
| `P::members(f)` | `part_members`, `fields`, `skip_value`'s object arm | — | — | one `while` each | none | **CL** (above) |
| `P::stamp() -> Option<Stamp>`: a string of 24 bytes that is `canonical_iso` → `Canon(copy)`, other strings `Other`; else `skip_value(1)`: `Null` → `Null`, else `Other` | `stamp(s, inout i)`: same, with `Canon(r.lo)`; unescaped other strings → `digit_stamp`, numbers → `number_stamp` (`Num` or `Other`) | **R** | borrowed | digit loops in the helpers | none | **P7-1**, **NUM** (above). An escaped string stays `Other` |
| `P::part(&mut self, e)`: `let mut scratch = take(&mut e.scratch)`; `"text"` + string → `scratch.clear()`, decode, `has = true`; other `"text"` → `has = false`; then `"\n\n"` + scratch into `content`; restore | `part(s, inout i, inout e)`: `e.scratch = truncate(..)` once, `part_members(s, inout i, inout e.scratch, inout has)`, then the same join with `arr_append` | **R** | disjoint field borrows (`inout e.scratch` while `e` is held) | members `while` | appends into `content`'s capacity | **SC**: the scratch is emptied once at the start of `part` instead of only before a `"text"` decode (the earlier bytes are never read, so the text is the same); no `take` needed |
| `P::content(&mut self, e)` | `content(s, inout i, inout e)` | **R** | `inout e` | one `while` over the array | decodes into `e.content` | = |
| `P::entry(&mut self, e)`: members over the line with `type`, `synthetic_reason`, `prompt_index`, `content`, `timestamp`, `created_at`, `tool_calls` | `entry(s, inout i, inout e)` + `fields` | **R** | `inout e` | `fields` `while` | none | **CL**; keys compared with `raw_is` (**KQ**: no slice `==` on a range of `s`; `eq_range` compares the same bytes) |
| `Collector::feed(&mut self, e: &Entry)` | `entry::feed(line, e, inout st, inout sl, inout bk)` → `user_view` / `assistant_view` | — | `e` borrowed (`&Entry` in Rust) | — | none for `Canon` | = decisions (`synthetic` not `Missing \| Null` skips, `query_range`, `<user_info>` with no `prompt_index`, `is_blank`, `answer` + `tool(n)`); **G**: a run-time guard re-checks that `Canon`'s offset lies between ASCII bytes of the line (what `stamp` found), so the 24 bytes pass as a `str` |
| `history_window` loop body: `from_utf8(line)` ok → `entry.reset()`, `P { s: line, i: 0 }.entry(&mut entry)`, `Some` → `col.feed(&entry)` | `read::line_step(line, inout e, ..)`: `utf8_valid(line)`, `line::reset`, `line::entry(line, inout i, inout e)`, `Ok` → `entry::feed` | — | the line is a borrowed `subview` of the window | — | none per line | = (`event_step` passes `subview(win, lo, hi)`; the 16 MiB guard stays) |
| `let mut entry = Entry::new()` once per window | `window`: `let mut entry = line::new_entry();` | — | owned, `inout` to each line | — | one `Entry` | = (the `tape` / `stack` / `dbuf` scratch and its growth are gone) |

The Q mutant markers that lived in the removed tape code moved to the same decisions in the new code (`q_mutants.py
--check` applies them): M08 at the `<user_query>` `begin2` in `user_view`, M09 at the `synthetic_reason` `Other` arm of
`feed`, M11 at `add_tools` in `assistant_view`.

### Gap rows found in this step

| Id | Hand Rust form | What Pruv cannot express | Where it shows |
|---|---|---|---|
| R | `Option<T>` with `?` | `?` only on `Result` (LG11) | every scanner function returns `Result<_, i64>` |
| CL | `members<F: FnMut(..)>(&mut self, f: F)` | no closures | `members` written out three times in `line.pruv` |
| T | `Option<(usize, usize)>` | no tuples in a module | `skip_string` returns `struct Raw` |
| CH | `char::from_u32(v).unwrap_or('\u{FFFD}').encode_utf8(..)` on a `u32` | no `char` from a run-time `i64` with fallback | `line::push_char` |
| KQ | `key == b"text"` on a subslice | `subview` only as a call argument; no slice `==` | `line::raw_is` (`eq_range`) |
| P7-1 | `Stamp::Canon([u8; 24])` | no fixed-size array value (existing row) | `Canon(at)` offset + guard **G** in `entry::feed` |

Scope rows (not language gaps): **NUM** (integer timestamps, beyond the hand), **ORD** (stamp resolved before the
user skips), **SC** (scratch emptied once per part).

### Consumer tests (step 2.11)

Only grok files changed (`line.pruv`, `entry.pruv`, `read.pruv`); no shared package changed, so only the grok runner
was run. Pairs are rs/rust and rs/llvm (pruvc-rs).

| Runner | Fixtures per pair (rs/rust, rs/llvm) |
| --- | --- |
| grok `test_impl --no-ts` (`GROK_COMPILE_WALL=3600`) | 254 / 254, 254 / 254 (run again after the `event_step` cleanup: 254 / 254, 254 / 254) |

Nothing was measured.

### Independent structure comparison

Before the commit a separate read of the same files against the same hand sources (Codex gpt-6.2-astra, high, static
reading plus the generated Rust) found no line that one side accepts and the other rejects, and the same user and
assistant text and tool counts fed to the collector. Points it raised and where they stand:

- Integer timestamps: `{"type":"user","content":"x","timestamp":0}` starts prompt `x` on both sides, with no time in the
  hand and `1970-01-01T00:00:00.000Z` in Pruv. That is **NUM** (the TypeScript reference and the fixtures want it).
- `entry::feed` on a hand-made `Entry` whose `Canon` offset is wrong returns `-1`; `window` always passes the line the
  `Entry` was scanned from, so guard **G** never fires there.
- `part` empties the scratch on entry as well as before each `"text"` decode: **SC**, same joined text.
- `assistant_view` adds tools with `n` calls of `turns::tool` (`add_tools`), the hand with one `tool(n)`; same saturated
  count. That is the open `turns` row (one `tool(n)` needs the T-rows rewrite).
- `query_range` returns a 2-slot array where the hand returns `Option<(usize, usize)>` (**T**, unchanged code).
- The dropped-line arm of `event_step` copies the first 8192 bytes (`util::span_bytes`) where the hand borrows them
  (unchanged code, listed below).
- `event_step` still mapped a `line_step` result of 1 to "grow and retry" (2), which no path returns any more
  (`line_step` and `feed` return 0 or -1): removed after the comparison, with its comment.

### Rows still open after step 2.11

- `turns` T7, T9, T10 and `transport` X1-X6: not started.
- Gap rows B, B-LLVM, V, ABI, U, U8, S, P7-1, CW, BV and the new R, CL, T, CH, KQ: open.
- grok `assistant_view`: `n` calls of `turns::tool` where the hand makes one `tool(n)` (waits on the T-rows rewrite).
- grok `event_step`: the dropped-line arm copies 8192 bytes with `util::span_bytes` (the hand borrows).

## Step 2.12: consumer API changes for `turns` and `transport`

Hand counterparts: `grok/perf/hand_history.rs` (`Collector`, `Slot`, `tool(&mut self, n)`, `dropped_user`),
`codex/hand/reader.rs` (`Rpc { output: BufReader, line: String }`, `turn.tools += 1` per tool item; Pruv passes the item sum once), `claude/handrust` `tool`.
`claude` keeps its own frozen `claude/turns.pruv` (its manifests reach only `json` and `jstr`), so the `pkgs/turns`
consumers are qwen, grok, codex and opencode.

### Changed

| Row | Before | After | Hand |
|---|---|---|---|
| T4 `tool` | `tool(inout st, inout sl)`: one call per tool call; codex, opencode and grok each had a private `add_tools(n, ..)` loop | `tool(inout st, inout sl, n)`: `tools = min(tools + n, 1e15)` in one step; the three `add_tools` loops are gone, qwen passes 1 | `tool(&mut self, n)`: `(s.tools + n).min(TOOLS_MAX)` |
| grok `assistant_view` | `add_tools(tools, ..)` = `tools` calls of `turns::tool` | one `turns::tool(.., tools)` | one `tool(n)` |
| grok `read::event_step` (dropped line) | copied the head with `util::span_bytes(win, lo, hi)` (up to 8192 bytes) | `framing::dropped_user(subview(win, lo, hi))`: borrowed | `dropped_user(&win[p..p + DROP_HEAD])` |
| X6 | `st: [i64]` 5-slot packet | `transport::Reader { status, filled, at, exit, end }` from `reader_new()`; mcp, codex and grok use the field names | reader state as named locals/fields |

grok `q_mutants.py` M11 keeps only the marker variant (the `add_tools` inject target is gone; the marker now makes
`tool(.., 1)`). Docs that named `add_tools` (codex `IMPL_NOTES.md`, grok `Q_README.md`, grok `DESIGN.md`) are updated;
the step 2.4 and 2.11 mentions above are history and stay.

### Not changed (gap rows)

- **T7, T9, T10** stay. The hand shape is `Collector { slots: Vec<Slot> }` with an owned `Vec<u8>` (`String`) per
  prompt and reply, filled in place and then serialised by the consumer. Probes on pruvc-rs (main, this step):
  a struct with a `Vec<Slot>` field is rejected on LLVM ("has owned collection fields, which need the Rust backend"),
  an `inout Vec<Slot>` parameter is rejected ("G6 foreign calls and collections are not implemented by the llvm
  codegen layer yet"), and `vec_at_mut` on a local `Vec<Slot>` with a `[u8]` field is rejected ("vec_at_mut needs the
  Rust backend"). All four consumers run on LLVM, so the rewrite would leave the LLVM pair. New gap id **VL** below;
  T7/T9/T10 keep **W-c** with **ABI / V** and **VL**.
- **X1, X2** stay **W-c** with no hand counterpart: the hand blocks on `read_line`/`write_all` (codex) or uses a
  reader thread with `recv_timeout` (supervisor), Pruv keeps one event loop with host effects. So `send` keeps its
  deadline `ms`, `poll` and the 8-poll `expired` stay, and `Reader.status`/`at..end` stay as fields because the loop
  returns between reads. The API is already send / close / poll; nothing Node-style is left in it after X6.
- **X3** (`grow_line` used by grok for the error-reply buffer `aux`) is unchanged; still the **W-c** error-reply
  storage row.
- **X4, X5** were already `=`.

| Id | Hand Rust form | What Pruv cannot express | Functions left unchanged |
|---|---|---|---|
| VL | `Collector { slots: Vec<Slot> }`, `Slot { prompt: String, reply: String, .. }` filled in place | `Vec<S>` of a non-recursive struct with owned `[u8]` fields on LLVM (struct field, `inout` parameter and `vec_at_mut`; R-6e-21 covers only recursive groups) | `turns` T7, T9, T10 (and the slot packet of T0) |

### Consumer tests (step 2.12)

pruvc-rs, pairs rs/rust and rs/llvm. claude is not a `pkgs/turns` consumer and was not run.

| Runner | rs/rust | rs/llvm |
|---|---|---|
| qwen `test_impl.py` | 26 / 26 | 26 / 26 |
| opencode `test_impl.py` | 199 / 199 | 199 / 199 |
| codex `test_impl.py` | 136 / 136 | 136 / 136 |
| mcp `test_impl.py` | 22 / 22 | 22 / 22 |
| grok `test_impl --no-ts` (`GROK_COMPILE_WALL=3600`) | 254 / 254 | 254 / 254 |

Nothing was measured.

### Independent structure comparison

Before the commit a separate static read (Codex gpt-6.2-astra, high) against `HEAD` and the hand sources found no input
whose Pruv output changes: the tool counts of all four callers, the borrowed `[lo, hi)` range of `event_step` (the same
range `span_bytes` copied and the hand slices) and every `Reader` field (0 status, 1 filled, 2 at, 3 exit, 4 end) are
kept. `reader_new` starts `status` at -6 instead of 0; every reader writes -6 before the first read. Points it raised:

- `turns::tool` with `n` near `i64::MAX` saturates to 1e15 where the hand `(t + n).min(..)` would overflow; no caller
  reaches it (grok counts array elements of a line under 1 MiB, codex checks `0..=1e8`).
- grok `assistant_view` skips the call for 0 tools where the hand calls `tool(0)`; same value.
- Existing differences (unchanged here): the codex RPC framing vs hand `Rpc::call` (EOF without LF, a bad line skipped,
  the 8 MiB cap), the event pump and the 8-poll `expired` (X1, X2), and the codex `parse_line` trim copy.

### Rows still open after step 2.12

- `turns` T7, T9, T10 (**VL**, **ABI / V**).
- `transport` X1, X2, X3 (**W-c**, no hand counterpart for the event loop).
- Gap rows B, B-LLVM, V, ABI, U, U8, S, P7-1, CW, BV, R, CL, T, CH, KQ and the new VL: open.

## Step 4.4l: UTF-8 fact for string spans (4.4i-UF), one-pass `decode_append` (4.4i-DP)

- `lex::validate_utf8` ensures `result == -1 ==> utf8_valid_span(input, start, end)`; `lex::scan_string` ensures the
  content between the quotes is valid UTF-8 (`utf8_valid_span(input, old(pos) + 1, pos - 1)`). Contracts and loop
  invariants only; the code is unchanged.
- `parse::strings_ok(input, tape, lo, hi)` (new pred): every string (kind 4) and key (kind 7) record in `lo..hi` has
  valid UTF-8 content. `record`, `container`, `scan_records` and `scan` carry it; `scan_document` and `parse` ensure
  `json::doc_strings(document)` for `Parsed`.
- `json::doc_strings(d)` (new pred) states the same fact over `d.source` / `d.tape`. `lines::parse_owned`,
  `parse_copy`, `parse_range` and `build_span` ensure it for `Parsed` (`build_span` also requires it of its input
  tape). A consumer that requires `doc_strings(doc)` can view a plain span as `str` with no run-time scan (R-6g-10).
- First tried as a `JsonDocument` invariant: the three construction sites verify, but every call that passes the
  document inside a loop re-proves the quantified fact, and `access::find_ascii` (`valid(doc, v)`, access.pruv:104)
  times out (`struct_invariant`, counterexample `canceled`) in claude a1/a2, json a4, qwen and grok. As a pred
  the fact is carried only where it is required.
- Negative check (temp copy): widening the span by one byte (`tape[6 * t + 1]` in place of `+ 1`) is rejected in
  `parse::scan_document` (line 371) and `lines::parse_owned` (line 54).
- `jstr::decode_append` is one pass: the count loop, `width` and `arr_reserve` are gone; plain runs are appended with
  `extend_from_slice` and each escape is written as UTF-8, like the hand's `decoded.get_or_insert_with(Vec::new)`
  (closes 4.4i-DP).
- The consumer moves (4.3h-O, the title of 4.4c-MB) are not done in this step (cap); they are now unblocked.

### Per-function structure (generated Rust vs `claude/handrust/src/lib.rs`)

Independent static comparison by Codex `gpt-6.2-astra` (review role), given the generated Rust of the probe root and
the hand Rust only. Every function is still structurally different; each difference has a row.

| Function | Pruv (generated Rust) | Hand | Same? | Row |
|---|---|---|---|---|
| `jstr::decode_append` | one pass, no reserve, decodes into the caller's buffer | decodes while parsing, `Cow::Borrowed` for a plain chunk | no: a plain token is copied into `out`; with `scanned = false` non-ASCII is decoded and re-encoded | 4.3h-O (borrow at the consumer); new 4.4l-RE |
| `jstr::decode_utf8` | always returns owned bytes | borrows a plain token | no | 4.3h-O |
| `lex::validate_utf8` | 16-byte ASCII block, then scalar walk with a block revisit | `str::from_utf8` once per line | no | new 4.4l-VU |
| `lex::scan_string` | revalidates each raw non-ASCII scalar after the line check | no per-scalar check after `from_utf8` | no | new 4.4l-SS |
| `parse::record` | flat 6-word tape, record limit 1048577, duplicate keys kept as separate records | `Value` tree, `BTreeMap::insert` (last duplicate wins), no record limit, depth 128 | no | existing tape-vs-tree rows (unchanged) |
| `parse::scan_records` | `reserve_exact(192)` up front, accepts a non-object root | `parse_line` requires an object root (`object(1)`), no up-front reserve | no | new 4.4l-RS |
| `parse::scan_document` | `to_vec` copy of the source into `JsonDocument` | borrows the line | no | existing owned-source row (U) |
| `lines::parse_owned` | `check_tape` makes 2 extra passes over the tape, possible `shrink_to_fit` | no tape re-check | no | new 4.4l-CT |
| `lines::parse_range` / `parse_copy` | extra `to_vec` copy of the range | parses the borrowed line | no | existing owned-source row (U) |

### New rows (4.4l)

| Id | Kind | Difference | Where |
|---|---|---|---|
| 4.4l-RE | shape | `decode_append` with `scanned = false` decodes and re-encodes raw non-ASCII; the hand copies the validated bytes | `pkgs/jstr` |
| 4.4l-VU | shape | `validate_utf8` revisits a 16-byte block on the first non-ASCII byte; the hand calls `from_utf8` | `pkgs/lex` |
| 4.4l-SS | shape | `scan_string` checks each raw non-ASCII scalar again after the whole-line check; could follow from `validate_utf8`'s ensures where the caller validated the line first; not tried | `pkgs/lex` |
| 4.4l-RS | shape | `scan_records` reserves 192 words up front and accepts a non-object root; the hand `parse_line` takes only an object | `pkgs/parse` |
| 4.4l-CT | shape | `parse_owned` re-walks the tape twice in `check_tape` | `pkgs/lines` |

### Consumer tests (step 4.4l)

pruvc-rs, pairs rs/rust and rs/llvm, with `doc_strings` as a pred (the invariant run is not counted).

| Runner | rs/rust | rs/llvm |
|---|---|---|
| json `run_a0` | 37 / 37 | 37 / 37 |
| json `run_a1` | 351 / 351 | 351 / 351 |
| json `run_a2` | (llvm only) | 114 / 114 |
| json `run_a3` | 454 / 454 | 454 / 454 |
| json `run_a4` | 6344 / 6344 | 6344 / 6344 |
| json `run_a5` | 83 / 83 | 83 / 83 |
| supervisor `test_a0` | 522 / 522 | 522 / 522 |
| supervisor `test_a1` | 410 / 410 | 410 / 410 |
| supervisor `test_a2` | 78 / 78 | 78 / 78 |
| supervisor `test_a3` | 35 / 35 | 35 / 35 |
| grok `test_impl --no-ts` (`GROK_COMPILE_WALL=3600`) | 254 / 254 | 254 / 254 |
| claude `test_a0` | 272 / 272 | 272 / 272 |
| claude `test_a1` | 2884 / 2884 | 2884 / 2884 |
| claude `test_a2` | 2443 / 2443 | 2443 / 2443 |
| claude `test_a3` | 134 / 134 | 134 / 134 |
| claude `test_a4` | 270 / 270 | 270 / 270 |
| claude `q.py` | 20270 / 20270 | 20270 / 20270 |
| claude `q_mutants.py` | 18 / 18 caught (9 detected, 9 proof-rejected) | (same run) |
| qwen `test_impl.py` | 26 / 26 | 26 / 26 |
| mcp `test_impl.py` | 22 / 22 | 22 / 22 |
| codex `test_impl.py` | 136 / 136 | 136 / 136 |
| opencode `test_impl.py` | 199 / 199 | 199 / 199 |

`claude/q.py` exits 1 only because it also lists the frozen py pairs (`build: 'py'`, not part of the check); both rs
pairs pass. opencode ran with `GROK_TS_TRANSPILER` set (the first run without it stopped before building). Nothing
was measured.

## Step 4.4m: borrowed plain strings for Claude replies (4.3h-O, part), 4.4l-SS corrected

- `access::plain_str(doc, id) -> Option<ref str> from(doc.source)` (new), `requires json::doc_strings(doc)`: a plain
  string node (kind 4, tape slot 5 == 1, no backslash) as a borrowed `str` view of its content (`subview` of
  `doc.source`, R-6g-10 / R-6g9-7 / R-6g9-8). No copy, no allocation, no run-time UTF-8 scan: the generated Rust is
  `Option<&'a str>` built with `from_utf8_unchecked`. `None` for an escaped node, another kind or a corrupt tape.
- Claude: `entry::parse_line` ensures `doc_strings` of the `Line::Object` document. `import::answer_blocks` (text
  blocks) and `import::consume_entry` (string content) require it and pass the borrowed view to `turns::answer`, as
  the hand `col.answer(s, ..)` does, when `plain_str` is `Some`; otherwise the owned decode (`string_text_owned`).
- **Not effective yet (found by the structure comparison, confirmed):** the document `parse_line` gets comes from
  `parse::parse` → `scan_document` → `scan_records` → `record`, and `record` writes slot 5 = `0` for every string
  (parse.pruv:263, `append_record(.., 0)`). Only `scan_into` (grok, parse.pruv:536-550) sets the plain bit. So on
  the Claude path `plain_str` is always `None` and every reply still takes the owned decode; the consumer shape is
  in place, the producer is not. Row 4.4m-PB.
- 4.4l-SS corrected: the hand is not free of a second check either. `value()` (handrust/src/lib.rs:507-508) runs
  `str::from_utf8` again on every string value after `parse_line`'s line check (keys are not rechecked). Pruv's
  `scan_string` checks each raw non-ASCII scalar of values and keys. The difference is the granularity and keys, not
  an extra pass; dropping the Pruv check needs a "line validated" flag through `record` / `container` /
  `scan_records` / `scan` and is not done (row 4.4m-SS replaces 4.4l-SS).

### Per-function structure (generated claude a4 Rust vs `claude/handrust/src/lib.rs`)

Independent static comparison by Codex `gpt-6.2-astra` (review role, Codex 2 account; the default account was at its
limit), given `claude/work/a4/rs_rust/a4_root_gen.rs` and the hand Rust only. Its main finding (the plain bit is never
set on this path) was confirmed in `parse.pruv`. No function on the reply or title path is the hand's shape yet.

| Function | Pruv (generated Rust) | Hand | Same? | Row |
|---|---|---|---|---|
| `access::plain_str` | `Option<&'a str>` from `doc.source`, `from_utf8_unchecked`, no allocation, no scan | `get_str` -> `Option<&'a str>` | type same; never `Some` on the Claude path (slot 5 is always 0) | new 4.4m-PB |
| `import::answer_blocks` (text block) | tries `plain_str`, then `string_field_of` -> owned `String` decode, then `answer(&str)` | `col.answer(t, at)` on the parsed `Cow` (no decode at the use) | no: an owned decode per plain text block | 4.4m-PB |
| `import::consume_entry` (string content) | tries `plain_str`, then `string_text_owned`, decode before the collector's early return | `col.answer(s, ..)` | no | 4.4m-PB, 4.3h-T |
| `entry::title_key` / `pick_title` / `ranked` | owned decode of each candidate before the blank check; the winner `String` moves into `Title::Ranked` | `pick` borrows and blank-checks; copies only the winner | no | new 4.4m-TB |
| `import::tail_info` / `read_window` (title) | `best = move cand`; the old buffer is dropped, not reused | one `String`, `clear` + `push_str` | no | 4.4m-TB |
| `entry::string_content` / `block_of` / `array_content` | owned decode of every piece (`Field::Value(String)`, `Block::Piece(String)`), even when the filter then rejects it | `Cow::Borrowed`; one `joined` only for 2+ pieces | no | new 4.4m-FB |
| `lex::scan_string` | per raw non-ASCII scalar check of values and keys, after the line check on `parse_owned` | `str::from_utf8` per value chunk after the line check (lib.rs:507-508) | no: granularity and keys | 4.4m-SS (replaces 4.4l-SS) |

### New rows (4.4m)

| Id | Kind | Difference | Exact obligation to close | Where |
|---|---|---|---|---|
| 4.4m-PB | producer | `record` writes plain bit 0 for every string, so `plain_str` is `None` on the `parse` path | `record` takes the plain bit from `string_token` instead of `lex::scan_string`; `local_records` (parse.pruv:193), the `scan_document` / `parse` ensures (382, 434) and `publication_ok` (394-417) allow `tape[6t+5] == 1` for kinds 4/7 (the form `check_tape` already accepts, 402); rerun every consumer of `pkgs/parse`. Not a SPEC gap; not done (cap, the reruns were in flight) | `pkgs/parse` |
| 4.4m-TB | shape | title: decode before the blank check, and a new `String` per winner instead of one reused buffer | after 4.4m-PB: `title_key` blank-checks `plain_str`'s view and copies only a nonblank one. Buffer reuse needs a `str` buffer that `clear` + append of a `str` keeps valid; `string_append` / `decode_append` on `[u8]` have no UTF-8 ensures, so a `[u8]` buffer cannot be lent to `text::one_line(str)` | `claude/entry`, `claude/import`, `pkgs/jstr` |
| 4.4m-FB | language | user content pieces are owned (`Field::Value(text: str)`, `Block::Piece(text: str)`) | an enum payload of `ref str` is `type/borrow_signature` (R-6g-11); keeping the functions, the borrowed form would need a view payload in a user enum | `claude/entry` |
| 4.4m-SS | shape | 4.4l-SS restated: the hand rechecks each value chunk too; Pruv checks per scalar and also keys | a "line validated" flag through `record` / `container` / `scan_records` / `scan` so `scan_string` skips the scalar check under `parse_owned` | `pkgs/lex`, `pkgs/parse` |

### Consumer tests (step 4.4m)

pruvc-rs (`PRUVC_RS=pruvc-rs/target/release/pruvc`), pairs rs/rust and rs/llvm, every consumer of `pkgs/access`.

| Runner | rs/rust | rs/llvm |
|---|---|---|
| claude `test_a0` | 272 / 272 | 272 / 272 |
| claude `test_a1` | 2884 / 2884 | 2884 / 2884 |
| claude `test_a2` | 2443 / 2443 | 2443 / 2443 |
| claude `test_a3` | 134 / 134 | 134 / 134 |
| claude `test_a4` | 270 / 270 | 270 / 270 |
| claude `q.py` | 20270 / 20270 | 20270 / 20270 |
| claude `q_mutants.py` | 18 mutants: 8 detected, 10 proof-rejected, none survive | (same run) |
| json `run_a0`-`run_a5` | 37 / 351 / 114 / 454 / 6344 / 83 | same |
| supervisor `test_a0`-`test_a3` | 522 / 410 / 78 / 35 | same |
| grok `test_impl --no-ts` (`GROK_COMPILE_WALL=3600`) | 254 / 254 | 254 / 254 |
| qwen `test_impl.py` | 26 / 26 | 26 / 26 |
| mcp `test_impl.py` | 22 / 22 | 22 / 22 |
| codex `test_impl.py` | 136 / 136 | 136 / 136 |
| opencode `test_impl.py` (`GROK_TS_TRANSPILER` set) | 199 / 199 | 199 / 199 |

`claude/q.py` exits 1 only for the frozen py pairs (`build: 'py'`); both rs pairs pass. These runs exercise only
the `None` branch of `plain_str` on the Claude path (4.4m-PB); the `Some` branch is verified but not run. Nothing was
measured.

## Step 4.4n: the parse path sets the plain bit (4.4m-PB), title blank check on the borrow (4.4m-TB part one)

- `parse::record` now takes the plain bit from `string_token` (one pass per string that validates in place and
  reports "no escape"). `scan_into` already did this. It no longer calls `lex::scan_string`. Slot 5 of every string
  and key record is 1 when the token has no backslash.
- `string_token` ensures `utf8_valid_span(input, start + 1, pos - 1)` on `Ok`, through a loop invariant on the
  outer and hex loops. That is what `check_tape` / `doc_strings` need for a plain node.
- The slot-5 facts in `parse` were widened to `slot5 == 0 || (slot5 == 1 && kind in {4, 7})`, the form
  `check_tape` already accepts. They are `local_records` (parse.pruv:195), the `scan_document` ensures (383, 386),
  `publication_ok` (400, 406, 414) and the `parse` ensures (438). 386 is now implied by 383 and is kept.
- `entry::title_key` first tries `access::plain_str`. On `Some(v)` it blank-checks the borrowed view and copies
  only a nonblank `v`. `None` (escaped, non-string or missing) keeps the old `string_field_of` decode.
  `pick_title`, `ranked`, `title_of` and `title_for_type` require `json::doc_strings(doc)`. Both callers in
  import.pruv get `d` from `parse_line`, which ensures it.
- `import::read_window`: with the new `doc_strings` requires on `title_for_type`, the solver gave up on
  `consume_entry`'s `doc_strings` precondition inside the window loop (`verify_unknown`, push canceled). That
  happened even with an `assert doc_strings(d)` right after the `Line::Object(d)` match. The loop body for one
  parsed line moved into `window_object` (title candidate, `consume_entry`) and `window_model` (model copy). It
  returns the `Title` for the loop to keep, since an `inout str` parameter is `type/inout_type`. The generated Rust
  is `#[inline(always)]` with `&doc` borrowed and the `Title` moved, so the run-time structure is the same as
  before. `tail_info` verified unchanged.
- `json/run_a2.py`: its independent oracle now expects the plain bit, so slot 5 is 1 for a string or key with no
  backslash. The other runners did not need a change.
- New test: `claude/test_a1.py` `plain` cases run `parse_line` -> `field` -> `plain_str` through the wrapper
  `a1_root.pruv::plain_w`. There are 13 hand-written cases: plain, empty, non-ASCII, escaped values, an escaped key
  with a plain value, an escape elsewhere on the line, non-string, missing, and a duplicate key. They show `Some`
  (with the byte length) on both backends for every unescaped string and `None` for escaped ones.

| Function | Pruv (generated Rust) | Hand (handrust/src/lib.rs) | Same? | Row |
|---|---|---|---|---|
| `parse::string_token` / `record` | one pass per string; non-ASCII scalars checked with `validate_utf8` in place; returns the plain bit; `record` writes it to slot 5 | `Parser::string` one pass; `Cow::Borrowed` without escape; `str::from_utf8` per value chunk | pass count and plain bit: same. Granularity of the UTF-8 check (per scalar, and keys too) differs | closes 4.4m-PB; 4.4m-SS open |
| `access::plain_str` on the Claude path | `Some(&str)` from `doc.source` for unescaped strings (it was always `None` before) | `get_str` -> `Some(&str)` (`Cow::Borrowed`) | same | 4.4m-PB |
| `import::answer_blocks` / `consume_entry` (plain text) | `plain_str` view passed to `turns::answer`; no decode | `col.answer(&str)` on the parsed `Cow` | same for unescaped text; escaped text is decoded at the use (hand decodes during parse) | 4.4m-PB, 4.3h-T |
| `entry::title_key` / `pick_title` | blank check on the borrowed view, then `to_owned` only for a nonblank candidate; escaped candidates are still decoded before the check | `pick`: borrow, `es_trim` check, return the borrow | check order and copy count: same for unescaped; escaped decode before the check remains | closes 4.4m-TB part one |
| `entry::ranked` / `title_of`, `import::tail_info` / `read_window` | the winning `String` moves into `Title::Ranked`, then `best = move cand` | `RankedTitle::update`: `text.clear(); push_str(t)` into one buffer | no: a new `String` per winning title line | 4.4m-TB part two (open) |

Independent structure comparison (Codex `gpt-6.2-astra`, review role, Codex 2 account). It got the generated
`a1_root_gen.rs` and the hand `lib.rs`, without this table. Its conclusions agree with the table:
- one forward pass per string token on both sides;
- unescaped title candidates are blank-checked on a borrowed `&str` on both sides;
- one copy per winning title on both sides (Pruv `to_owned` in `title_key`, hand `push_str` in `update`).

The remaining difference is where the copy lands: a fresh `String` per win vs one reused buffer. That is
4.4m-TB part two. The review found two more per-call differences. They are new rows:
- `entry::title_for_type` calls `access::string_is` up to three times (`custom-title`, `ai-title`, `summary`). The
  hand borrows `type` once with `get_str` and `match`es on it. Row 4.4n-TT.
- An ASCII key lookup (`access::field`) walks every direct member of the object, so last-wins resolves at the
  lookup, and `pick_title` walks it again for the second key. The hand resolves last-wins once at parse
  (`BTreeMap::insert`) and each `get` is a map lookup. Row 4.4n-KL.

The review also noted that `text::is_blank` stops at the first nonblank character, while the hand `es_trim`
trims both ends. The Pruv side does no more work than the hand here, so this gets no row. An escaped candidate is
decoded at the lookup in Pruv; the hand decodes every escaped value during the parse. That is the 4.3h-T
placement and gets no new row.

| Row | Kind | Difference | What closes it | Where |
|---|---|---|---|---|
| 4.4m-PB | closed | `record` wrote the plain bit 0 for every string | done in 4.4n: `record` takes it from `string_token`; `test_a1` `plain` cases show `Some` on both backends | `pkgs/parse` |
| 4.4m-TB (part one) | closed | decode before the blank check for unescaped candidates | done in 4.4n: `title_key` checks `plain_str`'s view | `claude/entry` |
| 4.4m-TB (part two) | shape, open | a new `String` per winning title, not one reused buffer | as stated in 4.4m: a `str` buffer that `clear` + append of a `str` keeps valid UTF-8 (the `[u8]` appends have no UTF-8 ensures) | `claude/import`, `pkgs/jstr` |
| 4.4m-SS | shape, open | per-scalar UTF-8 checks in `string_token` (values and keys) after the line check, vs the hand's per-value `from_utf8` | unchanged by 4.4n (`record` now calls `string_token`, not `lex::scan_string`): a "line validated" fact through `record` / `container` / `scan_records` / `scan` so `string_token` skips `validate_utf8` under `parse_owned` | `pkgs/parse`, `pkgs/lex` |
| 4.4n-TT | shape, new | `title_for_type`: up to three `string_is` scans of `type` vs one borrow + `match` | read `type` once through `plain_str` and compare the view (escaped `type` falls back to `string_is`) | `claude/entry` |
| 4.4n-KL | shape, new | key lookup walks all members per lookup (last-wins at the lookup) vs a map built once at parse | an index or a "keys unique" fact from the parse; needs a design step | `pkgs/access`, `pkgs/parse` |

### Consumer tests (step 4.4n)

pruvc-rs (`PRUVC_RS=pruvc-rs/target/release/pruvc`), pairs rs/rust and rs/llvm. Every consumer of `pkgs/parse`
was rerun, which covers `claude/entry` and `claude/import` too.

| Runner | rs/rust | rs/llvm |
|---|---|---|
| claude `test_a0` | 272 / 272 | 272 / 272 |
| claude `test_a1` (+13 `plain` cases) | 2897 / 2897 | 2897 / 2897 |
| claude `test_a2` (after the `window_object` split) | 2443 / 2443 | 2443 / 2443 |
| claude `test_a3` | 134 / 134 | 134 / 134 |
| claude `test_a4` (after the split) | 270 / 270 | 270 / 270 |
| claude `q.py` (after the split) | 20270 / 20270 | 20270 / 20270 |
| claude `q_mutants.py` (after the split) | 18 mutants: 8 detected, 10 proof-rejected (mutant 3 by `verify_unknown`, not a definite rejection), none survive | (same run) |
| json `run_a0`-`run_a5` (`run_a2` oracle updated) | 37 / 351 / 114 / 454 / 6344 / 83 | same |
| supervisor `test_a0`-`test_a3` | 522 / 410 / 78 / 35 | same |
| grok `test_impl --no-ts` (`GROK_COMPILE_WALL=3600`) | 254 / 254 | 254 / 254 |
| qwen `test_impl.py` | 26 / 26 | 26 / 26 |
| mcp `test_impl.py` | 22 / 22 | 22 / 22 |
| codex `test_impl.py` | 136 / 136 | 136 / 136 |
| opencode `test_impl.py` (`GROK_TS_TRANSPILER` set) | 199 / 199 | 199 / 199 |

The first claude `test_a2` / `test_a4` / `q.py` / `q_mutants.py` runs failed verification at `read_window`
(`verify_unknown`). The split above fixed that. `claude/q.py` exits 1 only for the frozen py pairs
(`build: 'py'`); both rs pairs pass. The `Some` branch of `plain_str` runs on `parse_line` documents on both
backends (`test_a1` `plain` cases); no separate trace of `answer_blocks` was made. Nothing was measured.

## Step 4.4p: one line check on the record path (4.4m-SS), `type` read once in the title path (4.4n-TT)

- `parse::string_token` takes `checked: bool` and requires `checked ==> utf8_valid_span(input, 0, len(input))`.
  With `checked`, a byte `>= 128` is skipped like any other non-special byte (hand
  `position(|b| matches!(b, b'"' | b'\\' | 0..=31))`). The `Ok` ensures `utf8_valid_span(input, start + 1, pos - 1)`
  is unchanged: under `checked` it follows from the input fact because both quotes are ASCII (SPEC LG30). The
  outer and hex loop invariants became `!checked ==> utf8_valid_span(input, start + 1, pos)`. No split was needed.
- `record` passes `true`; `token_at` (the `scan_into` path, grok) passes `false` and keeps the per-scalar check.
- `record`, `container`, `scan_records`, `scan` and `scan_document` require `utf8_valid_span(input, 0, len(input))`.
  `lines::parse_owned` already ran `validate_utf8(owned, 0, n)` before `scan`, so it was not changed.
- `scan`: the postcondition `strings_ok(...)` came back `verify_unknown` (canceled) with the new requires, and so
  did an `assert` of it inside the `Ok` match arm. An `assert is_ok(root) ==> strings_ok(input, tape, 0, len(tape) / 6)`
  placed before the `match` verifies, and so does the ensures after it. The generated Rust is unchanged by the assert.
- `parse::parse` now runs `lex::validate_utf8(input, 0, n)` first and returns `failure(invalid, 4)`, as the hand
  `parse_line` runs `str::from_utf8(line)` first. The old D33 path (per-scalar checks during the scan, then a
  `validate_utf8` of the rest after a grammar failure) is gone. Error results are the same: invalid UTF-8 was
  already reported, at its first byte, ahead of any grammar error (json `run_a0`-`run_a5` unchanged).
- `claude/entry::title_for_type` reads `type` once through `access::plain_str`. On `Some(t)` it compares the view by
  length and `eq_range` against `custom-title` / `ai-title` / `summary` and returns `NoTitle` for any other string.
  `None` (escaped, non-string or corrupt node) keeps the three `string_is` checks, so an escaped `type` gives the
  same result as before.

### Per-function structure (generated claude a1 Rust vs `claude/handrust/src/lib.rs`)

| Function | Pruv (generated Rust) | Hand (handrust/src/lib.rs) | Same? | Row |
|---|---|---|---|---|
| `parse::parse` / `lines::parse_owned` | one `validate_utf8` of the line, then the scan | `parse_line`: `str::from_utf8(line)`, then the parser | same (the check routine differs: 4.4l-VU) | closes 4.4m-SS |
| `parse::string_token` (from `record`) | one pass over `"`, `\`, `< 32`; bytes `>= 128` skipped, no per-scalar check; returns the plain bit | `Parser::string`: one `position` scan over the same bytes, `Cow::Borrowed` without escape | same pass and skip; Pruv carries a run-time `checked` parameter (constant `true` here; generated as a `&& checked` test per non-ASCII byte) | 4.4p-CF (new) |
| `record` / `container` / `scan_records` / `scan` | no per-value UTF-8 check | `value()`: `str::from_utf8` per string value chunk | Pruv does no more work than the hand (no row, as in 4.4n) | 4.4m-SS |
| `entry::title_for_type` | one `plain_str` read of `type`, then length + byte compares; `string_is` only for an escaped or non-plain node | `match get_str(e, "type") { Some("custom-title") .. }` | same for an unescaped `type` | closes 4.4n-TT |
| `entry::ranked` / `title_of`, `import::tail_info` / `read_window` | a new `String` per winning title | one reused buffer (`text.clear(); push_str(t)`) | no | 4.4m-TB part two (open) |

Independent structure comparison (Codex `gpt-6.2-astra`, review role, Codex 2 account; the default account was
at its limit). It got the generated `a1_root_gen.rs` excerpts and the hand `lib.rs`, without this table. Its
conclusions agree with the table:
- one whole-line UTF-8 check and one grammar pass on both sides; zero per-scalar checks on the record path (the hand
  adds a `from_utf8` per string value);
- an unescaped `type` is borrowed once and compared at most once (the three length tests differ, so at most one byte
  compare runs); an escaped `type` falls back to one to three `string_is` calls on the same id, with no second field
  lookup;
- `checked` is a constant at both call sites and the function is `#[inline(always)]`; whether the branch survives
  optimization was not checked. That is row 4.4p-CF.

It also listed differences that 4.4p did not change and that earlier rows already hold: the owned `source` copy per
parsed line (`input.to_vec()` in `scan_document`; the B-LLVM / R-6g6-2 copy), a non-object root parsed in full
before `entry::parse_line` rejects it, the `check_subtrees` pass in `publication_ok` (4.4l-CT), and the
input / record limits (16777216 bytes, 1048576 records) that the hand `parse_line` does not have (the A2 limits of
the shared parser, as in every earlier step). No wrong result on an accepted input was found.

| Row | Kind | Difference | What closes it | Where |
|---|---|---|---|---|
| 4.4m-SS | closed | per-scalar UTF-8 checks in `string_token` after the line check | done in 4.4p: the line fact reaches `string_token` through `record` / `container` / `scan_records` / `scan`; `parse` validates first | `pkgs/parse` |
| 4.4n-TT | closed | `title_for_type` scanned `type` up to three times | done in 4.4p: one `plain_str` read and view compares | `claude/entry` |
| 4.4p-CF | shape, new | `string_token`'s `checked` is a run-time `bool` (constant `true` from `record`, `false` from `token_at`) instead of two specialized scanners | a `token_at` path that also carries the line fact (then drop the flag), or a separate `string_token_checked`; verify the inliner folds the constant first | `pkgs/parse` |
| 4.4m-TB (part two) | shape, open | a new `String` per winning title, not one reused buffer | unchanged: a `str` buffer whose `clear` + append keeps valid UTF-8 | `claude/import`, `pkgs/jstr` |

### Consumer tests (step 4.4p)

pruvc-rs (`PRUVC_RS=pruvc-rs/target/release/pruvc`), pairs rs/rust and rs/llvm. Every consumer of `pkgs/parse`
was rerun, which covers `claude/entry` too.

| Runner | rs/rust | rs/llvm |
|---|---|---|
| claude `test_a0` | 272 / 272 | 272 / 272 |
| claude `test_a1` | 2897 / 2897 | 2897 / 2897 |
| claude `test_a2` | 2443 / 2443 | 2443 / 2443 |
| claude `test_a3` | 134 / 134 | 134 / 134 |
| claude `test_a4` | 270 / 270 | 270 / 270 |
| claude `q.py` | 20270 / 20270 | 20270 / 20270 |
| claude `q_mutants.py` | 18 mutants: 8 detected, 10 proof-rejected (mutant 3 by `verify_unknown`, as in 4.4n), none survive | (same run) |
| json `run_a0`-`run_a5` | 37 / 351 / 114 / 454 / 6344 / 83 | same |
| supervisor `test_a0`-`test_a3` | 522 / 410 / 78 / 35 | same |
| grok `test_impl --no-ts` (`GROK_COMPILE_WALL=3600`) | 254 / 254 | 254 / 254 |
| qwen `test_impl.py` | 26 / 26 | 26 / 26 |
| mcp `test_impl.py` | 22 / 22 | 22 / 22 |
| codex `test_impl.py` | 136 / 136 | 136 / 136 |
| opencode `test_impl.py` (`GROK_TS_TRANSPILER` set) | 199 / 199 | 199 / 199 |

`claude/q.py` exits 1 only for the frozen py pairs (`build: 'py'`); both rs pairs pass. No runner or oracle needed a
change. 4.4m-TB part two was not started within the cap. Nothing was measured.

## Step 4.4r: one reused title buffer (4.4m-TB part two), `string_token`'s constant flag folds (4.4p-CF), key lookup design (4.4n-KL)

- No compiler change and no `pkgs/` code change. Changed: `claude/entry`, `claude/import`, `claude/a1_root`.
- `claude/entry`: the `Title` enum is gone. `pub struct RankedTitle { rank: i64, text: [u8] }` is the hand
  `RankedTitle { rank: u8, text: String }`; `new_title()` returns rank 0 and an empty buffer (`vec![0u8; 0]`, no
  allocation). The buffer is `[u8]` because the catalog has no `string_clear` (gap C); its UTF-8 fact is
  `utf8_valid_span(st.text, 0, len(st.text))`, required and ensured by every function that writes it.
  - `title_key(doc, id, key, inout buf) -> i64`: on a nonblank candidate it runs `buf = truncate(buf, 0)` and
    `arr_append(inout buf, v, 0, len(v))` with the `str` (borrowed plain view or decoded value). Rust:
    `buf.truncate(0); buf.extend_from_slice(v.as_bytes())`, the hand `text.clear(); text.push_str(t)`. Appending a
    `str` to a valid prefix keeps the span fact (SPEC utf8 rules), so no run-time check is added. Returns 1 written,
    0 missing or blank (buffer unchanged), any other value a defect kind (`Field::Defect(0|1)` maps to -1 so 0/1 stay
    unambiguous).
  - `pick_title(.., inout buf)`: first key, then the second only when the first gave 0 (hand `pick`).
  - `ranked(doc, level, first, second, inout st)`: `level < st.rank` returns 0 before any lookup (hand
    `rank >= self.rank` before `pick`); `pick_title(.., inout st.text)`; `st.rank = level` only on 1. Ensures
    `old(st).rank <= st.rank`.
  - `title_of` / `title_for_type(doc, ty, inout st) -> i64`: same dispatch as 4.4p, now updating `st` in place.
- `claude/import`: `tail_info` and `read_window` keep one `RankedTitle` local across all lines (loop invariant
  `0 <= best.rank <= 3` and the span fact). `window_object` takes `inout best` and returns `bool`. The final title is
  `one_line(subview(best.text, 0, len(best.text)), 120)`: the `[u8]` span with the proven fact passes as a `str`
  (R-6g-10), generated as `from_utf8_unchecked` with no run-time check.
- Solver: with the struct in the module, two obligations came back `verify_unknown` (rlimit, "canceled"):
  `access::plain_str`'s precondition in `consume_entry`, and `window_object`'s precondition in `read_window`.
  Fixes: the string-content tail of `consume_entry` became the small `#[inline(always)]` function
  `answer_string(doc, c, at, inout col) -> i64` (0 = answered, else defect kind), with three `assert`s of its
  requires before the call; `read_window` asserts `doc_strings(d)` and `col.begun < 1000000000000` before
  `window_object`. The asserts produce no Rust.
- `claude/a1_root` (test wrappers): `pick_title_w` uses a local buffer and the new `pack_pick`; `title_of_w` uses
  `new_title()` and `pack_title(code, st)`. The wire format is unchanged.

### 4.4p-CF: the constant `checked` folds away

`string_token` is generated `#[inline(always)]` (Rust) / `alwaysinline` (LLVM backend) and both call sites pass a
literal: `record` `true`, `token_at` `false`. Checked on the optimized code of `record` (claude a1):
- rs/llvm: the pruvc `.ll` through `clang -O2 -S -emit-llvm`. `string_token` has no body left (inlined). In the
  inlined loop the only byte tests are `== 34`, `< 32`, `== 92`, the escape `switch` and the four hex tests. No
  `>= 128` test and no `validate_utf8` / `utf8_width` code remain.
- rs/rust: the generated Rust with `record` made public, `rustc -O --emit llvm-ir -C codegen-units=1`. Same result:
  the inlined loop tests `34`, `< 32`, `92`, the escape `switch` and hex digits only.

So the optimized `record` loop has the hand's shape (one scan over `"`, `\`, `0..=31`), and the `token_at` copy
keeps the per-scalar check that its unchecked input needs. Two specialized scanners in source would give the same
machine shape; the row is closed with this evidence and `pkgs/parse` is unchanged.

### 4.4n-KL: key lookup design (no code)

Hand: `object()` builds a `BTreeMap` per object at parse (`insert` overwrites, so the last duplicate wins), and each
`get` / `get_str` is a map lookup (`O(log n)` byte compares). Pruv: the tape has no index; `access::field` →
`find_ascii` walks every direct member of the object (`may_match` prefilter, then `ascii_equal`) and keeps the last
match. `pick_title` walks again for its second key; `consume_entry` does one walk per root key it reads (`type`,
`isSidechain`, `message`, `timestamp`, ...).

Options:
1. Map per object at parse (the hand shape). Needs a new tape side table and new `JsonDocument` invariants proved
   by `parse`, read by every `pkgs/access` consumer. It also adds the hand's per-object allocation that the tape
   avoids. Large; changes all consumers of `pkgs/json` / `pkgs/parse` / `pkgs/access`.
2. "Keys unique" fact from the parse, so a lookup can stop at the first match. JSON allows duplicate keys and the
   hand keeps the last, so the parse would have to detect duplicates (a per-object sort or hash). Not cheaper than 1.
3. One walk per object for a fixed key set: `access::fields(doc, id, keys) -> [i64]` (or a small fixed-arity
   struct) that walks the members once and overwrites slot k on each match of key k (last-wins as now). Contract per
   slot: `r >= 0 ==> r < len(doc.tape) / 6 && doc.tape[6 * r + 4] == id`, the same as `find_ascii`. The claude root
   reader would resolve its root keys in one walk per line instead of one walk per key. No tape or parse change;
   only `pkgs/access` (new function) and `claude/entry` / `claude/import` (callers).

An earlier step removed a one-pass root reader (`entry::root_of` filled by `fields_into`) in favour of one
`field` call per key, because the hand calls `get` per key (table "lex and access" above). Option 3 brings a single
walk back, but for the reason the hand has a map: each key costs one resolution, not one member walk.

Recommendation: 3. It matches the hand's cost model (each key resolved once per object, last-wins once) without a
parse-time map. Proof risk: the per-slot loop invariant over a key list; keep the key list a fixed small arity
(one function per caller key set, or a struct of `i64` slots) to avoid quantified invariants over `[str]`. Measure
only after the title path and this are both in place.

### Per-function structure (generated claude a4 Rust vs `claude/handrust/src/lib.rs`)

| Function | Pruv (generated Rust) | Hand (handrust/src/lib.rs) | Same? | Row |
|---|---|---|---|---|
| `entry::RankedTitle` / `new_title` | `{ rank: i64, text: Vec<u8> }`, created once before the line loop, empty `vec![0u8; 0]` | `#[derive(Default)] { rank: u8, text: String }` in `State` / `tail_packet` | same (one buffer, no allocation at creation; `[u8]` for `String`: gap C) | closes 4.4m-TB part two |
| `entry::title_key` | nonblank candidate: `buf.truncate(0); buf.extend_from_slice(v.as_bytes())`; blank or missing: buffer unchanged | `pick` returns `&str`, `update` does `text.clear(); text.push_str(t)` | same copy count (one per winning title, into the reused buffer); the clear + append sits in `title_key`, not after `pick` | closes 4.4m-TB part two |
| `entry::pick_title` | second key only when the first gave 0 | `for k in [k1, k2]`, first nonblank | same | — |
| `entry::ranked` | `level < st.rank` returns before any lookup; `st.rank = level` after a write | `if rank >= self.rank { if let Some(t) = pick(..) { rank; clear; push_str } }` | same order (rank check before the pick); rank and text written in the other order, same final state | — |
| `entry::title_of` / `title_for_type` | one `string_id` of `type`, then the 4.4p view compares | `match get_str(e, "type")` | same as 4.4p | — |
| `import::tail_info` / `read_window` (title) | `rank > 0` → `one_line(from_utf8_unchecked(&best.text[..]), 120)` | `finish`: `(rank > 0).then(\|\| one_line(&self.text, 120))` | same (no copy, no run-time UTF-8 check) | — |
| `import::window_object` | `title_for_type(d, ty, best)` in place, then `consume_entry`, then `window_model` | `consume`: `st.title.update(e)`, then the entry | same order; returns `bool` instead of the candidate | — |
| `import::answer_string` (new, `#[inline(always)]`) | the string-content tail of `consume_entry`: borrowed view or decoded value, `answer` once | inline `Value::String(s) => col.answer(s, ..)` | same after inlining (split for the solver) | — |

Independent structure comparison (Codex `gpt-6.2-astra`, review role, Codex 2 account; the default account was
at its limit). It got the generated Rust excerpts (`a4` gen) and the hand `lib.rs` excerpts, without this table.
Its conclusions agree on the rows above: one title buffer reused across records in both `tail_info` and
`read_window`, the rank check before the candidate lookup, a later equal-rank nonblank title wins, and the final
title borrowed from the buffer with no UTF-8 re-check. It found no wrong-result path: a `Field::Defect(0|1)` maps to
-1 and the callers accept only 0 and 1; `answer_string` maps `InternalInvariant(0)` to -4 and the caller rejects any
nonzero code. Its one open question, whether `access::string_append` can return a positive defect, is answered by
its contract (`ensures result <= 0`). The other differences it listed are earlier rows, unchanged here: an escaped
candidate decoded at the lookup (4.3h-T placement), the owned user prompt and the model written after the content
(4.4c-MB), the extra frame / size / packet checks (A2 limits), and `read_window`'s `plan` / `check_output` helpers.

| Row | Kind | Difference | What closes it | Where |
|---|---|---|---|---|
| 4.4m-TB (part two) | closed | a new `String` per winning title | done in 4.4r: `RankedTitle { rank, text: [u8] }` updated in place by truncate + `str` append | `claude/entry`, `claude/import` |
| 4.4p-CF | closed | `string_token`'s `checked` is a run-time flag | evidence in 4.4r: the literal folds after the forced inline on both backends (section above); no code change | `pkgs/parse` |
| 4.4n-KL | shape, open (designed) | one member walk per key lookup vs a map per object | option 3 above: one walk per object for a fixed key set in `pkgs/access` | `pkgs/access`, `claude/entry`, `claude/import` |

### Consumer tests (step 4.4r)

Only `claude` changed, so only the claude runners ran (`PRUVC_RS` = the existing release `pruvc`, not rebuilt).

| Runner | rs/rust | rs/llvm |
|---|---|---|
| `claude/test_a0.py` | 272/272 | 272/272 |
| `claude/test_a1.py` | 2897/2897 | 2897/2897 |
| `claude/test_a2.py` | 2443/2443 | 2443/2443 |
| `claude/test_a3.py` | 134/134 | 134/134 |
| `claude/test_a4.py` | 270/270 | 270/270 |
| `claude/q.py` (270 fixed + 20000 generated) | 20270/20270 | 20270/20270 |
| `claude/q_mutants.py` (18) | 8 detected, 10 proof-rejected, 0 survived | (rust backend only) |

`q.py` exits 1 only because of the frozen py pairs (`ERROR build: 'py'`). Mutant 4 (title rank `<` to `<=`) went
stale because its old snippet named the removed `Title` enum; it now targets `if level < st.rank { return 0; }` and is
detected by fixture 171 ("titles tie later wins").

## Step 4.4s: `pkgs/access` member cursor, claude root keys in one walk (4.4n-KL option 3)

New in `pkgs/access` (existing functions unchanged):

| Function | Contract | Body |
|---|---|---|
| `members_end(doc, id) -> i64` | `result >= 0 ==> 0 <= id < result <= len(tape)/6` | -3 bad id, -2 not an object, else `tape[6id+3]` (the skip link) |
| `member_next(doc, id, j, stop) -> i64` | requires `id < j < stop <= len(tape)/6`; `result >= 0 ==> j < result <= stop`, `tape[6j] == 7`, `tape[6(j+1)+4] == id` | `find_ascii`'s per-member checks (key kind, parent, value in range, value parent, skip in range); -4 = corrupt tape |
| `key_is(doc, j, key) -> bool` | requires `0 <= j < len(tape)/6` | `may_match(source, s, e, key) && ascii_equal(source, s, e, key)` |

Option 3's `access::fields(doc, id, keys)` could not be written generically: no `[str; N]` and no const generics
(row 4.4s-FK). The fixed key set is in the caller, the Rust `for (k, v) in obj { match k { .. } }` shape:
`entry::root_keys` walks the root once with the cursor and sets one of 13 `i64` slots of `RootKeys` per matching key.

### Per-function structure (generated claude a4 Rust vs `claude/handrust/src/lib.rs`)

| Function | Pruv (generated Rust) | Hand | Same? | Row |
|---|---|---|---|---|
| `entry::root_keys` | `members_end`, then `while j < stop { n = member_next(..); if key_is(.., "type") { k.ty = v } else if .. ; j = n }`; `RootKeys` by value, no allocation | `object()` builds a `BTreeMap` per object at parse; each `get` is a map lookup | cost model same (each root key resolved once per object, last wins); shape differs (no map) | 4.4n-KL closed for the root |
| `import::consume_entry` / `user_text` / `title_for_type` | slot reads (`string_of(k.ty)`, `true_of(k.side)`, `object_of(k.message)`, `stamp_from(k.timestamp)`) | `get_str(e, ..)`, `is_true(e, ..)`, `e.get(..)` | same per-key reads, no member walk per key | — |
| nested `message.content`, `origin.kind`, block keys | `access::field` per key | `get` per key on the nested map | as 4.4r | — |
| `import::window_line` (new) | parse + `window_object` for one line; -1 accepted / skipped, else the `fail_at` position | inline in the line loop | same after inlining (split for the solver) | — |

4.3h-R (the reason `root_of` was removed): `root_of` allocated an 8-word `[i64]` and did 7 `field` walks for every
object. `root_keys` allocates nothing, walks once, and an empty object reads only the skip link.

| Row | Kind | Difference | What closes it | Where |
|---|---|---|---|---|
| 4.4n-KL | closed (root) | one member walk per root key | done in 4.4s: `root_keys` on the access cursor | `pkgs/access`, `claude/entry`, `claude/import` |
| 4.4s-FK | closed (L-gaps 25) | no generic `fields(doc, id, [&str; N]) -> [i64; N]` | R-6t-5 (`const N: i64`, static `[str; N]`); `access::fields` added, `root_keys` uses it | `pkgs/access`, `claude/entry`, `claude/import`, compiler |

### Consumer tests (step 4.4s)

`pkgs/access` changed, so every consumer ran on both backends (`PRUVC_RS` = the existing release `pruvc`, copied
unchanged; not rebuilt).

| Runner | rs/rust | rs/llvm |
|---|---|---|
| claude `test_a0`-`test_a4` | 272 / 2897 / 2443 / 134 / 270 | same |
| claude `q.py --pairs rs/rust,rs/llvm` | 20270/20270 | 20270/20270 |
| claude `q_mutants.py` (18) | 9 detected, 9 proof-rejected, 0 survived | (rust backend only) |
| json `run_a0`-`run_a5` | 37 / 351 / 114 / 454 / 6344 / 83 | same |
| supervisor `test_a0`-`test_a3` | 522 / 410 / 78 / 35 | same |
| grok `test_impl.py --no-ts` (`GROK_COMPILE_WALL=3600`) | 254 / 254 | 254 / 254 |
| qwen `test_impl.py` | 26 / 26 | 26 / 26 |
| mcp `test_impl.py` | 22 / 22 | 22 / 22 |
| codex `test_impl.py` | 136 / 136 | 136 / 136 |
| opencode `test_impl.py` (`GROK_TS_TRANSPILER` set) | 199 / 199 | 199 / 199 |

Mutant 1 (isMeta filter inverted) went stale (its snippet named the removed `is_true(doc, root, "isMeta")`); it now
targets `user_text`'s `true_of(doc, k.meta)` line and is detected by fixture 0 ("head0: basic").

## Step 4.4zc: rows 4.4l-RE, 4.4l-VU, 4.4l-RS, 4.4l-CT

Changes `pkgs/jstr`, `pkgs/lex`, `pkgs/parse`, `pkgs/lines`, `claude/entry.pruv` and the `parse::parse` callers
(grok `summary`, json `a2`/`a3`/`a4`/`b`/`q` roots, opencode `convert`/`plan`). No compiler or hand change.
Hand reference: `claude/handrust/src/lib.rs` (`Parser::escape`, `object`, `value`, `parse_line`) and std
`core::str::validations::run_utf8_validation`.

| Row | Function | Pruv now | Hand / std | Same? |
|---|---|---|---|---|
| 4.4l-RE | `jstr::string_utf8`, `jstr::scalar` | `string_utf8` decodes with `decode_utf8(.., true)` (bytes already validated by the scanner, no second UTF-8 pass); `scalar` reads the low surrogate only after `a[np] == '\\' && a[np+1] == 'u'`, its four hex digits directly | `String::from_utf8(bytes)` once; `escape`: `if self.eat("\\u") { let low = self.hex4()?; .. }` | yes |
| 4.4l-VU | `lex::validate_utf8` | one loop: an ASCII byte at `i % 8 == 0` starts the skip of all-ASCII 16-byte blocks (`ascii_block`), then single ASCII steps; a lead byte checks its width's continuation bytes in turn (no inner loop) | `run_utf8_validation`: pointer-aligned two-word `NONASCII_MASK` test, per-width continuation checks | loop shape yes; block test and alignment differ (gaps below) |
| 4.4l-RS | `parse::scan_records`, `parse::open_container`, `parse::parse(input, object)` | object root: skip whitespace, EOF = kind 5, first byte not `{` = kind 6, then `open_container(.., 6)`; else `record`. `record`'s `[`/`{` arms call `open_container`. No tape reserve. `parse` ensures `object && ok ==> tape[0] == 6`; claude `entry::parse_line` drops its root-kind test | `parse_line`: `ws(); object(1)?; ws(); is_empty`; `object` starts with `eat("{")` | yes |
| 4.4l-CT | `lines::parse_owned` | `parse::scan` ensures the record-local facts (`rec_ok`, check_tape's first pass), so `parse_owned` runs only `json::check_subtrees` | no publication check | no: `check_subtrees` is the D32 proof gaps G-DB33-2 (parent loop) and G-DB33-1 (subtree pass); L-gaps 21 attempt left on a side branch (row below); L-gaps 23 branch `lair/pruv-l-gaps-23-co-1258` drops it (proves, a5_root 17.5 s against 7.3 s), not merged; L-gaps 24 side branch `lair/pruv-l-gaps-24-ct-wip` (`opaque pred fn` tree predicates) 11.3 s, a2_root fails, not merged |

`lines::build_span` (grok `scan_into`) still calls `check_tape`; it is not in these rows.

| Row | Kind | Difference | What closes it |
|---|---|---|---|
| 4.4zc-WL | language gap, closed (L-gaps 18) | no word (`u64`) load from `[u8]`, so `ascii_block` is a 16-byte min reduction instead of std's `(block \| block2) & NONASCII_MASK == 0` | an unaligned word read of `[u8]`: `load_le` already reads it; L-gaps 18 lets the verifier prove bit tests over it |
| 4.4zc-PA | language gap, open | no pointer address, so the block skip starts at `i % 8 == 0` (offset) instead of std's pointer alignment `align_offset` | a byte-array base address or `align_offset` |
| 4.4l-CT | proof gap, open | `check_subtrees` still runs at publication | G-DB33-2 and G-DB33-1 (D32): prove parent containment and the subtree facts in `scan`. L-gaps 21 attempt (no split lemma: `record` / `open_container` ensure the per-record child-count step, `container` carries linkage, subtree facts, child count and key parity as loop invariants) is on side branch `lair/pruv-l-gaps-21-row3-wip` (820edda0), not merged: `open_container`'s `tree_ok` postcondition fails and four `container` facts hit the solver limit (see the L-gaps 21 section in `bench/l3/lib/RESHAPE.md`). L-gaps 22 attempt (`set_header`, `child`, `member` helpers and a verifier link between a plain `count` and a quantified one at a ground instance) proves on side branch `lair/pruv-l-gaps-22-row2-wip` (7df89c86) in three fresh runs, not merged: a5_root verify is 30-34 s against 7.3 s (see the L-gaps 22 section in `bench/l3/lib/RESHAPE.md`). L-gaps 23 (verifier change merged; sources on branch `lair/pruv-l-gaps-23-co-1258`, not merged): the verifier starts a goal that reads an aggregate at the lemma stage; with that, three fresh runs pass at 17.5 s, still over the 8.8 s target; the remaining time is the new proofs (see the L-gaps 23 section in `bench/l3/lib/RESHAPE.md`). L-gaps 24 (language feature `opaque pred fn`, SPEC R-6o-5, merged; sources on side branch `lair/pruv-l-gaps-24-ct-wip`, not merged): with the tree predicates opaque, a5_root verifies at 11.3 s (target 8.8 s) and a2_root fails on the `child` count postcondition (see the L-gaps 24 section in `bench/l3/lib/RESHAPE.md`) |

### Consumer tests (step 4.4zc)

`PRUVC_RS` = `/Users/spica4345/Projects/pruv/pruvc-rs/target/release/pruvc` (not rebuilt). Final source, both
backends.

| Runner | rs/rust | rs/llvm |
|---|---|---|
| json `run_a0`-`run_a5` | 37 / 351 / 114 / 454 / 6344 / 83 | same |
| claude `test_a0`-`test_a4` | 272 / 2897 / 2443 / 134 / 270 | same |
| claude `q.py` | 20270/20270 | 20270/20270 |
| qwen `test_impl.py` | 27 / 27 | 27 / 27 |
| grok `test_impl.py` (`GROK_TS_TRANSPILER` set) | 254 / 254 | 254 / 254 |
| grok `q.py` | 0 unexplained | 0 unexplained |
| codex `test_impl.py` | 136 / 136 | 136 / 136 |
| codex `q.py` (2023 cases) | 0 unexplained (1633 exact, rest P1-P8) | same |
| mcp `test_impl.py` | 22 / 22 | 22 / 22 |
| opencode `test_impl.py --jobs 2` | 199 / 199 | 199 / 199 |
| opencode `q.py` | 0 unexplained | 0 unexplained |
| qwen `q.py` (rs pairs) | 0 unexplained (19757 same, P1 64, P4 201, P5 1; lists 21 same, P4 1) | same |
| mcp `q.py` (rs pairs, 2040 requests) | rc 0 (2027 exact, rest named classes) | same |

The py pairs in the q.py scripts have no runners in this worktree (only the rs compiler is built), so claude and
codex `q.py` exit 1 on them; qwen and mcp `q.py` ran on the rs pairs through a wrapper that drops the py pairs from
the pair list in memory (`q.py` unchanged).

Review: one Codex structure review (default account) found RE, VU and RS not equal (the low-surrogate read before
`\u`, the continuation loop, the duplicate root dispatch in `record`); CT is the recorded gap. All three were fixed.
The confirm review gave all fixed and one new RE item (`\u` tested in `scalar` and again in the step); fixed by the
direct hex read above. Structure is not zero (gaps above), so performance was not measured.

### Rows after 4.4ze

- 4.4x-DS (`jstr` `decode_append`, scanned mode ensures no UTF-8 fact) stays open, blocked by the new verifier gap
  4.4ze-UW: the sub-window `utf8_valid_span(b, q, e)` after an ASCII byte is not derived from
  `utf8_valid_span(b, lo, hi)` when `lo` is symbolic (window encoded over `select(lo + i)`).
- Design chosen for after the gap: the literal-backslash rule in `step`/`scalar` (an escape is decoded only if it
  ends on an ASCII byte; `ensures a[p] == 92 ==> a[result.next - 1] < 128`). The source-requires option is rejected
  (false on valid input `"\\é"`). Full rows: `../lib/RESHAPE.md` "Rows after 4.4ze".
- No package source changed, so no consumer run. Not measured.

### Rows after L-gaps 17

- 4.4x-DS closed (`jstr`): the verifier gap 4.4ze-UW is fixed, and design (b) is in. `step` and `scalar` decode an
  escape only if it ends on an ASCII byte (`\` then ASCII, `\u` with four ASCII hex digits, the low surrogate
  likewise) and ensure `a[p] == 92 ==> a[result.next - 1] < 128`; otherwise `\` is one literal byte (a `\u` with a
  non-ASCII digit included, pinned by an ensures).
  `decode_append` ensures `(utf8_valid(old(out)) && utf8_valid_span(source, start + 1, end - 1)) ==> utf8_valid(out)`.
  Valid JSON gives the same bytes. Full rows: `../lib/RESHAPE.md` "Rows after L-gaps 17".

### Rows after L-gaps 18

- 4.4zc-WL closed (`lex`): `ascii_block` is std's two-word test, `let w0 = load_le(input, i, 8); let w1 =
  load_le(input, i + 8, 8); return band(bor(w0, w1), NONASCII_MASK) == 0;` (`-9187201950435737472` is
  `0x8080808080808080`). The verifier now proves `result ==> forall j in 0..16: input[i + j] < 128` over the loaded
  words; contracts unchanged. Full rows: `../lib/RESHAPE.md` "Rows after L-gaps 18".
- 4.4l-VU now differs from `run_utf8_validation` only in the alignment (4.4zc-PA, still open: offset `i % 8 == 0`
  instead of the pointer's `align_offset`).

### Rows after L-gaps 25

- 4.4s-FK closed. SPEC R-6t-5 adds `const N: i64` function parameters (N inferred from an argument length or given
  as `::<N>`) and static `[str; N]` (literal elements only; by-value parameter or immutable local). Each concrete N
  is one instance on both backends (Rust `[&'static str; N]`, LLVM `[N x { ptr, i64 }]`).
- New in `pkgs/access`: `fields<const N: i64>(doc, id, keys: [str; N]) -> [i64; N]`. `members_end`, then one walk
  with `member_next`; for each member an inner loop over `keys` sets `r[i] = j + 1` on `key_is` (last match wins);
  `[stop; N]` for a bad id / non-object, `[-4; N]` for a corrupt tape, `-1` for a missing key.
  `entry::root_keys` is `access::fields(doc, 0, [13 keys])`; `RootKeys` is gone and `entry` / `import` read
  `k[0]`..`k[12]` (slot order in the comment above `root_keys`).
- Structure against hand m11 `fields<const N: usize>` (`r = [None; N]`, kind check, `for _ in 0..count`, inner
  `for j in 0..N` with `keys[j].as_bytes() == k`): same (one walk, inner key loop, last wins, no allocation). The
  generated Rust is `access_6_fields_1_k13` in `claude/work/a4/rs_rust/a4_root_gen.rs`.
- Performance (claude `q_measure.py --pairs rs/rust,rs/llvm`, 7 rounds, new and base a2726b61 back to back with the
  same compiler under the perf lock, load about 3; cells in `work/lgaps25/perf_cells.json`): (Pruv/hand new) over
  (Pruv/hand base) geometric mean 1.037, turns10-200 1.07-1.12, other inputs 0.98-1.06. The base `if`/`else if`
  chain stopped at the first matching key; `fields` (like the hand) tries all 13 keys per member, and `key_is`
  (`may_match` + `ascii_equal`) costs more than the hand's length check + `memcmp`. Open as 4.4zf-KE (`key_is`
  against slice equality); the structure is the same.
- Audit fixes before the close (L-gaps 25): `[str; N]` cell facts are quantified under `forall`;
  `Option<[T; N]>` checks the `Element` bound; a `str` alias names the element; an explicit `::<N>` that disagrees
  with an argument length is `type/fixed_array_length`. `for` over `[str; N]` and array patterns over a
  length-parameter array are unsupported in SPEC R-6t-5 (index loop and `len` tests instead). The fixes touch the
  checker and verifier only, so the measured code is unchanged.

### Rows after L-gaps 26

- 4.4zf-KE open. Tried in `jstr::ascii_equal`: compare the plain prefix (bytes below 128, not a backslash) byte by
  byte against the key and decode with `step` only from the first escape or high byte, in place of the
  `stop - p == klen` plain fast path. Measured (claude `q_measure.py --pairs rs/rust,rs/llvm`, 7 rounds, new and base
  0e77a02b back to back with the main compiler under the perf lock, load under 3; local `work/lgaps26/perf_cells.json`
  and `perf_cells_base.json`): (Pruv/hand new) over (Pruv/hand base) geometric mean 1.02, turns10-200 cells within
  noise (window 1.17-1.33x hand on both), so the change was reverted. The base fast path already handles plain keys
  with one length check and a byte loop; `key_is` is not where the 1.2-1.4x on turns and window_8MiB comes from.
  Next: profile the generated a4 `window` path (q_measure deletes its build directory, so the profile needs a kept
  build) before touching shared code again. Pruv still has no `str` / `[u8]` slice `==`, so the key compare stays a
  byte loop either way.

### Rows after L-gaps 27

- 4.4zf-KE: window and turns fixed in shared code. A kept a4 build was profiled on `window_8MiB` (generated Rust
  about 40 ms, hand about 29 ms per call): about 1400 of 5000 samples sat in `malloc` / `realloc` / `free`, most of it
  the line tape growing from capacity 0 to 96 words (about five reallocations per JSONL line). 4.4zc had removed the
  up-front reserve that the json hand `parse` keeps (`Vec::with_capacity(32 * WIDTH)`). `parse::scan_records` now
  reserves 192 words (32 records) before the first append. That is one place, used by every `parse::parse` /
  `parse::scan` caller. Per-function table and probe table: local `work/lgaps27/compare.md`.
- The L-gaps 25 turns cells (1.07-1.12x) have the same cause. Every turn line starts a new tape, and `fields` was not
  the cost (L-gaps 26 had already shown that `key_is` is not it).
- Measured with claude `q_measure.py --pairs rs/rust,rs/llvm`, 7 rounds, under the perf lock at load about 2 (local
  `work/lgaps27/perf_cells.json`, 36 time cells per pair). Pruv/hand time geometric mean 0.927 on rs/rust and 0.922
  on rs/llvm.
  - window_8MiB: 0.95 / 0.94.
  - turns0-200, all operations: 0.75-1.0.
  - small: 0.70-0.80.
  - ring_wrap: 0.89-1.02.
- Two cells stay above hand:
  - depth_limit window: 1.76 / 1.88. The time is in `open_container` and the inlined `call`, which is
    `publication_ok` -> `json::check_subtrees`. That subtree walk is quadratic in depth on nested input, so this cell
    is 4.4l-CT, not a parse shape. The 4.4l-CT side branches drop the walk only in `lines::parse_owned`; claude
    `parse::parse` still calls it through `publication_ok`.
  - long_text window on rs/rust: 1.20 (0.90 on rs/llvm). The hot loop is the per-byte loop of
    `parse::string_token` (inlined in `record`, called with `checked = true`). The hand `Parser::string` scans with
    `bytes().position(..)`, which looks only for a quote, a backslash or a control byte.
    - Both sides validate the line's UTF-8 once, and neither makes an extra pass.
    - The Pruv loop does more per byte: it also tests `c >= 128 && checked` and the backslash, and it moves an `i64`
      `pos` through `inout` with `as usize` / `as i64` casts. The read uses `get_unchecked`, so no bounds check is
      left in the loop.
    - The extra branches come from the Pruv source: one function serves both checked and unchecked input. The casts
      come from how the Rust backend lowers `i64` indexes.
    - Possible shared fix, not tried in L-gaps 27 (time cap): give `pkgs/parse` a scan for checked input that only
      advances to the first quote, backslash or control byte, and use it on the `record` string path.
    - Open as 4.4zf-LT.
- Peak RSS stays slightly above hand on most cells (tools/perf_gate.py: 29 failing cells, all better than L-gaps 26):
  1.00-1.06 on M/L cells (turns16-200, ring_wrap, window_8MiB, depth_limit, long_text), largest on rs/rust
  (turns200 head 1.057). The gap is 16-110 KB, roughly flat across sizes, so it looks like a fixed share; this has not
  been split into fixed and proportional parts yet. Open as 4.4zf-RS.

### Rows after L-gaps 28

- 4.4zf-LT: long_text window fixed in shared code; most of the cost was not the scan loop.
  - `parse::record` now calls `string_token_valid`, a scan for input that is already known to be valid UTF-8. It
    stops only at a quote, a backslash or a control byte, like the hand's `bytes().position(..)`. The old
    `string_token` lost its `checked` parameter and now serves only the unvalidated `token_at` path. On its own this
    did not move the cell (1.22): a kept a4 build, profiled with `sample`, showed the record scan loop already about as
    fast as the hand's.
  - The cost was in the claude user-text path. `string_field_of` / `string_content` call `access::string_text_owned`,
    which always ran the `jstr` escape decode. That decode reads every byte to look for a backslash that a plain
    string does not have. `string_text_owned` now checks the tape plain bit (slot 5 == 1) and copies the raw span with
    `jstr::string_text_plain_owned`, which is the hand's `Cow::Borrowed` case. Both branches give the same text.
  - Measured under the perf lock, load about 2 (claude `q_measure.py --pairs rs/rust,rs/llvm`, 7 rounds; local
    `work/lgaps28/perf_cells.json`; 11-round long_text run in `work/lgaps28/long_text_q.json`):
    - long_text window: 1.20 -> 1.029 on rs/rust, 0.90 -> 0.729 on rs/llvm.
    - No time cell got worse by more than 0.03 against L-gaps 27; turns and ring_wrap moved 0.03-0.18 lower.
  - The `i64` casts are not a cost: the scan loop assembly is the same length as the hand's.
  - The last 3 % on rs/rust is representation, not a loop. `parse::parse` copies the line into
    `JsonDocument.source`, and the reply keeps an owned copy of the text. The hand borrows both from the input line.
    Removing the copies needs a borrowed document (tape plus `&[u8]` source with a lifetime), which Pruv does not have.
    Open as 4.4zg-LB (borrowed document source).
- 4.4zf-RS: the peak RSS gap is a fixed share, not a proportional one. Kept builds were run with `/usr/bin/time -l`,
  hand and Pruv rs/rust alternating, 7 runs each, median:

  | input (window unless named) | input bytes | hand KiB | Pruv KiB | Pruv - hand KiB |
  |---|---:|---:|---:|---:|
  | empty file | 0 | 1600 | 1632 | 32 |
  | turns1 | 195 | 1616 | 1680 | 64 |
  | turns10 | 1950 | 1712 | 1792 | 80 |
  | turns200 | 39580 | 1984 | 2080 | 96 |
  | turns200 head | 39580 | 1680 | 1776 | 96 |
  | ring_wrap | 399780 | 2336 | 2432 | 96 |
  | window, first 1 MiB | 1048493 | 2960 | 3072 | 112 |
  | window, first 4 MiB | 4194177 | 6064 | 6176 | 112 |
  | window_8MiB | 8388476 | 10160 | 10256 | 96 |
  | long_text | 7340032 | 11152 | 11216 | 64 |
  | depth_limit | 952890 | 2800 | 2928 | 128 |
  | block_arrays | 3392750 | 9168 | 7104 | -2064 |

  - The gap does not grow with input: 32 KiB with no input, 64-128 KiB as soon as code runs, the same from 195 bytes
    to 8 MiB. There is no proportional part to fix.
  - Cause: code size. The Pruv runner has 351676 bytes of `__text` against 282364 for the hand runner (about 68 KB,
    17 pages more); `__DATA` is the same size. The 32 KiB at zero input is the larger image mapped at start; the rest
    is the extra code pages that the parse, access and reply paths touch once. No start buffer differs: both reserve
    their buffers lazily, and block_arrays shows the Pruv buffers below hand.
  - perf_gate still counts these cells as proportional failures (28 cells, 1.004-1.047, all RSS except depth_limit
    time and long_text rs/rust time). They are fixed-share debt. Which functions make up the 68 KB has not been
    broken down yet (next: symbol sizes of both runners). Open as 4.4zg-CS (generated code size).

### Rows after L-gaps 29

- 4.4l-CT: closed for `lines::parse_owned` and claude `parse::parse` (L-gaps 29 section in `bench/l3/lib/RESHAPE.md`).
  `pkgs/parse` proves parent containment and the subtree span facts in `scan`, so neither path runs
  `json::check_subtrees` any more. The child count and key parity facts are not proved; json b_root `array_length` is
  the only user, and it calls `parse::parse_counted`, which keeps the run-time pass.
  - Measured with claude `q_measure.py --pairs rs/rust,rs/llvm`, 7 rounds, main compiler, under the perf lock at load
    2-3 (local `work/lgaps29/perf_cells.json`, `perf_gate.txt`): depth_limit window 1.76 / 1.88 -> 0.351 / 0.486
    (Pruv/hand, rs/rust / rs/llvm); window_8MiB 0.843 / 0.769; long_text window 1.028 / 0.719 (unchanged).
  - tools/perf_gate.py: 27 failing cells, against 28 after L-gaps 28: 26 RSS cells (4.4zg-CS) and long_text window
    on rs/rust (4.4zg-LB). No other time cell is above hand.
- 4.4zg-CS: the code-size gap is the shared inline policy, not one function. Symbol sizes of the kept a4 runners
  (`nm -S` / `otool -tV`, local `work/lgaps29/symbols.md`): `__text` 351676 B against 282364 B for the hand, +69312 B.
  - The harness `main::call` alone is 228 B in the hand runner and 88152 B in the generated one (+87924 B).
  - The separate solution functions are 10296 B smaller than the hand's, and core / alloc code is 8268 B smaller.
    std is the same, and panic and bounds sites are equal within a few.
  - Cause: 249 of the 290 generated functions carry `#[inline(always)]`; the hand `lib.rs` has none. They come from
    the SPEC inline policy (`pruvc-rs/crates/pruv-core/src/inline_policy.rs:1-7`, constants in `shared_tables.rs:24-26`,
    shared with the Python compiler). A non-recursive function is forced inline when its inclusive size is at most
    INLINE_SMALL (8), when it has exactly one call site, or when its extra copies fit INLINE_BUDGET (1500).
  - The one-call-site rule folds each packet entry (`window_packet`, `head_packet`, `tail_packet` and their helpers,
    about 25 KB as separate symbols in the hand) into its only caller, the harness `call`. Each inlined path keeps its
    own copies of the Vec grow calls (110 direct calls to `do_reserve_and_handle` inside `call`).
  - Not changed: the policy is shared by every bench and the time cells depend on it. A size-aware rule (for example, no
    forced inline of a function above some size into an exported entry) needs its own row with a full time
    comparison. The RSS cells at 1.004-1.047 stay fixed-share debt under this row.
- 4.4zg-LB: language gap, open. The hand borrows the JSONL line for the document source (`Document<'a>`) and the
  prompt / reply text. A Pruv borrowed document would be a `ref` struct holding the tape and a `ref [u8]` source. It
  cannot be used here: `JsonParse::Parsed` and claude `Line::Object` carry the document as an enum payload
  (`claude/entry.pruv:6`, `30-31`; qwen `entry.pruv:3`, `24`), and qwen `import.pruv:498`, `528-533` keeps a
  `Vec<JsonDocument>`. A borrowing struct may not be an enum payload, a `Vec` / `Option` element or a generic argument
  (SPEC R-6g-11, `SPEC.md:1685-1687`; R-6g9-8 at `SPEC.md:2156` allows only view wrappers such as `Option<ref str>`). Passing the tape and
  status separately and building a local view in each caller would work around the gap rather than close it, so it
  is not done. Needed: a borrowing enum payload (an enum that is itself `ref` when a payload is), or a borrowed
  document form in `pkgs/json`.
  - Within the language, the user prompt no longer takes an owned copy. `claude/entry.pruv` `user_string_node` runs
    `user_text`'s checks and returns the string node. `claude/import.pruv` `human_string` then borrows the plain string
    with `access::plain_str` (`Option<ref str>`), as `answer_string` already does for replies. Escaped strings keep the
    owned decode. Results and defect order are unchanged (claude a0-a4 pass).
  - Re-measured under the perf lock at load 2.1 (local `work/lgaps29/perf_cells_r3.json`): long_text window
    1.028 -> 1.000 / 0.697 (rs/rust / rs/llvm). That meets the at-or-below-hand limit for the cell. The only time cells
    above 1.0 are window_8MiB tail rs/llvm (1.024) and turns0 window rs/rust (1.976), both at the one-tick timer floor
    (41-83 µs).
  - The source copy of the line (the document's own `[u8]`) is still the open part of this row.

### Rows after L-gaps 30

- 4.4zg-CS: closed. The `__text` gap came from the copy-budget arm of the Rust compiler's inline policy, not from the
  one-call-site arm. Each generated function's inline marks were replayed with a port of the policy on the claude a4
  runner (local `work/lgaps30/cs_policy.py`; the unchanged port gives the same 376380 B as the compiler):
  - `__text`: hand 282364 B; policy before 376380 B (251 forced-inline functions); without the copy budget 289340 B
    (208); without the one-call-site arm 318204 B; no forced inline at all 287868 B.
  - The budget let a function of up to 1500 lines be copied into a second caller, and each copy kept its own Vec grow
    paths. Removing it keeps the small-function and one-call-site arms.
  - Rust compiler only: `pruvc-rs/crates/pruv-core/src/inline_policy.rs` now forces inline for `incl <= INLINE_SMALL`
    or exactly one call site (SPEC R-7-3). The frozen Python compiler and the LLVM rule R-7-7 are unchanged.
  - Time, claude `rs/rust` against hand, 7 alternating rounds under the perf lock at load about 1.9: every time cell
    stays at or below hand within noise, long_text window 1.012 -> 1.006; the RSS cells drop from 1.04-1.07 to
    0.99-1.03. json `run_r03_measure.py`, old and new compiler alternating: all cells at or below hand
    (query_chain1 L 9.15 -> 7.72 ms against hand 8.4-9.2 ms).
