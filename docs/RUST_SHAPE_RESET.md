# Rust-shape reset

User decision, 2026-11-24. This document is the plan of record for replacing Node-shaped Pruv code with code shaped like hand Rust. It overrides the per-row perf-debt work in `docs/L3_RESTRUCTURE_SPEC.md` §7.5 until the reset is done. That work is frozen. The R-queue in §7.4 still records what each piece must keep passing.

## 1. Principle

1. **Shape comes from hand Rust.** Every product module is written the way an experienced Rust programmer writes the same job: typed structs and enums, owned collections where Rust owns, borrowed slices where Rust borrows, `inout` where Rust takes `&mut`, ordinary loops. Pruv then adds proofs on top (requires/ensures, bounds and overflow facts that remove checks).
2. **Node/Electron is the output oracle only.** Tests feed the same input to the TS implementation and to Pruv and compare outputs. The TS code never decides data layout, module boundaries or state shape.
   - Bugs found in TS are fixed in Pruv and recorded as known differences, judged against the spec.
   - Behaviour must match the TS implementation, or improve on it.
3. **Packets live only in test harnesses.** Word packets (`[i64]` frames, `words_array`/`unpack`), step-by-step begin/step wire calls and checksum adapters may exist in `*_test`/`perf` roots that talk to the oracle. Product modules take and return typed values.
4. **Missing language features are fixed, not worked around.** If hand Rust can express something that Pruv cannot, the fix goes into the language, both compilers, `SPEC.md` and the shared library. During the reset this beats the AGENTS.md concept-5 rule "work around now and list the gap".
5. **Structure check before merge.** Each bundle puts the Pruv module next to its hand Rust counterpart and lists, per function, the types, ownership, loops and allocation sites. Any difference needs a written reason: a proof-driven removal, or a gap row.
6. **Performance follows from shape.** Each piece is still measured: time, peak RSS and allocation count at M/L against hand Rust, ≤1.0x. Gains below 1.0x come only from §5.

## 2. Diagnosis (why the reset)

- Fragments copied Node data shapes, and the shapes spread through shared `pkgs/` and `lib/`:
  - a global `HashMap<i64, WireNode>` node store (budget M12)
  - `[i64]` word packets and `Ctx` word sections (`lib/ent_*`, `req/*`)
  - line copy, then `parse_owned` tape, then an owned `str` per field (grok RPC, qwen, codex, opencode)
- Per-row fixing then spent each bundle on one gate cell.
- The classification of the 26 open M/L gate rows (2026-11-24) was: N9 T6 P4 S4 L2 C1. Only 3 of 26 were language or compiler causes; 14 of 26 were representation causes.

## 3. Inventory summary

Inventory delegation `64c782e8` (2026-11-24, readable with `history.read-delegation`) classified 164 Pruv modules against hand Rust:

| Verdict | Count | Meaning |
|---|---:|---|
| N | 35 | Representation differs from hand Rust at the core |
| P | 65 | Partly matches |
| R | 13 | Matches on the compared path |
| ? | 51 | No direct hand counterpart, or scope unconfirmed |

- **Node-heavy (N):**
  - `pkgs/turns`
  - `lib/ent_bound`, `ent_dispatch`, `ent_packet`, `ent_state`, `ent_value`, `entry_api`
  - budget `bud_auto`, `bud_charge`, `bud_cleanup`, `bud_clock`, `bud_halt`, `bud_lifecycle`, `bud_map`, `bud_store`, `bud_tools`, `bud_vlife`, `bud_vwords`, `bud_world`, and the word-packet roots `m4`/`m7`/`m8`/`m9`/`m10`/`m11`/`m12`
  - `codex/read`, `grok/rpc`, `mcp/perf/old_packet`, `qwen/boundary`, `qwen/chain`, `qwen/entry`, `qwen/import`
