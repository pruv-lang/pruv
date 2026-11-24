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

Same rule and columns as step 2.1. Hand counterparts: `json/hand/hand_json.rs` (`parse`, `Parser`, `units`, query
string output) for `parse` and `jstr`; the `Collector` in `qwen/handrust/src/lib.rs:158`, `claude/handrust/src/lib.rs:618`
and `grok/perf/hand_history.rs:705` for `turns`; `supervisor/hand/supervise_hand.rs` `read_pipe` (64 KiB chunk,
`buf[..n]`) and `codex/hand/reader.rs` `Rpc` (`BufReader::read_line`) for `transport`. No performance was measured.

## Status

| Package | Differences left | Measured |
|---|---|---|
| jstr | 4 (VC unit buffer, VC count-then-fill, copy-to-`String` variants, SL/S empty string) | no |
| parse | 5 (iterative scan vs recursive descent, VC tape growth, B-LLVM source copy, D32 publication checks, string block skip) | no |
| turns | whole package (ABI/V: owned `Vec<Turn>` collector) | no |
| transport | 4 (word-packet state shared with mcp, VC line growth, per-byte framing, tail FM) | no |

## jstr (`jstr/jstr.pruv`)

| Pruv | Hand Rust | Types | Ownership | Loops | Allocations | Difference and reason |
|---|---|---|---|---|---|---|
| `hex`, `step`, `scalar`, `width` | `hex4`, per-escape decode in `unescape`, `char::len_utf8` | `[u8]`, `i64` | borrow | one escape or scalar | none | = |
| `units_equal(source,s,e,key,klen)` | `units(raw).eq(key.iter().copied())` | `[u8]`, `[i64]` | borrow | one pass, early exit | none | = |
| `ascii_equal(source,s,e,key)` | query mode 3: `text.is_ascii() && !body.contains('\\')` then `body == text` | `[u8]`, `str` | borrow | one pass | none | = |
| `string_units(source,s,e) -> [i64]` | `units(raw).collect::<Vec<u16>>()` (`with_capacity` + push) | `[u8] -> [i64]` | owned result | decode pass, then copy pass | Pruv 2 (`alloc(e-s)` then exact copy); hand 1 | **VC**: no reserve-without-fill, so the exact-length result is a second buffer. Unchanged. |
| `string_utf8(source,s,e) -> [u8]` | `unescape(inner, &mut Vec<u8>)` (push into a reserved `Vec`) | `[u8] -> [u8]` | owned result | Pruv: count pass, then fill pass; hand: one push pass | 1 each | **changed**: all-plain branch was a per-byte loop, now one `copy_range` (`copy_from_slice`), as the hand `extend_from_slice` of a plain body. The count-then-fill shape stays (**VC**). |
| `string_utf8_plain`, `string_utf8_scanned` | same as above, plain body `to_vec()` | `[u8] -> [u8]` | owned result | as above | 1 | = (plain) / VC (scanned) |
| `string_text`, `string_text_scanned`, `string_text_plain` | `String::from_utf8(vec)` (move) | `[u8] -> Text` | **copies** through `utf8_to_string` | — | Pruv 2 (bytes, then `String`); hand 1 | Differs. The `*_owned` variants are the hand shape (`str_from_utf8` move). The copy variants stay until consumers move to `*_owned` (consumer steps, not this package). |
| `string_text_owned`, `*_scanned_owned`, `*_plain_owned`, `owned_text(take data)` | `String::from_utf8(vec)` | `[u8] -> str` | move | validate pass | 1 | = except the unreachable fallback `let none: str = ""; copy(none)` (**SL/S**: an empty `String` the hand never builds). Unchanged. |
| `Text::InternalInvariant` | `unreachable!()` / `unwrap()` | enum | — | — | none | Kept: the verifier cannot yet show the branch is dead; same effect class as a hand panic. |

## parse (`parse/parse.pruv`)

