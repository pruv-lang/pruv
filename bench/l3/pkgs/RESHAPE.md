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
| T1 | `new_state`, `new_slots`, `new_buffer`, `empty_buffer`, `grow` | `VecDeque::new()`, per-turn `String::new()` | packets | owned | fill loops | up-front slabs; `grow` = new buffer + copy | Slabs: **ABI / V** (T0). `grow` is **=** (step 2.9): `take buf` + `resize(move b, ..)` (`Vec::resize`, one realloc, no new buffer); caller `opencode/convert.pruv:202` passes `move buf`. `empty_buffer` is **P** (DB7 lazy first slab, no hand counterpart). |
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
- `supervisor/core_root.pruv`, `supervise.pruv` and `a1_root.pruv` use their own `supervisor/btail.pruv`, and
  `lib/tail.pruv` is the old library copy; neither is the `pkgs/tail` package, so they were not touched here.

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