- **Keep (hand Rust uses the same representation):**
  - the JSON tape (`json/hand/hand_json.rs` `parse_tape`)
  - the MCP result packet (`mcp/perf/hand_packet.rs` `core_hand`)
  - the flat arrays in `perf/d`
  - `pkgs/lex`, `pkgs/bytes`, `codex/folder`, `grok/framing`, `qwen/framing`, `supervisor/packet`

  What goes is building a tape where the consumer does not need one, repackaging internal state at every step, and recopying output prefixes.
- **Unmapped:** 51 `?` modules, mostly grok list/copy/path/summary, opencode, req and lib `ent_*`. These have no hand counterpart yet. Each bundle that touches one writes the hand Rust first, then ports.

## 4. Language gaps

| Id | Gap | Hand Rust form | Blocks |
|---|---|---|---|
| B | Borrowed input stored in a struct or returned (`Doc<'a> { src: &'a [u8], tape }`, `&str` field results) | `json/hand/hand_json.rs` `span`, `grok/perf/hand_rpc.rs` `text_of` | `pkgs/json`, `parse`, `lines`, `access`, `jstr`, `lib/json`, `lib/parse`, `lib/lines`, json roots, `codex/item`, `codex/util`, `grok/util`; design: docs/B_BUNDLE_SPEC.md; B4 done: path liveness (R-6g9-4) lets a view die inside an `if` |
| V | Recursion through collections (`enum Value { Array(Vec<Value>), Object(Vec<(Text, Value)>) }`), order and duplicate keys preserved | `budget/m12_test/hand_life.rs:38` | budget M12, `bud_auto`, `bud_lifecycle`, `bud_vlife`, `bud_vwords`, `ent_value`, `req_value`, part of `qwen/import` |
| U | Exact UTF-16 keys as a first-class view (`&[u16]`) next to UTF-8 `str` | `p01_keys/hand_keys.rs`, M4 `decode_key` | `bud_keys`, `bud_vwords`, `bud_text`, m11, `p01_keys`, `ent_u16`, `ent_path`. P2 (owned and borrowed `[u16]` on both backends) is merged; step 2.9 does not narrow the public `i64` key arrays, which are consumer API (`bench/l3/pkgs/RESHAPE.md` step 2.9) |
| U8 | `str` is UTF-8 in the verifier (step 2.8) | `argv.split_terminator('\0')` (an ASCII byte is a char boundary, no check) | The verifier does not know a `str` is valid UTF-8, so it cannot prove that the byte after a NUL is a char boundary (`invariant_preserved`, counterexample `start = end + 1`). `args::build_args` takes it as a `requires`; `args::arg_spans` keeps its pre-existing run-time boundary check and exports the fact as an `ensures` (one check the hand has not; no new run-time cost vs before step 2.8) |
| ABI | Owned collections (`Vec<Turn>`, typed state) across the host boundary, for the Rust backend and the LLVM backend separately | `qwen/handrust` `window_import` | `qwen/boundary`, `q_root`, `root`, `bud_tlife` |
| S | Empty-string allocation (`string_new` effect) | — | Not shown to block; check during V |
| C | Capacity-keeping clone, and clear without free (G-DB64-4, G-DB64-6) | `Vec::clone_from`, `Vec::clear` | Not shown to block; check during V |
| B-LLVM | Borrowed fields and borrowed results on the LLVM backend (step 2.1, `bench/l3/pkgs/RESHAPE.md`) | `Document<'a> { source: &'a [u8] }`, `fn trim(&str) -> &str` | Borrowed results and locals: closed in bundle B1 (SPEC R-6g9-1, `{ ptr, i64 }` views; `text::trim`, `text::clip_bytes` reshaped). Open: borrowing structs (`pkgs/json` source copy), still `codegen/unsupported_type` on LLVM until bundle B2; design: docs/B_BUNDLE_SPEC.md |
| SR | Push a scalar or a borrowed sub-range into a builtin without a temporary (step 2.1) | `out.push(c)`, `out.push_str(&s[i..j])`, `cmd.arg(&s[i..j])` | No scalar push; `subview` is not accepted as the direct argument of `string_push_str` / `proc_args_push`. `text::one_line` and `args::build_args` left unchanged (no wrapper fn). **Closed by LB7 for the sub-range part** (SPEC R-6g6-3): `string_push_str` / `string_from` take `subview(t, lo, hi)` directly, no temporary String; scalar push and `proc_args_push` stay open. **Closed by LB9** (SPEC R-6g8-1, R-6g8-2): `proc_args_push` takes `subview(t, lo, hi)` directly (`cmd.arg(&s[i..j])`), and `string_push_char(move s, c)` emits `s.push(c)` with no temporary; `text::one_line` / `args::build_args` rewrites are left to the pkgs bundles. Rewritten in step 2.5: `text::one_line` pushes each scalar as `subview(s, i, j)` (`out.push_str(&s[i..j])`); `args::build_args` waits for a pkgs rewrite on the LB9 `proc_args_push`. Rewritten in step 2.8: `args::build_args` pushes `subview(argv, start, end)` after each `find_byte` (`cmd.arg(&argv[start..end])`), no span packet; it states as a `requires` that the byte after a NUL is a char boundary, which the verifier does not know of a `str` (U8, `bench/l3/pkgs/RESHAPE.md` step 2.8); `arg_spans` stays for codex `child_args` |
| VC | Reserve without fill (step 2.1) | `Vec::with_capacity(n)` + `push` | `alloc` fills, `truncate` may shrink-realloc; `names::project_name` left unchanged (two buffers); overlaps LB5. **Closed by LB5** (`arr_reserve`/`arr_push`/`arr_append`, in-place truncate, SPEC R-6g4): no longer a gap. Former VC rows are status **W** (writable, rewrite pending) in `bench/l3/pkgs/RESHAPE.md` step 2.2 |
| FM | Mutable borrow of one of two fields chosen at run time (step 2.1) | `let d = if c { &mut x.a } else { &mut x.b };` | Blocks the decoder pick of a hand-shape `tail` rewrite (`supervisor/hand/core_hand.rs`); `tail` left unchanged. Not closed by LB9: needs the B-bundle borrow/provenance design (a `&mut` whose target field is chosen at run time, so the checker must track a borrow that may alias either field); design: docs/B_BUNDLE_SPEC.md; B4 (path liveness, R-6g9-4), the B5 prerequisite, is done. **Language part closed by B5** (place bindings `let d = inout P`, R-6g9-5); the `tail` rewrite is a package change left to a later pkgs bundle (`bench/l3/pkgs/RESHAPE.md` step 2.9). **`tail` rewritten in step 2.10** (hand `Tail` / `Decoder`, decoder pick by place binding; new per-line gaps CW, BV in `bench/l3/pkgs/RESHAPE.md` step 2.10) |
| SL | String literal bound to a `str` local stays `&'static str` (step 2.1) | `let a = "lit";` | `let a: str = "lit"` emits `String::from`; still in `let none: str = ""` locals of codex, claude, grok (generalises S). **Closed by LB9** (SPEC R-6g8-3): an immutable `str` literal local read only through borrows emits `let x: &'static str = "lit";`; the `alloc` effect is still required |
| TK | Move a buffer field out of an owned struct taken from a match binder (step 2.1) | `Ok(doc) => doc.tape` | In `json/a2_root.pruv`: `let mut d: T = binder;` emits `Clone::clone`, `move binder` of a struct is `type/foreign_move` ("non-collection"), the consuming `move binder.f` is `foreign_move` on a match binder (R-6e-16), and a `take` record parameter is refused when the struct has an `[i64]` field ("take is only for array, collection or aggregate"; `[u8]`-only structs pass). The root keeps `copy(document.tape)`; the Rust backend emits a move anyway, LLVM emits a tape-sized `@pruv_copy`. **Closed by LB9** (SPEC R-6g8-4): in a consuming match, `let t = move binder.f;` on a struct binder is accepted, Rust emits the partial move `let t = binder.f;` and LLVM empties the payload field with no copy; the `json/a2_root.pruv` rewrite is left to the pkgs bundles (`mut` copies and `take` record parameters stay as they were). Rewritten in step 2.8: `json/a2_root.pruv` is `let tape: [i64] = move document.tape;`, Rust `let tape = document.tape;`, LLVM has no `@pruv_copy` (`bench/l3/pkgs/RESHAPE.md` step 2.8) |
| IF | A struct field as an `inout` argument of a builtin (step 2.2b) | `self.data.reserve(n)` | `arr_reserve(inout bk.data, n)` on an `inout bk: Store` is `type/inout_argument` ("an inout argument must be the whole name of a local array or an inout parameter"); `turns::reserve` growth left unchanged (`bench/l3/pkgs/RESHAPE.md` T6). **Closed by LB6** (SPEC R-6g5-2): `arr_reserve`/`arr_push`/`arr_append` accept `inout x.f` on a mutable owner and emit `x.f.reserve_exact(n)` / `x.f.push(v)` in place. Step 2.4 left `turns::reserve` unchanged: the `Store` arena keeps `len(data)` as its capacity (index writes at `bk.end` into zeroed bytes), so a capacity-only `arr_reserve` does not replace the zero-filled growth without the Vec-shaped arena of **ABI / V** (`bench/l3/pkgs/RESHAPE.md` T6) |
| SB | A `str` as the byte source of a bulk copy (step 2.2b) | `dst[at..at+n].copy_from_slice(&s.as_bytes()[..n])` | `copy_range(inout dst, at, src, lo, n)` with `src: str` is `type_mismatch` ("expected [u8], found str"); `turns::put_str`/`write_piece` stay per byte (`bench/l3/pkgs/RESHAPE.md` T2). **Closed by LB6** (SPEC R-6g5-1): `copy_range`/`arr_append` into a `[u8]` take a `str` (or a literal) as the byte source and emit `s.as_bytes()`. Rewritten in step 2.4: `put_str` is one `copy_range` from the `str` (`bench/l3/pkgs/RESHAPE.md` T2, T7) |
| RU | `Result<(), E>` (step 2.2c) | `fn record(..) -> ParseResult<()>` | `Result<(), Fault>` is `type_argument`; `parse::record` returns `Result<i64, Fault>` with `Ok(0)` (`bench/l3/pkgs/RESHAPE.md` P6). **Open (LB6 not done)**: `Result<(), E>` needs a unit expression `()`, a zero-word Ok payload in both LLVM layouts, unit binders and `ok_val` of unit in the verifier, and `?` on it; too wide for the LB6 budget. **Closed by LB8** (SPEC R-6g7-1): `Ok(())` builds and matches a `Result<(), E>` (field-less Ok in the verifier, empty `{}` Ok slot in LLVM) and a bare `f(..)?;` statement propagates it; generated Rust is `Result<(), E>` / `Ok::<(), E>(())` / `Ok(()) =>`. Rewritten in pkgs step 2.7: `parse::record` returns `Result<(), lex::Fault>` (no `Ok(0)`), and every `lex::scan_*` takes `inout pos` and returns `Result<(), Fault>` (`bench/l3/pkgs/RESHAPE.md` step 2.7). Both Rust backends emit the hand form: `f(..)?;` for a unit result and `let x: T = f(..)?;` / `x = f(..)?;` for a value, with the rest of the block at the same level (the checker's `e?` match is recognised in codegen; the error type equals the function's, so `From` is the identity). No codegen difference remains; cost metering keeps the match form (its `return` carries a cost check) |
| OS | `old(x)` of a scalar `inout` parameter in a contract (step 2.2c) | `self.pos` (no ghost) | `old(..)` covers arrays and non-generic struct `inout` (and their field paths), not scalar `inout`; `parse::record`/`container` keep the extra `initial` argument (the cursor at entry) for their `ensures` (`bench/l3/pkgs/RESHAPE.md` P6). **Closed by LB6** (SPEC R-6g5-4): `old(x)` of a whole `inout` `i64`/`bool` parameter; the emitted code is `x: &mut i64` with no ghost. Rewritten in step 2.4: `initial` is gone, the contracts use `old(pos)` (`bench/l3/pkgs/RESHAPE.md` P6; `Ok(0)` stays for RU) |
| SX | Append a fixed list of scalars in one step without a heap temporary (step 2.2c) | `tape.extend_from_slice(&[kind, start, pos, id + 1, parent, 0])` | A stack array cannot be the source of `arr_append` here, and the `alloc(6, 0)` temporary passed straight to `arr_append` is not promoted to the stack (SPEC stack promotion excludes it), so it is a heap `Vec`; `parse::append_record` keeps six `arr_push` (`bench/l3/pkgs/RESHAPE.md` P3). **Closed by LB6** (SPEC R-6g5-5): `arr_push(inout a, v1, .., vk)` (k <= 16) emits `a.extend_from_slice(&[v1, .., vk])`; LLVM grows once. Rewritten in step 2.4: one `arr_push` of six values (`bench/l3/pkgs/RESHAPE.md` P3) |
| MC | Byte search in a `[u8]` range (step 2.2c) | `memchr` inside `BufRead::read_line`, `slice.iter().position(..)` | No byte-search builtin (std has only `index_of_i64` over a whole `[i64]`); `transport::receive_grow` scans for the newline byte by byte (`bench/l3/pkgs/RESHAPE.md` X5). **Closed by LB6** (SPEC R-6g5-3): `find_byte(a, b, lo, hi)` over `[u8]` or `str` emits `a[lo..hi].iter().position(..)`; LLVM calls `memchr`. Rewritten in step 2.4: the newline scan is one `find_byte` (`bench/l3/pkgs/RESHAPE.md` X5) |
| ZP | Bulk copy of a prefix of an `[i64]` that is not the whole array (step 2.3, open debt G-DB21-1) | `tape[..m].to_vec()` | `copy(subview(tape, 0, m))` and `arr_reserve(inout exact, m)` + `arr_append(inout exact, tape, 0, m)` both fail verification with "z3 error 6: invalid pattern"; `arr_append(inout exact, subview(tape, 0, m))` is a `view_context` type error. `lines::build_span` keeps a zeroed `alloc(m, 0)` and an element loop (`bench/l3/pkgs/RESHAPE.md` L5). **Closed by LB7** (SPEC R-6g6-2): the verifier names each array lambda inside an aggregate trigger, so both forms verify and emit `tape[..m].to_vec()` / `extend_from_slice`. Rewritten in step 2.5: `lines::build_span` copies the tape prefix with `copy(subview(tape, 0, m))` (`bench/l3/pkgs/RESHAPE.md` L5) |

