# L3 LB1: the hand_life value shape in Pruv

Language bundle LB1 of the Rust-shape reset. Goal: Pruv declares and uses the `hand_life.rs` value shape
(`enum Value { ..., Array(Vec<Value>), Object(Vec<(Text, Value)>) }`) directly, in both compilers, Rust backend.

## What changed

- **R-6e-20 recursion through a vector** (SPEC.md, after R-6e-19). A non-generic struct field or enum payload field
  may be `Vec<S>` with `S` a non-generic struct/enum of the same recursive group. The field is a plain `Vec<S>`
  (no `Box`); the vector edge does not count for inhabitance (an empty vector ends the recursion).
  - Allowed on `Vec<S>`: `vec_new`, `vec_push` (constructor / call / `move` payload), `len`, `vec_get` as a borrowed
    read (match scrutinee, read argument, field read), `vec_at_mut` (in-place replace of an element or one of its
    fields, order and duplicates kept, as in `hand_life.rs` `put` / `edit_object`).
  - Generic recursion through a vector: `type/recursive_generic_type`. HashMap recursion: still rejected
    (`type/foreign_owned_type`); later work.
  - LLVM: `codegen/unsupported_type` ("has owned collection fields, which need the Rust backend"). LB3 does LLVM.
- **Sound size measure.** `size(x)` = 1 + field weights; a `Vec<S>` field weighs the sum of its elements' sizes.
  Both verifiers keep an uninterpreted handle sort per element type and an uninterpreted `vsum(arr, len)`; only
  point-wise true facts are added: `vsum >= 0`, empty vector sums to 0, `vec_push` adds `size(x)`, a `vec_at_mut`
  write-back replaces the old element size by the new one, an element read at a proved index has `size <= vsum`
  and is well formed. No "child is smaller" axiom. Recursion on an element read from the parameter proves
  `decreases size(v)`; recursion on the same value is `verify/recursive_decrease`.
  The run-time size contract (`pruv_csize__S`) folds the element sizes the same way.
- **Codegen.** A borrowed element read emits `(*unsafe { v.get_unchecked(i) })` when the index is proved, else
  `(*v.get(i).unwrap_or_else(|| fail))`; no clone. Both compilers emit the same text (checked on the spec tests and
  the demo in all verify / foreign-check modes, and by `tests/diff_rust.py`).

Files: `pruvc/check.py`, `pruvc/codegen.py`, `pruvc/verify.py`, `pruvc-rs/crates/pruv-core/src/check.rs`,
`pruvc-rs/crates/pruv-core/src/codegen.rs`, `pruvc-rs/crates/pruv-proof/src/model.rs`,
`pruvc-rs/crates/pruv-proof/src/symbolic.rs`, `SPEC.md`.

## Tests

| file | expect |
|---|---|
| `tests/spec/r6e_20_value_tree.pruv` | accept; recursive depth proves `decreases size(v)`, no `Box`, no clone |
| `tests/spec/r6e_20_lookup_last.pruv` | accept; last-matching-key read, `vec_at_mut` replace, run output |
| `tests/spec/r6e_20_same_value.pruv` | reject `verify/recursive_decrease` |
| `tests/spec/r6e_20_generic.pruv` | reject `type/recursive_generic_type` |
| `tests/spec/r6e_20_hashmap.pruv` | reject `type/foreign_owned_type` |
| `tests/spec/r6e_20_llvm.pruv` | reject `codegen/unsupported_type` on the LLVM backend (LB3 replaced it with the accept test `tests/spec/r6e_21_llvm_tree.pruv`) |
| `tests/spec/r6e_20_observed.pruv` | accept; an input with `len == 0` has size 1, `size(t.kids[0]) < size(t)` in a contract |
| `tests/spec/r6e_20_push_moved.pruv` | accept; a local moved into `vec_push` |
| `tests/spec/r6e_20_other_vec.pruv` | reject `verify/recursive_decrease`; recursion on an element of a vector built locally |
| `tests/spec/r6e_20_grow_child.pruv` | reject `verify/ensures`; the old size after a child replace |
| `tests/spec/r6e_20_outer_owner.pruv` | reject `verify/call_requires`; a struct outside the group weighs its `Vec<S>` elements |
| `tests/spec/r6e_20_owned_get.pruv` | reject `type/foreign_owned_type`; returning a borrowed element |
| `tests/spec/r6e_20_field_borrow.pruv` | reject `type/recursive_borrow`; `vec_at_mut` during `match vec_get(..).v` |
| `tests/spec/r6e_20_push_borrowed.pruv` | reject `type/foreign_move`; a read parameter as `vec_push` payload |
| `tests/spec/r6e_20_generic_hashmap.pruv` | reject `type/recursive_generic_type` before the HashMap check |
| `tests/spec/r6e_20_nested_borrow.pruv` | reject `type/recursive_borrow`; `vec_at_mut` during a match on an element of an element |
| `tests/spec/r6e_20_field_owned.pruv` | reject `type/foreign_owned_type`; returning a field of a borrowed element |
| `tests/spec/r6e_20_forall_child.pruv` | accept; `forall i in 0..len(t.kids): size(t.kids[i]) < size(t)` |

The audit (Codex gpt-6.2-astra ultra) found the outer-owner size split, four Python checker gaps, two missing
verifier facts, a verifier cost on programs without vector edges and the missing negative tests above; all were
fixed. The confirm audit (same model) then found a nested element match that missed the borrow (both compilers),
a field of a borrowed element accepted as owned (Rust), quantified child-size contracts unproved (Rust) and a
remaining `vec_new` / `vec_push` cost (Rust); these were fixed by the main session after that audit (no further
audit), with the three tests above. Its fifth point, run-time `requires` checks over `size(v[i])`, is an open item.

Demo: `bench/l3/budget/m12_test/value_ops.pruv` ports the hand_life object operations (rfind get, put = replace the
last match in place or append, duplicates kept, array child replaced in place, node count and sum over the tree).
`value_ops_hand.rs` is the hand Rust twin. Both compilers' output and the hand binary print the same 9 lines
(`3 7 9 2 -1 3 5 8 59`). M12 performance is not part of LB1.

## Open items

- **UTF-16 text.** There is no `u16` element type; the demo keeps UTF-16 code units in `Vec<i64>`. The smallest
  feature would be a `[u16]` / `Vec<u16>` element kind like `[u8]` (range 0..=65535 on store, 2-byte storage); not done
  in LB1.
- **Pushing into an `inout Vec<S>` parameter.** An `inout` vector cannot be reassigned as a whole, so `put` takes the
  owning struct (`inout o: Obj`, `o.fields = vec_push(move o.fields, ..)`), as hand_life's `&mut self` does.
- **Moving a field out for a final value** (`Value::Object(move o.fields)`) needs the field to be restored by the next
  statement; the demo reads through the field instead.
- **Owned element read / remove.** `vec_get` of a recursive element is borrowed only; there is no take/`retain`
  (hand_life `remove`), so element removal is later work.
- **Editing inside a nested object through `vec_at_mut`** (hand_life `edit_object` on a nested `Value::Object`) needs a
  mutable match on the element's payload; not covered.
- HashMap recursion and generic recursion (see above); LLVM (LB3).
- The verify-mode generated Rust can contain a nested `unsafe` block (rustc warning only).
- **Run-time check of `requires ... size(v[i]) ...`.** Both compilers reject generating it (`codegen/unsupported_type`
  with `--no-verify`); such contracts work only when verified.