| Pruv | Hand Rust | Types | Ownership | Loops | Allocations | Difference and reason |
|---|---|---|---|---|---|---|
| `parse(input)` error path | `hand_json.rs:243`: on a grammar error (kind != 4) `from_utf8(&input[position..])` | `[u8]` | borrow | one rescan of the rest | none | **changed**: was a full rescan `validate_utf8(input, 0, n)` on every failure; now only `kind != 4` failures rescan, from the failure position, as the hand does (D33: bytes before the failure are already valid). = now. |
| `scan_document` (state machine + `[i64]` frame stack, 4 slots per frame) | `Parser::record` recursive descent, `container`/`string`/`number`/`literal` | `[u8] -> JsonParse` | borrow input | Pruv one flat loop with explicit stack; hand recursion | tape only | **Differs** (loop shape). Not rewritten in this step: the hand `Parser<'a> { input: &'a [u8], .. }` is B-LLVM, and a recursive rewrite with `inout` position/tape parameters has not been tried yet (open item, no gap id claimed). |
| `widened`, `regrown` | `tape.reserve(WIDTH)` + `extend_from_slice` (realloc growth) | `[i64]` | owned | copy loop | new buffer + copy | **VC** (realloc growth without fill, LB5). Unchanged. |
| `Parsed(JsonDocument{ source: copy(input), .. })` | `Document<'a>{ source: &'a [u8] }` | `[u8]` | **copy** | — | +1 | **B-LLVM** (same row as json step 2.1). Unchanged. |
| `publication_ok` (`json::check_subtrees`) | none | `[i64]` | borrow | per record | none | D32 proof-only check. Still needed: `scan_document` does not yet prove the tape shape that `check_subtrees` checks. Unchanged. |
| `string_token` | `Parser::string` per byte | `[u8]` | borrow | 16-byte block skip then per byte | none | Loop difference, same as `lex::scan_string` (step 2.1). Unchanged. |
| `write_record`, `close_record`, `first_slots`, `token_at`, `scan`, `scan_into` | `Parser::record` writes, `output(32*WIDTH)` | `[i64]` | `inout` (= `&mut Vec`) | — | none | = |

## turns (`turns/turns.pruv`)

| Pruv | Hand Rust | Types | Ownership | Loops | Allocations | Difference and reason |
|---|---|---|---|---|---|---|
| whole package: `st[4]`, `sl[200*8]` word packets, `buf` 65584 bytes per slot, `Collected{turns:[i64],text:[u8],..}`, `begin*`/`answer*`/`finish*`/`*_ref` variants | `Collector { turns: VecDeque<Turn> }`, `Turn { prompt: String, reply: String, created_at: Option<String>, .. }`, `begin`/`skip_user`/`answer`/`tool`/`finish -> Vec<Turn>` | packets vs `Vec<struct>` | packets vs owned structs | — | slabs vs per-turn `String` | **ABI / V**: an owned `Vec<Turn>` (struct with `String` and `Option<String>` fields) cannot be built and returned across the host boundary on both backends, and `VecDeque` push/pop of such structs has no Pruv form. Whole package unchanged; the `Collected` 200×8 slot table is the §7 example. |

## transport (`transport/transport.pruv`)

| Pruv | Hand Rust | Types | Ownership | Loops | Allocations | Difference and reason |
|---|---|---|---|---|---|---|
| `live`, `stdin_closed`, `send`, `close`, `poll`, `expired` | `child.stdin.write_all` with deadline, `child.kill`/`wait` | `Loop` moved through | move (= owned handle) | `expired`: at most 8 polls | none | = (event-loop shape; the hand programs use threads, the same operations per call) |
| `receive_grow` chunk read | `read_pipe`: `pipe.read(&mut buf)` into `[0; 65536]`, `buf[..n]` | `[u8]` | `inout chunk` (= `&mut [u8; 65536]`) | copy | none | **changed**: per-byte copy loop is now one `copy_range` (`copy_from_slice`). = now. |
| `receive_grow` framing | `BufReader::read_line`: `memchr` then `extend_from_slice` | `[u8]` | `inout line` | Pruv per byte; hand memchr + bulk copy | growth only | **Differs** (loop shape). A bulk copy needs a growable line with spare capacity (VC); left unchanged. |
| `grow_line(line, cap)` | `line.reserve(1)` (realloc, amortised doubling) | `[u8]` | owned | — | new buffer + `copy_range` | **VC** (realloc growth without fill, LB5). Unchanged. |
| `st: [i64]` 5-slot state | `struct Reader { status, partial, offset, exit, len }` | packet vs struct | `inout` | — | none | Differs. The packet is the API of `bench/l3/mcp` (frozen in this step, LB5 is editing it) and codex/grok; changing it is a consumer step. |
| stderr `tail_push(inout tb, inout ts, ..)` | `tail.push(&bytes)` on a ring struct | packets | `inout` | — | none | **FM** (step 2.1 `tail` row). Unchanged. |

## Gap rows found in step 2.2

No new ids. Rows reused from §4:

| Id | Hand Rust form | What Pruv cannot express | Functions left unchanged |
|---|---|---|---|
| VC | `Vec::with_capacity` + push; `Vec::reserve` realloc growth | no reserve-without-fill, growth is a new buffer + copy | `jstr::string_units`, `jstr::string_utf8` count pass, `parse::widened`/`regrown`, `transport::grow_line` and the per-byte framing |
| B-LLVM | `Parser<'a> { input: &'a [u8] }`, `Document<'a>` | borrowed fields on LLVM | `parse` source copy |
| ABI / V | `Collector { turns: VecDeque<Turn> }` returning `Vec<Turn>` | owned struct collections across the host boundary | `turns` (whole package) |
| SL / S | `""` literal never allocated | `let none: str = ""` emits `String::from` | `jstr::owned_text` fallback |
| FM | ring tail picked by stream | `inout` binding to a run-time chosen field | `transport` stderr tail (via `tail`) |