SPEC.md §6e currently restricts recursive types (`SPEC.md:744`). V lifts that restriction for `Vec` and `HashMap` payloads.

Refinements from the debate on this plan (delegation `0d3535db`, 2026-11-24):

- **B needs provenance and exclusivity, not one lifetime.** Grok `text_of` (`hand_rpc.rs:500`) returns a borrow of either the input or a reused scratch buffer. Today a `subview` lives only for one call (`SPEC.md` R-6g-6, R-6g-8), and it cannot be bound to a local (G-DB63-1). Before B is designed, fix these cases as tests:
  - a JSON doc borrowing one input
  - a Grok result that may borrow either of two sources
  - rejection, in both compilers, of: a local escaping, reusing or reallocating the buffer while a result borrows it, and moving or freeing the owner
- **V needs a real size measure.** Both verifiers give collection fields a size weight of 0 (`pruvc/verify.py` `weight`, `pruv-proof/src/model.rs:958`), so `Array([Null])` and `Null` have the same size. Adding "a child is smaller" as an axiom would contradict that. V defines a measure over finite owned trees that counts the actual Vec elements and present map values, and proves a strict decrease for the chosen child after construction, replacement and move. A decoder may use the remaining input length instead. A call on the same value is rejected in both compilers.
- **V is four blockers, each with its own deliverable:**
  1. the recursive type declaration, starting monomorphic (`Value`, `Field`), with generics opened only where needed
  2. mutation: last-match lookup and in-place child replace, keeping order and duplicates (`hand_life.rs:113`); a `HashMap` replacement would change behaviour
  3. LLVM internal layout: create, overwrite, move and free exactly once
  4. host ABI: allocator, which side frees, ownership on failure
