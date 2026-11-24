# B bundle: borrowed data that outlives one call

Design step for gap rows **B**, **B-LLVM**, **FM** and the rest of **G-DB63-2** in `docs/RUST_SHAPE_RESET.md` §4.
Design only: no compiler change is made by this document. Every rule here is checked by both compilers (`pruvc`,
`pruvc-rs`) with the same diagnostic code, and every accept test gives byte-identical Rust from both.

## 0. Where we start

LB2 (`docs/L3_LB2.md`, SPEC R-6g-11, R-6g-12) already closed the *language* part of B on the Rust backend:

- `ref [u8]` / `ref str` views, borrowing structs (`struct Doc { src: ref [u8], tape: [i64] }` emits
  `struct Doc<'a>`), and `-> T from(p1, .., pn)` results;
- provenance (a set of root variables, each shared or exclusive), block-level live ranges, `type/borrow_conflict`,
  `type/borrow_escape`, `type/borrow_signature`;
- a string literal is a static view with no provenance; a literal argument of a read `str` parameter needs no `alloc`.

So the B row still lists blocked files for four main reasons:

1. **LLVM.** Every consumer piece also builds on LLVM, and the LLVM backend rejects any `ref` type or borrowing
   struct (`pruvc/llvmgen.py:6873`, `codegen/unsupported_type`). A rewrite that uses R-6g-11 would break the LLVM
   build of the piece, so none was made. This is B-LLVM (debt D-LB3) and it is what blocks most rows.
2. **Liveness is block-level.** A view is live through the whole `if` / loop statement that mentions it. Hand Rust
   relies on NLL: in `supervisor/hand/core_hand.rs:61-75` the `decoder` borrow ends inside the `if` before
   `self.append(..)`. The block rule rejects that.
3. **No mutable place binding** (FM): `let decoder = &mut self.decoders[stream];` (`core_hand.rs:61`) and the
   RESHAPE form `let d = if s == 1 { &mut tl.d1 } else { &mut tl.d2 };`.
4. **No mutable borrowing struct**: `let mut` / `inout` of a borrowing struct is `type/borrow_signature`, so the
   jstr cursor `struct Units<'a> { raw: &'a [u8], pos: usize }` (RESHAPE J1) cannot be written.

Plus the G-DB63-2 residue: R-6g8-3 emits `let x: &'static str = "lit";` but still demands `! alloc`.

Plus the record accessors of the hand document (`hand_json.rs:282-300`): `record(&self) -> &[i64]` lends an owned
field and `value(&self) -> Result<&[i64], i64>` puts a view in a `Result` (B7, B8; added after the debate, §12).

What hand Rust does **not** need, and this bundle therefore does not add: a borrowing *struct* inside an enum,
`Option` or `Result` payload. The hand parser returns the owned tape, `fn parse(input: &[u8]) ->
ParseResult<Vec<i64>>` (`json/hand/hand_json.rs:244`), and the caller builds `Document { source: input, tape }`.
Borrowing-struct payloads and `Vec<&[u8]>` stay open (§11) until a hand form that needs them is found.

## 1. Survey: what other languages do with stored borrows

