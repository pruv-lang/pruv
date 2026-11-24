# L3 LB2: borrowed views without copies

Language bundle LB2 of the Rust-shape reset. Goal: Pruv writes the hand-Rust borrowing shapes directly, in both
compilers, Rust backend, with no copy: `struct Req<'a> { method: &'a [u8], .. }`, a document that borrows its input,
and a result that borrows either the input or a reused scratch buffer (`text_of` in `bench/l3/grok/perf/hand_rpc.rs`).

## What changed

- **R-6g-11 borrowed views** (SPEC.md §6g, after R-6g-10).
  - *Syntax.* `ref [u8]` / `ref str` types (`ref` is contextual), structs with `ref` fields (borrowing structs, Rust
    `struct Doc<'a>`), and results `-> T from(p1, .., pn)` (`from` is contextual) that name the parameters the result
    may borrow. Pruv writes no lifetimes.
  - *Checking: provenance + exclusivity.* Every borrowed value carries a set of root variables, each shared (read
    argument, `subview`, whole borrow) or exclusive (`inout` argument). While a borrow is live (from the statement
    after its `let` through the last statement of the block that mentions it; a loop or `if` counts whole), a shared
    root may be read but not stored, passed `inout`, moved, grown or cleared, and an exclusive root may not be used
    at all: `type/borrow_conflict`. This is the Rust `&` / `&mut` rule, so the generated Rust borrow-checks.
  - *Escape.* A returned borrow must come from the `from` parameters (`type/borrow_escape`). A borrowing local
    passed at a `from` position lends what it borrows, not its own storage; a view of an owned field of a
    borrowing-struct parameter cannot be returned (`type/borrow_escape`).
  - *Placement.* `ref` / borrowing types only in an immutable `let`, a result, a borrowing-struct field and (struct
    only) a read parameter; elsewhere `type/borrow_signature`. A view in an owned position is `type/array_alias`
    (`copy(m)` makes an owned array); a store into a view is `type/immutable`.
  - *Proof.* A view bound from `subview(x, lo, hi)` is modelled as `copy(subview(x, lo, hi))`; a call result is a
    fresh value constrained by the callee's `ensures`. No copy at run time.
- **R-6g-12 string literal arguments (G-DB63-2).** A string literal passed straight to a read `str` parameter of a
  user function is emitted as the literal (`f("abc")`), with no allocation and no `alloc` effect.
- **Codegen (Rust only).** `fn f<'a>(..)` for a function with a `from` list; its `from` parameters are `&'a [u8]`,
  `&'a str`, `&'a mut ::std::vec::Vec<u8>` (resizable `inout`), `&'a mut [u8]` (fixed `inout`) or `&Doc<'a>`
  (borrowing struct: the result borrows what the struct borrows, not the struct binding); the result is `&'a [u8]`,
  `&'a str` or `Doc<'a>`. A view is `unsafe { x.get_unchecked(lo..hi) }` when verified and the checked slice under
  `--no-verify`. Borrowing structs derive `Clone, Debug`. No `to_vec` or `clone` of borrowed bytes. In a program
  with `pub fn`, a private function with a `from` list keeps its source name. The LLVM backend rejects a program
  with a `ref` type or a borrowing struct with `codegen/unsupported_type` (LB3).
- **Fix found on the way.** The Python checker read the callee name of a call after `s.call` had rewritten it in
  place (`old(c).m` becomes a field read), an internal error on `tests/runtime/lg22_old_struct.pruv`; the lookup now
  happens first.
- **Audit fixes** (Rust checker, `borrowed.rs`). A view of an owned `[u8]` / `str` field of a local record now holds
  only the record variable, not what the record borrows (Python and rustc already agreed); a `from` list naming a
  parameter twice is `type/borrow_signature` (Python already rejected it). SPEC states both.

Files: `pruvc/check.py`, `pruvc/codegen.py`, `pruvc/llvmgen.py`, `pruvc/modules.py`, `pruvc/mono.py`,
`pruvc/small.py`, `pruvc/typeinfo.py`, `pruvc/vast.py`, `pruvc/verify.py`, `pruvc/vparser.py`,
`pruvc-rs/crates/pruv-core/src/borrowed.rs` (new), `abi.rs`, `ast.rs`, `bindings.rs`, `check.rs`, `codegen.rs`,
`emit_common.rs`, `foreign_shim.rs`, `lexer.rs`, `lib.rs`, `llvmgen.rs`, `modules.rs`, `mono.rs`, `parser.rs`,
`pruvc-rs/crates/pruv-proof/src/symbolic.rs`, `SPEC.md`.

## Tests