- **V checks deep and wide values.** Traversal, mutation and drop are measured on deep and narrow values and on wide values, for both run-time stack use and proof time. The plan then sets an input depth policy (`SPEC.md:1113`).
- **U is split.** Owned UTF-16 storage (`Vec<u16>` keys owned by M12 objects, `hand_life.rs:6`) is part of V. Borrowed `&[u16]` lookup views remain U.
- **G-DB63-2.** A string literal passed as a `str` argument makes the effect checker infer `alloc`, although generated Rust passes `&'static str`. This goes in the B bundle.

## 5. Beyond hand Rust: where Pruv can win

Matching hand Rust gives parity. Going below 1.0x needs something that safe hand Rust cannot do. These are the sources, in expected order of payoff:

1. **Proven bounds and overflow.** Safe Rust keeps `[]` bounds checks and `checked_*`/panic paths that LLVM cannot remove. Pruv proves them and emits `get_unchecked` and wrapping ops. Biggest in tape access chains, ring/tail spans and per-unit decoding loops.
2. **Proven validity removes re-validation.** Once a parse proves the input is valid UTF-8 and well-formed JSON, later stages skip `from_utf8`, `unwrap` and re-checks that hand Rust repeats at module boundaries.
3. **Exact sizes from contracts.** When `ensures` gives the output length, Pruv reserves once and writes without growth checks. This saves allocations and memory, not just time.
4. **No panic and unwind paths.** Code without reachable panics is smaller and inlines better.
5. **Effects in signatures.** A pure function is known pure, so calls can be hoisted, merged or memoised without alias analysis.