| Language | Stored / returned borrows | Taken here |
|---|---|---|
| Rust | Named lifetimes on structs, elision for the common function cases; NLL makes a borrow live only where it may still be used, along control-flow paths, not to the end of the lexical scope ([RFC 2094](https://rust-lang.github.io/rfcs/2094-nll.html), [elision RFC 141](https://rust-lang.github.io/rfcs/0141-lifetime-elision.html)). | The target shape (`Doc<'a>`, `&'a mut`), and NLL-style path liveness (§4.2). Not the lifetime syntax. |
| Swift | `~Escapable` values that may not outlive what they depend on ([SE-0446 review](https://forums.swift.org/t/se-0446-nonescapable-types/74666), [acceptance](https://forums.swift.org/t/accepted-with-modifications-se-0446-nonescapable-types/75504)); returning such values is a separate *lifetime dependency* annotation naming the source parameter ([pitch](https://forums.swift.org/t/pitch-non-escapable-types-and-lifetime-dependency/69865), [pitch 2](https://forums.swift.org/t/pitch-2-lifetime-dependencies-for-non-escapable-values/78821), [summary](https://mjtsai.com/blog/2025/03/19/lifetime-dependencies-in-swift-6-2-and-beyond)). | Confirms LB2's choice: a dependency list on the result (`from(p)`, Swift `@lifetime(p)`) instead of named lifetimes; a dependent value is a type property (borrowing struct), not a per-variable one. |
| Verus | Mutable references as arguments first, then returned `&mut` with specs, "partially supported" ([guide: returning mutable borrows](https://verus-lang.github.io/verus/guide/mutable-references.html), [features](https://verus-lang.github.io/verus/guide/features.html), [design discussion #35](https://github.com/verus-lang/verus/discussions/35)); stored `&mut` use a `has_resolved` assumption at the last mutation ([TreeMap chapter](https://verus-lang.github.io/verus/guide/container_bst_mut_refs.html)). | Ordering: shared borrows everywhere first, exclusive borrows only where the encoding stays simple (place bindings, §5), no stored `&mut`. |
| Creusot / RustHorn | `&mut T` = (current, prophecy `^x`); when the borrow ends, assume current == prophecy ([Creusot: mutable borrows](https://creusot-rs.github.io/creusot/guide/representation_of_types/mutable_borrows.html), [RustHornBelt](https://people.mpi-sws.org/~dreyer/papers/rusthornbelt/paper.pdf)). | Not taken. A prophecy is needed when a `&mut` escapes into data or a result. A Pruv place binding never escapes and its root is frozen while it lives, so plain write-back at the end of the live range is exact (§5.3) and z3 sees no new symbol kinds. |
| Dafny | No borrows; arrays are heap references, aliasing is handled by `reads` / `modifies` frames and `fresh` ([Dafny paper](https://arxiv.org/pdf/1412.4395), [tutorial](https://arxiv.org/pdf/1701.04481)). | Not taken: a heap model would put frame conditions on every Pruv function. Value semantics + provenance keeps the proofs as they are. |
| Hylo | Mutable value semantics; `inout` bindings and `inout` subscripts *project* part of a value, one binding has access at a time; `longer_of` yields one of two `inout` arguments ([subscripts](https://docs.hylo-lang.org/language-tour/subscripts), [bindings](https://docs.hylo-lang.org/language-tour/bindings), [projections discussion](https://github.com/orgs/hylo-lang/discussions/1868)). | The FM form: an `inout` binding to a projected part of a value, chosen at run time, with exclusive access, which is Pruv's `inout` semantics extended from arguments to locals. |
| Vale | Immutable region borrowing: data frozen for a scope is read through raw references, no checks ([part 1](https://vale.dev/blog/zero-cost-borrowing-regions-part-1-immutable-borrowing), [overview](https://vale.dev/blog/zero-cost-borrowing-regions-overview)). | The LLVM lowering argument (§7): a frozen root makes a raw `{ ptr, len }` view safe with no run-time check. |

## 2. Surface syntax

The smallest Rust-like change; **no named lifetimes**. LB2 already infers every lifetime from `from` lists and
provenance, the Rust codegen invents `'a`, and every hand form in the B/FM rows has one lifetime per function
(Rust elision covers all of them except `text_of`, whose `'a` ties two parameters, which `from(src, buf)` already
says). Named lifetimes would add syntax that the checker would have to compare with what it infers anyway; Swift
reached the same conclusion with dependency lists on results.

New forms (everything else is LB2):

```
// FM: an exclusive place binding (Rust `let d = &mut P;`)
let d = inout tl.decoders[s];
let d = inout if s == 1 { tl.d1 } else { tl.d2 };

// cursor: a mutable borrowing struct (Rust `let mut u = Units { raw, pos: 0 };`, `fn next(u: &mut Units<'_>)`)
let mut u: Units = Units { raw: subview(src, lo, hi), pos: 0 };
fn next_unit(inout u: Units) -> i64 { .. }

// field lending (Rust `fn record(&self, id) -> &[i64]`): the result borrows the parameter's owned field
fn record(d: JsonDocument, id: i64) -> ref [i64] from(d.tape) { .. }

// a view in a Result payload (Rust `fn value(&self, id) -> Result<&[i64], i64>`)
fn value(d: JsonDocument, id: i64) -> Result<ref [i64]> from(d.tape) { .. }
```

- `ref [i64]` (and `ref [bool]`) join `ref [u8]` / `ref str`: the element type does not change the rules.
- `from(p.f..)` names an owned field path of a read parameter; the result borrows that parameter's storage (Rust
  `&'b Doc<'_>` with the result `&'b`), not what `p` borrows. `from(p)` keeps its LB2 meaning.

- `inout` is already the Pruv word for `&mut` at call sites (`f(inout x.f)`); `let d = inout P;` is the same marker
  on a binding. No `&`, no `*`: `d.f = e` writes the field, `d = e` writes the whole place, `d` is passed as
  `inout d` or read. This is how `inout` parameters already behave.
- Place grammar `P ::= x | P.f | P[i] | if c { P } else { P }`. `x` is a `let mut` local or an `inout` parameter
  (R-6g-3 roots). `P[i]` is an element of a `Vec<T>` of structs or of a scalar array. The `if` form needs an `else`
  and both arms of the same type.

## 3. SPEC rules to add (new section §6g9, "Borrows past one call (B bundle)")

| Rule | Content | Sub-bundle |
|---|---|---|
| R-6g9-1 | LLVM lowering of `ref [u8]` / `ref str` locals, arguments and `from` results (layout §7). Replaces the LLVM sentence of R-6g-11 and SPEC line 1161. | B1 |
| R-6g9-2 | LLVM lowering of borrowing structs (fields, read parameters, results through `ptr %out`). Borrowing structs may carry `invariant`s; an invariant may mention `ref` fields. | B2 |
| R-6g9-3 | Static text locals need no `alloc`: an R-6g8-3 local (`let x: str = "lit";`, read only through borrows) is a static view; `effect/missing_effect` no longer fires for it. A literal in any owned position still allocates (R-6g-12 unchanged). The empty array literal `[]` in a `ref [T]` position (a `from` result, a `ref` field, a `ref` let) is a static empty view with no provenance (hand `return &[];` in `text_of`). | B3 |
| R-6g9-4 | Path liveness. A borrow (view, borrowing struct, place binding) is live at a program point iff some path from that point reaches a mention of it before the binding goes out of scope. Replaces the block rule of R-6g-11 ("a loop or `if` counts whole"). It accepts strictly more programs; every R-6g-11 reject test still rejects. | B4 |
| R-6g9-5 | Place bindings `let d = inout P;` (§5). | B5 |
| R-6g9-6 | Mutable borrowing structs: `let mut` and `inout` parameters of a borrowing struct type (§6). | B6 |
| R-6g9-7 | `ref [i64]` / `ref [bool]`, and field lending `from(p.f..)` of an owned field of a read parameter. | B7 |
| R-6g9-8 | A view (`ref [T]` / `ref str`, not a borrowing struct) as an `Option` / `Result` payload of a `from` result or an immutable `let`; Rust `Result<&'b [i64], i64>`, LLVM tag + `{ ptr, i64 }`. | B8 |

Rules stay as rule ids, as in R-6g-11; no new error code except `type/place_binding` (R-6g9-5, an invalid place).

## 4. Checker

### 4.1 Provenance and aliasing (unchanged core)

Provenance stays LB2's: a set of roots, each shared or exclusive. New sources:

- `let d = inout P;` gives `d` an exclusive *footprint*: the longest common field path of the places in `P`
  (`x.decoders` for `x.decoders[i]`, `x` for `if c { x.a } else { x.b }`, `x.t` for `if c { x.t.a } else { x.t.b }`).
  Both arms of an `if` must have the same root variable (different roots is `type/place_binding`). Conflicts use the
  R-6g-3 path-overlap test: while `d` is live, a mention of a path that overlaps the footprint is a conflict, a
  disjoint path (`x.bytes` beside `x.decoders`) is not. This is what rustc does for `&mut self.decoders[s]` and
  for `if c { &mut x.a } else { &mut x.b }` (a later use of `x.b` is rejected because `d` may be `x.b`). The
  verifier writes back the footprint only (§5.3). The index of `P[i]` is evaluated once into a snapshot, so `i`
  itself is not frozen.
- A `let mut` borrowing struct has the union of the provenances of every value assigned to it (one flow-insensitive
  set per variable; the borrow is still live by R-6g9-4).
- An `inout` borrowing-struct parameter has the provenance of the caller's argument (the caller treats the argument
  as an exclusive root, as for any `inout`), and inside the callee its `ref` fields may only be assigned values whose
  provenance is that parameter (subviews of its own `ref` fields, or static literals): Rust `&mut Units<'a>` can
  only store `&'a` data it already has. Anything else is `type/borrow_escape`.

Conflicts are LB2's: while a borrow is live, a shared root is read-only and an exclusive root is untouchable
(`type/borrow_conflict`). A place binding is an exclusive borrow of its footprint, so while `d` is live no path
overlapping the footprint may be read, written, passed, moved or matched, including in a loop `invariant`,
`decreases` or `assert` (a contract that mentions the footprint is a mention); `d` itself may be used freely.

### 4.2 Path liveness (R-6g9-4)

Backward gen/kill liveness over the function body's structured control flow (`if`, `while`, `for`, `match`,
`break`, `continue`, `return`). State: a bit set of the borrow variables live after the current statement. A mention
generates; a `let` and a whole assignment of a `let mut` borrowing struct (B6) kill. `return` starts from the
variables the result mentions; `continue` / `break` take the set at the loop head / after the loop. Loops are solved
by whole-graph sweeps in reverse statement order until no set changes, not by iterating each loop on its own: with
gen/kill bit sets each node's set only grows, so at most (loop depth + 2) sweeps run (nested loops do not multiply).
Each `if` / `match` adds one join point, so a nested block hands its parent O(1) exits. A statement conflicts with a
borrow only where that borrow is live at it, and only statements reachable from the entry are checked.

### 4.3 Complexity bound (rule 2 of AGENTS.md)

LB2 is O(S x V) per block (S statements, V simultaneously live views). B4 replaces the scan with two indexes:

- per root `r` (and per footprint path): a bit mask `root_mask[r]` of the borrows whose provenance contains `r`,
  split into a shared mask and an exclusive mask;
- per statement: its root mentions (already collected for the effect pass), each tagged read / write / move.

Liveness is a bit set over the B borrow variables of the function: O((d + 2) x S x B / 64) words, d the loop
nesting depth (§4.2). Conflict checking tests each root mention once with `live & root_mask[r] != 0` (the shared or
the exclusive mask by mention kind): O(B / 64) per mention, so O(M x B / 64) in total, M the number of root
mentions; only a mention that conflicts scans its mask to name the borrow in the message, and checking stops at the
first error. Total **O((d + 1) x S x B / 64 + M x B / 64 + Σ|provenance|)** per function, linear in program size
while B < 64 (one word). Functions with no `ref` type, borrowing struct or place binding still skip the pass.

### 4.4 Refusals

| Program | Code |
|---|---|
| `let d = inout P;` where a place root is not a `let mut` local or `inout` parameter, the arms have different roots or types, `P` is a call, `subview`, map lookup or an element of a collection of non-struct, non-scalar type | `type/place_binding` |
| a place binding stored in a field, returned, moved, passed by `take`, captured by `par`, or bound with `let mut` | `type/borrow_escape` (returned / stored) or `type/borrow_signature` (`let mut`, `take`, `par`) |
| any use of a path overlapping `d`'s footprint (or of another place binding over it), contract clauses included, while `d` is live | `type/borrow_conflict` |
| a `ref` field of an `inout` borrowing-struct parameter assigned data not from that parameter | `type/borrow_escape` |
| a borrowing struct in `Vec` / `Option` / `Result` / enum payload / generic argument | `type/borrow_signature` (unchanged, §11) |

## 5. FM: place bindings (R-6g9-5)

### 5.1 Meaning

`let d = inout P;` makes `d` an alias of the place chosen by `P` at the binding. The index of `P[i]` and the
condition of `if` are evaluated once, at the binding. `d` behaves like an `inout` parameter of the enclosing scope:
reads see the place, writes change it, `inout d` passes it on (reborrow).

### 5.2 Verification obligations

- `P[i]`: `0 <= i < len(..)` at the binding (`verify/index_oob`, as an indexed read).
- `d` has the place's type and its invariants: struct invariants of `d`'s type are assumed at the binding and
  checked at the write-back, as for an `inout` argument at a call.
- Invariants of the *root* (`x`'s struct invariant, which may relate `x.a` to `x.b`) are checked at every
  write-back, exactly as after an R-6g-3 field-path call: an invariant broken through `d` is reported there
  (`verify/invariant`).

### 5.3 z3 encoding: copy-in / write-back

At the binding, `d := ite(c, x.a, x.b)` (or `x.v[i0]` with `i0` a snapshot of `i`). `d` is then an ordinary local.
The write-back points are the **live-to-dead edges** of `d` under §4.2: every control-flow edge from a point where
`d` is live to a point where it is not. On a straight path that is the edge after the last use. When `d` is bound
before a loop and used inside it, `d` is live at the loop head, so the whole loop is live and the only live-to-dead
edges are the loop's exits (`break`, the failed condition, `return`); there is no write-back on the back edge.
Each `return` / `break` / `continue` that leaves the live region is such an edge. At each one the verifier emits
`x := x[a := ite(c0, d, x.a), b := ite(c0, x.b, d)]` (or `x.v := x.v[i0 := d]`), `c0` the snapshot of the
condition, then checks the root invariants (§5.2). This is the R-6g-3 call encoding stretched over a region. It is
exact because the checker forbids every mention of the footprint while `d` is live, contracts included, so nothing
can observe the gap between a write through `d` and the write-back; hence no prophecy (Creusot `^d`) is needed.
One write-back per edge keeps the formula linear in the number of exits.

### 5.4 Codegen

- Rust: `let d = &mut x.v[i];` (`get_unchecked_mut` when verified) / `let d = if c { &mut x.a } else { &mut x.b };`;
  reads `d.f`, writes `d.f = e`, whole writes `*d = e`, `inout d` passes `d` (reborrow `&mut *d` where rustc needs it).
- LLVM: `d` is a `ptr`: a `select` of the two field GEPs, or the element GEP into the vector's data. The same code
  path as an `inout` parameter (`inout_place_ptr`, `llvmgen.py:1336`). The vector cannot grow while `d` is live
  (the root is exclusive), so the pointer stays valid.

## 6. Mutable borrowing structs (R-6g9-6)

`let mut u: Units = Units { raw: v, pos: 0 };` and `fn next(inout u: Units)`:

- `u.pos = e` and whole assignment `u = Units {..}` are allowed; `u.raw = e` is allowed and adds `e`'s provenance
  (local) or must come from `u` itself (parameter, §4.1).
- The verifier treats `u` as any mutable struct; `ref` fields are values (`copy(subview(..))` model), so no new
  encoding. Struct invariants are re-checked where they are for owned structs.
- Rust: `let mut u = Units { .. };`, `fn next(u: &mut Units<'_>)` (a borrowing struct `inout` parameter that is not
  in a `from` list gets an anonymous lifetime; when it is in a `from` list, `u: &mut Units<'a>` and the result
  `&'a [u8]` borrows what `u` borrows, as for read parameters).
- LLVM: same as any `inout` struct parameter (`ptr`); the `ref` field pair is copied by value on assignment and is
  never dropped.

## 7. LLVM codegen (B1, B2)

Current owned layout: a buffer is `{ ptr, i64 }` as an SSA value and `{ ptr, i64, i64 }` (data, len, cap) as a
struct field (`llvmgen.py:77`, `:729`, `:1863`); extended aggregates are passed as `ptr` and returned through a
leading `ptr %out`; a value is a *view* or *owned* syntactically, and a view is never dropped (`llvmgen.py:4866`).
A `subview` argument is already a `{ ptr + lo, hi - lo }` pair (`llvmgen.py:3671`), and a literal bound to a
borrowed local reads the read-only global (`llvmgen.py:2410`, `:3227`).

So a borrowed view is the existing view value, made storable. One rule changes: **ownership is decided by type
before syntax.** Today "a call result is fresh" and fresh values are dropped at the end of the statement or owned
by the slot they land in; a `from` call returning `ref [u8]` would then be freed, a double free of the owner's
buffer. Under R-6g9-1 a value of a `ref` type is never owned, whatever expression produced it, and a borrowing
struct value owns only its owned fields.

- **`ref [u8]` / `ref str` value**: `{ ptr, i64 }`, never owned, never dropped, never cloned deep (clone = copy the
  pair). A `ref` local is a stack slot of `{ ptr, i64 }`.
- **`ref` result** (`-> ref [u8] from(..)`): returned by value as `{ ptr, i64 }` (two registers on arm64 and
  x86-64); no `%out`.
- **`ref` field** of a borrowing struct: `{ ptr, i64 }` inline, *not* the `{ ptr, i64, i64 }` owned field form, so
  a field read is one load of the pair and the struct is 8 bytes smaller per view than an owned field.
  `JsonDocument { source: ref [u8], tape: [i64] }` is `{ { ptr, i64 }, { ptr, i64, i64 } }`, the same size and
  field order as Rust's `Document<'a>` (`&[u8]` + `Vec<i64>`). Every reader of a field (`g9_gep` users, read
  argument passing, `subview` of a field, `len`, `==`) goes through one helper that returns the `{ ptr, i64 }`
  pair of a buffer-typed field: for a `ref` field it is the field itself, for an owned field the first two words.
  So `subview(doc.source, lo, hi)` and passing `doc.source` to a `[u8]` parameter need no other adapter.
- **Moving an owned field out of a borrowing struct** (`a2`: `move document.tape` in a consuming `match`, R-6g8-4):
  the drop of the remainder skips `ref` fields anyway, so the partial move is the owned-struct case unchanged.
- **Borrowing struct helpers**: `pruv_drop__T` / `pruv_clone__T` skip `ref` fields (drop) and copy the pair
  (clone); owned fields are handled as today. A borrowing struct is an extended aggregate: read parameter `ptr`,
  result through `ptr %out`, which is what the Rust backend's `&Doc<'a>` / `Doc<'a>` mean.
- **Read parameter of a `from` list**: unchanged (`ptr` + length for `[u8]` / `str`, `ptr` for a struct, `ptr` to
  the `{ ptr, i64, i64 }` slot for an `inout` buffer). A result that borrows an `inout` buffer (`text_of`) returns
  `{ data, len }` read from the slot after the callee's last write; the buffer cannot be reallocated while the
  result lives (exclusive root), so the pointer stays valid.
- **Run-time checks**: none new. Provenance guarantees the owner outlives the view (the LB3 note, and Vale's frozen
  region argument). `--no-verify` keeps the existing `subview` range checks.
- **Host entry / ABI**: borrowing types stay out of host entries (`type/borrow_signature`, unchanged).

## 8. Both compilers

| Area | pruvc (Python) | pruvc-rs |
|---|---|---|
| B1/B2 LLVM | `llvmgen.py`: remove the `prog.borrow_site` refusal (`:6873`); `lty` of `ref` types and borrowing structs; view slots; drop/clone helper generation | `pruv-core/src/llvmgen.rs` (+ `abi.rs` for helper emission); same refusal removal |
| B3 effects | `check.py` effect inference for R-6g8-3 locals | `check.rs` effect inference |
| B4 liveness + index | `check.py` borrowed-view pass | `borrowed.rs` |
| B5 place bindings | `vparser.py` (`let d = inout P`), `vast.py`, `check.py`, `verify.py` (write-back), `codegen.py`, `llvmgen.py` | `parser.rs`, `ast.rs`, `check.rs`, `borrowed.rs`, `pruv-proof/src/symbolic.rs`, `codegen.rs`, `llvmgen.rs` |
| B6 mutable borrowing structs | `check.py` (placement), `codegen.py` (`&mut T<'_>`) | `borrowed.rs`, `codegen.rs` |
| B7 field lending, `ref [i64]` | `typeinfo.py`, `vparser.py` (`from(p.f)`), `check.py`, `codegen.py`, `llvmgen.py` | `parser.rs`, `ast.rs`, `borrowed.rs`, `codegen.rs`, `llvmgen.rs` |
| B8 view payloads | `check.py`, `verify.py` (payload = `copy` model), `codegen.py`, `llvmgen.py` (enum layout) | `borrowed.rs`, `symbolic.rs`, `codegen.rs`, `llvmgen.rs` |

Parity check per sub-bundle: byte-identical Rust (verified and `--no-verify`), identical diagnostics, and for every
sub-bundle except B3 and B4 the LLVM binary of each accept test prints the same as the Rust binary.

## 9. Interaction with `inout`, `move`, `take`, consuming `match`

- **`inout` argument**: a root passed `inout` while a shared borrow of it is live is `borrow_conflict` (LB2). A
  place binding `d` may itself be passed `inout d` (a reborrow; `x` stays frozen). Two place bindings of the same
  root are never live together (the second binding mentions `x`).
- **`move` / `take`**: moving a root under a live borrow is `borrow_conflict` (LB2). Moving a borrowing struct moves
  its provenance with it (`let e = move d;`). `take` parameters of a borrowing type stay `borrow_signature`: an
  owned-consuming callee cannot hold a borrow past its return except through a `from` result, which already exists.
- **Consuming `match` / TK (R-6g8-4)**: `move binder.tape` out of a consuming match on an owned struct is
  unchanged; for a borrowing struct the binder's provenance is the scrutinee's. A consuming match whose scrutinee is
  a root of a live borrow is `borrow_conflict`.
- **R-6g-3 field-path `inout`**: `f(inout x.a)` while `d = inout if c { x.a } else { x.b }` is live is a conflict
  (the footprint is `x`, §4.1); `f(inout x.bytes)` while `d = inout x.decoders[s]` is live is not (disjoint paths).
- **`vec_at_mut` / `map_get_mut` arms (R-6e-8)**: a place binding inside such an arm may take the arm binder as its
  root; the arm's own disjointness rules apply first.

## 10. Blocked files and the rewrite each sub-bundle enables

`lib/json`, `lib/parse`, `lib/lines` are not present under `bench/l3/lib` at HEAD (it holds `ent_*` pieces only);
they are the library copies of the `pkgs` packages and take the same rewrite as their `pkgs` rows.

| File | Today (copy / shape gap) | Rewrite (hand form) | Needs |
|---|---|---|---|
| `pkgs/json` | `JsonDocument { source: [u8], tape }` owns a copy of the input; `JsonParse { Parsed(document), Failed }` | `JsonDocument { source: ref [u8], tape: [i64] }` with the same invariants (`Document<'a>`); `JsonParse::Parsed(tape: [i64])` (hand `ParseResult<Vec<i64>>`) | B1, B2 |
| `pkgs/parse` | entry returns `JsonParse` with a copied source (P7 `copy(input)`) | `parse(input: [u8]) -> JsonParse` returns the tape only; no source copy | B2 |
| `pkgs/lines` | `parse_range` / `parse_copy` pay `copy(subview(data, lo, hi))` (L1, L3) | `parse(subview(data, lo, hi))` then `JsonDocument { source: subview(data, lo, hi), tape }` (hand `parse(&data[lo..hi])`) | B2 |
| `pkgs/access` | `raw_span(doc, id) -> Span` (indices), callers slice again; records read by index arithmetic on `doc.tape` | `span(doc, rec: [i64]) -> ref [u8] from(doc)` (hand `Document::span`, B2); `record(doc, id) -> ref [i64] from(doc.tape)` (B7); `value(doc, id) -> Result<ref [i64]> from(doc.tape)` (B8) | B2, B7, B8 |
| `pkgs/parse` `Pass` (parser state) | `Pass` holds indices and the tape is passed `inout` beside the input | `struct Parser { input: ref [u8], pos: i64, tape: [i64] }`, `let mut p`, methods `fn record(inout p: Parser, ..)` (hand `Parser`, `hand_json.rs:248`) | B6 |
| `pkgs/jstr` | J1 cursor written as `(source, start, end)` indices and a `Step` result | `struct Units { raw: ref [u8], pos: i64 }`, `fn next(inout u: Units)` (hand `units`, `body_units`) | B6 (+B2 on LLVM) |
| json roots (`json/a0..a5_root`, `b_root`, `q_root`) | build an owned `JsonDocument`; `a2` returns `copy(document.tape)` | build `JsonDocument { source: input, tape }` from the parsed tape; `a2` returns the tape moved | B2 |
| `codex/item` | `item_text` / `stamp` return `copy(none)` / `copy(iso.text)` | `-> ref str from(d)` returning the document view or the static `""` | B2 (+B3 for `none` locals) |
| `codex/util` | `raw(d, id) -> [u8]`, `str_at(d, id) -> str` copy | `-> ref [u8] from(d)` / `-> ref str from(d)` | B2 |
| `grok/util` | `span_bytes`, `cfg_str`, `long_str` copy a range of the packet (`copy(x)`, `copy(none)`) | `-> ref [u8] from(t)` / `-> ref str from(t)` (hand `text_of` without the scratch branch) | B1, B3 |
| `pkgs/text` (B-LLVM row) | `trim(s) -> str ! alloc`, `clip_bytes(s, max) -> str ! alloc` (`substr` copies) | `trim(s: str) -> ref str from(s)`, `clip_bytes(..) -> ref str from(s)` (hand `fn trim(&str) -> &str`) | B1 |
| `pkgs/tail`, transport X7 (FM row) | `ts: [i64]` decoder state, copied in and out | `Tail { decoders: Vec<Decoder>, bytes: [u8], .. }`, `let d = inout tl.decoders[s];` (`core_hand.rs:61`) | B4, B5 |

Rows tied to the stale `alloc` of literal locals (`text::engine_line`, `let none: str = ""` in codex / claude /
grok): drop `! alloc` after B3.

## 11. Sub-bundles (each ≤ ~2 h, both compilers)

Ordered so the first ones unblock the most rows. B1 → B2 is the critical path for the B row (every consumer row
except access's record lending and the two cursors); B3 is independent; B4 → B5 closes FM; B6 closes J1 and the
parser state; B7 → B8 close `pkgs/access`.

| # | Content | SPEC | Unblocks | Tests (new) |
|---|---|---|---|---|
| B1 | LLVM: `ref [u8]` / `ref str` locals, arguments, `from` results as `{ ptr, i64 }`; static literal views; remove the blanket refusal for programs without borrowing structs | R-6g9-1 | `text::trim`, `text::clip_bytes`, `grok/util`, `codex/util` view results on LLVM; R-6g-11 view tests run on LLVM | `r6g9_1_view_result`, `r6g9_1_inout_source` (`text_of`), `r6g9_1_literal` |
| B2 | LLVM borrowing structs: `{ ptr, i64 }` fields, the buffer-field pair helper, drop/clone helpers, read params, `%out` results, partial move of an owned field; invariants on borrowing structs (both backends) | R-6g9-2 | `pkgs/json`, `parse` (tape-only result), `lines`, `access` `span`, json roots, `codex/item`, `codex/util` | `r6g9_2_doc`, `r6g9_2_doc_invariant`, `r6g9_2_forward`, `r6g9_2_move_tape`, demo `lb2_borrow` on LLVM |
| B3 | `alloc` off for R-6g8-3 static text locals; static empty `[]` view | R-6g9-3 | the `none` / `engine_line` stale `alloc` marks; `text_of`'s `return &[]` | `r6g9_3_static_local`, `r6g9_3_empty_view`, `r6g9_3_owned_still_allocs` (reject) |
| B4 | Path liveness (worklist, gen/kill bit sets) + root masks (§4.2-4.3); every R-6g-11 test unchanged | R-6g9-4 | hand shapes where a view dies inside an `if`; prerequisite of B5 | `r6g9_4_dies_in_if` (accept), `r6g9_4_loop_still_live` (reject), `r6g9_4_nested_loops`, `r6g9_4_many_views` (compile-time bound) |
| B5 | Place bindings: parser, footprint conflicts, live-to-dead write-back with root invariant check, Rust + LLVM | R-6g9-5 | FM: `tail` decoder pick, transport X7 | `r6g9_5_vec_elem` (`core_hand` shape: `continue` and a disjoint `x.bytes` write inside the region), `r6g9_5_if_field`, `r6g9_5_loop_region`, `r6g9_5_index_bump` (accept); `r6g9_5_root_read`, `_contract_mention`, `_root_invariant` (verify), `_escape`, `_two_roots` (reject) |
| B6 | Mutable borrowing structs (`let mut`, `inout` param), owned fields writable | R-6g9-6 | jstr J1 `Units` cursor, `parse` `Parser` state | `r6g9_6_cursor`, `r6g9_6_parser` (ref input + owned tape), `r6g9_6_store_foreign` (reject `borrow_escape`) |
| B7 | `ref [i64]`, field lending `from(p.f)` | R-6g9-7 | `access` `record` | `r6g9_7_record`, `r6g9_7_lend_then_write` (reject `borrow_conflict`) |
| B8 | Views in `Option` / `Result` payloads | R-6g9-8 | `access` `value` | `r6g9_8_value`, `r6g9_8_payload_escape` (reject) |

Still not in this bundle (no hand form in the B / FM rows needs them): *borrowing structs* in `Option` / `Result` /
enum payloads (the hand parser returns the owned tape), `Vec<ref [u8]>`, and a struct with two independent
lifetimes (`Parser<'a, 'b>`). Each becomes its own row when a rewrite hits it; the last is the point where a
`from` list stops being enough and named lifetimes would have to be reconsidered.

## 12. Debate

The draft (B1-B6, with record lending and payload views deferred) was attacked by a second model. The bundle's
`debate` role could not run: the delegation was refused because the routed account had hit its usage limit. The
opposing review was therefore run as a sub-agent on Claude Sonnet (a model different from the author, Claude Opus
5.5), with the draft and the source files cited above. Each attack, the answer, and what changed:

1. **Place-binding write-back in loops and root invariants.**
   *Attack:* "after its last use on each path" is ill-defined when `d` is bound before a loop: the last use differs
   per iteration, and a write-back on the back edge would flush a still-live `d`. A root invariant relating `x.a`
   and `x.b` could be broken through `d` and never checked. Contracts mentioning `x` were not said to be mentions.
   *Answer:* Right on all three. Under path liveness `d` is live at the head of such a loop, so the correct points
   are the live-to-dead edges, which are the loop exits; root invariants must be checked where the R-6g-3 call
   encoding checks them.
   *Decision:* §5.3 defines write-back on live-to-dead edges; §5.2 checks root invariants at each write-back; §4.1
   counts contract clauses as mentions. New tests `r6g9_5_loop_region`, `_contract_mention`, `_root_invariant`.

2. **"At most two passes" and the complexity bound.**
   *Attack:* reassignable borrowing structs (B6) add kills, nested loops iterated one at a time multiply, and the
   per-root list makes each mention O(borrows of that root), so many subviews of one `src` are quadratic.
   *Answer:* Right. Per-loop iteration was the wrong algorithm; a single worklist with gen/kill bit sets is bounded
   by (depth + 1) visits per node. The conflict test should be a mask intersection.
   *Decision:* §4.2 uses one reverse-postorder worklist; §4.3 states O((d + 1) x S x B/64 + M x B/64 + Σ|prov|)
   with `live & root_mask[r]`. New test `r6g9_4_nested_loops`.

3. **LLVM ownership is syntactic, so a `from` call result would be freed.**
   *Attack:* `llvmgen.py:4866-4867` says a call result is fresh and owned; a `ref` result would be dropped, a
   double free. `{ ptr, i64 }` ref fields break readers that assume the 3-word field. `a2`'s partial move out of a
   borrowing struct is not covered. `subview` of an empty, null-data vector computes `null + lo` under `inbounds`.
   *Answer:* The first three are right. The last is not a bug: `subview` bounds give `lo == 0` for an empty
   vector, and an `inbounds` GEP with a zero offset from null is defined (LangRef allows the null pointer itself);
   that path already exists today for `subview` arguments.
   *Decision:* §7 states the type-before-syntax ownership rule, a single buffer-field pair helper for all field
   readers, and the partial move; no change to `subview` lowering. New test `r6g9_2_move_tape`.

4. **Hand shapes the design cannot express.**
   *Attack:* `hand_json.rs:282-300` needs `record(&self) -> &[i64]`, `value() -> Result<&[i64], i64>` (field
   lending, `ref [i64]`, a view in a `Result`), yet the draft claimed B2 unblocks all of `access`. The hand
   `Parser { input, pos, tape }` (`hand_json.rs:248`) is a mutable borrowing struct with an owned field, not in the
   tests. `text_of` returns `&[]`, which the literal rule did not cover.
   *Answer:* Right. Leaving those for "when a rewrite hits them" contradicts the rule that the Pruv shape matches
   the hand shape function by function; `access` is a B-row file, so they are in scope.
   *Decision:* added B7 (`ref [i64]`, `from(p.f)`, R-6g9-7) and B8 (views in `Option` / `Result` payloads,
   R-6g9-8); `access` needs B2 + B7 + B8; B6 lists the parser state and `r6g9_6_parser`; B3 adds the static empty
   view. Borrowing *structs* in payloads stay out (no hand form needs them).

5. **Whole-root exclusivity and the frozen index are stricter than rustc.**
   *Attack:* `&mut self.decoders[stream]` borrows only `self.decoders`; the draft froze all of `tl` and also the
   index `i`, although the index is snapshotted. And `from` lists cannot express two independent lifetimes.
   *Answer:* Right on both freezes. `core_hand.rs:61-75` happens to pass under whole-root plus path liveness
   (`self.append` runs only after `decoder`'s last use on each path), but transport X7 and the RESHAPE rewrite touch
   sibling fields, and freezing a snapshotted index has no soundness reason. R-6g-3 already has the path-overlap
   test. Two lifetimes: no hand form in these rows needs them, so the inferred design stands; recorded as the point
   to revisit.
   *Decision:* §4.1 uses the longest common field path as the exclusive footprint with R-6g-3 overlap, the index is
   not frozen, and the write-back covers the footprint only. New tests `r6g9_5_index_bump` and a disjoint-field write
   in `r6g9_5_vec_elem`. Two-lifetime structs listed under "still not in this bundle".

Kept against the review: inferred lifetimes (`from` lists) over named lifetimes. Every hand form in the B, B-LLVM
and FM rows has one borrowed region per result or struct, Rust's own elision covers all but `text_of`, and
`from(src, buf)` states that one exactly.