| file | expect |
|---|---|
| `tests/spec/r6g_11_doc_input.pruv` | accept; `struct Doc<'a>`, `fn body_of<'a>(d: &Doc<'a>) -> &'a [u8]`, no copy |
| `tests/spec/r6g_11_two_sources.pruv` | accept; `text_of` borrows the input or an `inout` scratch buffer |
| `tests/spec/r6g_11_local_view.pruv` | accept; G-DB63-1 `subview` bound to a local, `get_unchecked`, no copy |
| `tests/spec/r6g_11_copy_owned.pruv` | accept; `copy` of a view, owner free again after the view's last use |
| `tests/spec/r6g_11_struct_forward.pruv` | accept; a local `Doc` passed to `body_of` and its view returned |
| `tests/spec/r6g_11_owned_field_local.pruv` | accept; view of an owned field of a local record holds the record only, so its borrowed owner may be written |
| `tests/spec/r6g_11_escape_local.pruv` | reject `type/borrow_escape`; result borrows a local |
| `tests/spec/r6g_11_escape_struct.pruv` | reject `type/borrow_escape`; borrowing struct borrows a local |
| `tests/spec/r6g_11_escape_owned_field.pruv` | reject `type/borrow_escape`; view of an owned field of a struct parameter |
| `tests/spec/r6g_11_reuse_buf.pruv` | reject `type/borrow_conflict`; scratch reused while a result borrows it |
| `tests/spec/r6g_11_read_exclusive.pruv` | reject `type/borrow_conflict`; reading a buffer lent `inout` |
| `tests/spec/r6g_11_realloc_owner.pruv` | reject `type/borrow_conflict`; owner grows under a view |
| `tests/spec/r6g_11_store_owner.pruv` | reject `type/borrow_conflict`; owner written under a view |
| `tests/spec/r6g_11_move_owner.pruv` | reject `type/borrow_conflict`; owner moved under a view |
| `tests/spec/r6g_11_loop_conflict.pruv` | reject `type/borrow_conflict`; view live for the whole loop |
| `tests/spec/r6g_11_no_from.pruv` | reject `type/borrow_signature`; borrowed result without `from` |
| `tests/spec/r6g_11_from_duplicate.pruv` | reject `type/borrow_signature`; a parameter named twice in `from` |
| `tests/spec/r6g_11_ref_param.pruv` | reject `type/borrow_signature`; `ref` parameter |
| `tests/spec/r6g_11_mut_ref.pruv` | reject `type/borrow_signature`; `let mut` view |
| `tests/spec/r6g_11_store_view.pruv` | reject `type/immutable`; store into a view |
| `tests/spec/r6g_11_own_alias.pruv` | reject `type/array_alias`; view in an owned position |
| `tests/spec/r6g9_1_struct_llvm.pruv` (was `r6g_11_llvm`) | reject `codegen/unsupported_type` for a borrowing struct on the LLVM backend (views lower under R-6g9-1) |
| `tests/spec/r6g_12_literal_arg.pruv` | accept; `f("abc")` with no allocation and no `alloc` |
| `tests/spec/r6g_12_literal_owned.pruv` | reject `effect/missing_effect`; a literal bound to an owned `str` still allocates |

Every accept test and the demo give byte-identical Rust from both compilers in verified and `--no-verify` mode, and
that Rust compiles with rustc; every reject test gives the same code from both compilers.

## Audit notes

The Python and Rust implementations were written by Codex gpt-6.2-sol (one worker per compiler); the main session
did the parity fixes. Parity work removed: a literal-argument special case in each compiler that only matched the
first test text (the test now uses a regex), and a Python `__pruv_lend` twin plus `Copy` derive that worked around
the template `&'a Doc<'a>`. That template tied the result to the struct binding, so Python forwarded a local struct
through the twin while Rust rejected it with `borrow_escape`. Both compilers now emit `&Doc<'a>` and agree
(`r6g_11_struct_forward`), and a view of an owned field of such a parameter, which `&Doc<'a>` cannot return, is
rejected in both (`r6g_11_escape_owned_field`).

One audit (Codex gpt-6.2-astra, scoped to the checkers after two runs over the whole diff were stopped by the
provider's content filter) found three required items:

- Rust rejected a view of an owned field of a local record while the record's borrowed owner was written; Python
  accepted, and the Python output compiles with rustc. Fixed in Rust (`r6g_11_owned_field_local`).
- `from(x, x)`: Python `borrow_signature`, Rust accepted. Fixed in Rust; SPEC says each name at most once
  (`r6g_11_from_duplicate`).
- Both checkers do work proportional to (statements x live views) in a block, quadratic when many views are live at
  once. This is new checking of new syntax only: a program with no `ref` type or borrowing struct skips it (both
  checkers test that first), so no existing program compiles slower. Not changed; listed under Open items.

The confirm audit (same reviewer) found both fixes correct and the checkers in agreement on nested owned and
borrowing-struct fields of locals and parameters. Its one required item was SPEC wording: the new sentence gave a
borrowing-struct field of a local record (`src(d.child)`) the record variable `d` as well, while both checkers (and
rustc, since `&Inner<'a>` lends only `'a`) take only what the record borrows. SPEC now says so, and the unreachable
branch in `borrowed.rs` that tried to add it was removed.

## Demo

`bench/l3/budget/lb2_borrow/borrow_demo.pruv` has `struct Req { method: ref [u8], path: ref [u8], n: i64 }`, a
`parse_req(src) -> Req from(src)` and `text_of(src, lo, hi, inout buf) -> ref [u8] from(src, buf)` that returns
`subview(src, lo, hi)` when the range has no backslash and otherwise unescapes into the reused buffer.
`borrow_hand.rs` is the hand-Rust twin (`Req<'a>`, `fn text_of<'a>(src: &'a [u8], .., buf: &'a mut Vec<u8>) ->
&'a [u8]`). Both compilers give the same Rust, which has no `to_vec` / `clone`, and the binary prints the same 9
lines as the hand twin (`3 6 224 525 12 5 433 3 224`). Performance is not part of LB2.

## Open items

- **LLVM** rejects borrowed views (LB3).
- **Mutable views** (`&mut [u8]` locals or fields) are not in the language; only `inout` parameters lend exclusively.
- **Borrowed collections.** `ref` types cannot be `Vec` / `Option` / map elements, enum payloads or generic
  arguments (`type/borrow_signature`), so a `Vec<&[u8]>` token list needs later work.
- **Checker cost.** Borrow checking is O(statements x live views) per block; a block with thousands of
  simultaneously live views would compile slowly. An index by root would make it linear.
- **Owned fields of a borrowing struct** cannot be lent through a `from` parameter (`&Doc<'a>`); a function that needs
  that returns from the field's owner instead.