Expected payoff after the debate (`0d3535db`):

- Item 1 is the most promising, mainly in data-dependent tape indexing (`hand_json.rs:280` `record()`). Plain sequential loops may have no bounds branch left after LLVM.
- Items 2 and 3 are likely parity restores. Hand Rust already returns the tape directly and precomputes key sizes. `ensures` does not emit `reserve` today, and the proof model does not track capacity. Count only the scans and reallocations left after safe Rust uses the same reservation strategy.
- Items 4 and 5 are left out of performance budgets until measured. An earlier guard-versus-direct-call test differed by under 0.1 ns.
  - Calls without `!` can still write `inout` arguments. Moving or merging them needs read/write and alias evidence, not purity alone.

Each claimed win must show an assembly or instruction diff naming the check that survives in hand Rust and is gone in Pruv (`docs/L3_RESTRUCTURE_SPEC.md` §4.2). A hand Rust author can write the same `unsafe` code; the Pruv claim is that it is proven safe, so the comparison stays against safe hand Rust plus a labelled unsafe variant.

## 6. Order

1. **Language bundles, both compilers, one at a time.** They all touch the type checker and verifiers, so they do not run in parallel with each other. Fragment work that does not touch the compilers may run beside them.
   - **LB1 (V1, V2):** the recursive owned value: a monomorphic declaration, a size measure, last-match lookup and in-place child replace, owned UTF-16 keys, on the Rust backend. This comes first because budget M12 and `ent_value` depend on it, and DB64 showed it is the main cause there.
   - **LB2 (B, G-DB63-2):** borrowed doc and text fields with provenance and exclusivity, starting from the test cases in §4.
   - **LB3 (V3, V4, ABI):** the LLVM layout of recursive and owned collections, then owned collections across the host boundary.
   - **LB3b (layout follow-up):** overlapped enum payloads, `realloc` growth, native scalar `Vec` fields, the
     checked-mode length bound and budget charge in native push, and borrowed views on LLVM (debt rows D-LB3-1..3,
     docs/L3_LB3.md Open items).
   - **LB5 (buffer growth gaps from MCP):** non-filling growth (`reserve`/append, or `resize` without fill when the
     tail is proved written before read), in-place `x = truncate(x, m)`, `reserve_exact` before `resize`, and `str`
     lent as read-only `[u8]` (debt rows D-MCP-1..2, bench/l3/mcp/RESHAPE.md).
   - **LB4 (U):** borrowed UTF-16 views, if still needed after LB1.

   Each bundle adds tests in `tests/` and updates `SPEC.md`.
2. **Shared packages, in dependency order:**
   1. `json`, `lex`, `bytes`, `text`, `names`, `args`, `tail`
   2. `jstr`, `parse`, `turns`, `transport`
   3. `access`, `lines`
   4. `time`

   The old `lib/` JSON copies are removed, not ported; consumers move to `pkgs/`.
3. **Entry/req shared lib, in order:**
   1. `ent_u16`
   2. `ent_path`, `ent_packet`
   3. `ent_value`, `ent_state`
   4. `ent_accounts`, `ent_dispatch`
   5. `ent_bound`, `ent_canon`
   6. `ent_sources`
   7. `entry_api`
   8. `entry/root`, then `req`
4. **Fragments.**
   1. MCP stdio check first: it is the first product candidate, and its `R` modules make it the smallest port.
   2. Supervisor.
   3. Grok RPC and history. The RPC line path can start before B lands: original arg plus start/end positions plus an `inout` buffer (`grok/rpc.pruv:138` against `grok/perf/hand_rpc.rs:575`).
   4. Qwen, Codex, OpenCode.
   5. Entry/req.
   6. Budget, starting from the M12 value model once V lands.
Order changes from the debate (`0d3535db`):

- **Independent paths start without waiting for all of B and V.** These are `lex` and `bytes`, the MCP output path (existing `inout`) and the Grok RPC line path.
- **Entry modules that call each other move together, one call path at a time.** `ent_packet` ↔ `ent_value` and `ent_state` ↔ `ent_dispatch` call each other. `ent_wire` (called by `ent_packet`) joins step 3.2. Before that, the plan fixes the boundary between types and external input validation.
- **The P10H experiment runs early.** It covers the cost of about 420k host calls and reading the latest value at lookup time. Porting `req` waits for its result.
- **Claude R08 is part of step 4.** It sits with Qwen/Codex/OpenCode, and the `lib/` consumers' move to `pkgs/` is listed in each bundle.
- **Measuring starts with the first product path.** When a shared ownership boundary changes, one representative consumer is measured at M/L right away. Step 5 below then closes the full table.

5. **Bulk remeasure.** All pieces, all M/L cells, time, RSS and allocation count against hand Rust. §7.5 rows are closed in bulk. Cells still over 1.0x are grouped by cause (shape, language gap, codegen) and fixed once in shared code.

Each bundle keeps to AGENTS.md: at most 2 subs at a time, 2 hours per bundle, disjoint files.

**Scheduling.** The 51 `?` modules make module counts a poor estimate; the older estimate was 80–120 sub-steps for the non-budget fragments (`docs/L3_RESTRUCTURE_SPEC.md:511`). Instead, each product operation gets an agreed hand Rust baseline with the same meaning first. The schedule is estimated from those operations.

## 7. Done criteria per piece

**Order (user decision, 2026-11-24): structure first, measurement last.** The premise of the reset is that Pruv is written the same as the hand Rust. So:

1. A piece is not finished while its structure table has any difference from the hand Rust (types, ownership, borrows, loops, allocation and copy sites, function by function). The only allowed difference is a proof that removes a check the hand Rust keeps.
2. If Pruv cannot express what the hand Rust does, the piece stops there. The missing feature is added to the language (a language bundle) before the piece goes on. No near-shape workaround, no "debt row" for a shape difference.
3. Performance is measured only after the table shows zero differences, once, as a confirmation. There is no measure-fix-measure loop. If a structurally identical piece is still over 1.0x, the cause is in the compiler's code generation; it is shown line by line and fixed in the compiler.

- Oracle match: 0 unexplained differences against the TS implementation on the existing differential suites, with fixed TS bugs listed as known differences.
- The structure table from §1.5 is filled in, with no unexplained shape difference.
- Time, peak RSS and allocation count ≤1.0x hand Rust at M and L, measured with interleaved runs on the same machine.
- No word packets or step wire in product modules; harness-only adapters are named `*_test` or live under `perf/`.
- Both compilers, both backends where the piece already ran on both.

Tightened after the debate (`0d3535db`):

- **What the structure table covers.** It follows the real call path from the product entry point to the owner of the result. It lists field accesses, bytes copied and allocation sites, not type names.
  - A named struct that still wraps word arrays fails, for example `pkgs/turns` `Collected` with its 200×8 slots.
  - The allowed JSON tape and MCP packet state their use and their boundary.
  - An open gap row is not a pass.
- **Measure the complete operation.** The hand lane must build the same result. A checksum-only hand lane like `hand_rpc.rs:600` does not count. Cost moved outside the adapter must still show up.
- **Oracle semantics.** Where a hand lane is known to diverge (for example escaped object keys, or the `error.code` scope in `hand_rpc.rs:4`), compare the output bytes and errors of TS, hand Rust and Pruv on that counterexample. Each intended TS fix cites the spec for its expected value. The R12 four compiler/backend pairs, existing fixtures, rejection of bad external input and effect order are linked from the done table.
- **Measurement protocol.**
  - Fixed per piece: M/L inputs, build flags, the measured span and the repeat count.
  - Peak RSS is taken from a fresh process. Language-level allocation requests are reported separately from system `malloc` calls, since the LLVM cache makes them differ.
  - Where hand Rust allocates 0 times, the comparison uses absolute counts.
  - A paired result whose uncertainty crosses 1.0 is "undecided", not pass.
  - Fixed RSS cost is reported separately from the proportional part, as in AGENTS.md perf rule 3.

## 8. Existing work

- DB63 (grok RPC) and DB64 (budget M12 slab) were stopped unmerged. Their worktrees are removed and the branches kept as reference for the grok and budget ports:
  - `lair/db63-brief-grok-rpc-lines-5778` has no commits of its own. Its uncommitted diff (grok `rpc.pruv`, `perf/hand_rpc.rs`, `perf/perf_root.pruv`, `perf/run_perf.py`) and notes are kept in the local work records, not in git.
  - `lair/db64-brief-budget-m12-typed-fcff` ends at `998a5f60`. Its perf gate failed on 18 of 74 cells (RSS 1.38x at M, 1.56x at L). The causes support this plan: a 96-byte `WireNode` because there is no value enum (G-DB64-3, gap V), `encode_key`/`words_array` allocations (G-DB64-7), `bud_vwords` needing an `inout` Vec API (G-DB64-8), and a node built per field in the R cell (G-DB64-9).
- L3-1..L3-10 test suites, fixtures and hand Rust lanes stay. They are the oracle and baseline harness for the reset.

## 9. Toward replacing Lair

The reset applies to every later migration step (BLUEPRINT L track). L3 backend pieces follow §6. L4, the screen, is built on Rust UI tools (GPUI, Slint, iced) in Rust shape from the start; Electron is the behaviour oracle there too.
