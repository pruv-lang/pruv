# L3 LG4: field element writes, array field moves, aggregate frame lemma

Scope: G-LG3-2 and G-LG3-1 from `docs/L3_LG3.md`, and G-R03D-1 from `docs/L3_RESTRUCTURE_SPEC.md` §7.5. Both
compilers (Python `pruvc/`, Rust `pruvc-rs/`) change with the same semantics and diagnostics, on both backends (Rust
source and LLVM). No `bench/` or `pkgs/` fragment is changed, and §6g is unchanged.

## What landed

- Field element write (SPEC.md §4, assignment places). `p.a[i] = v` and nested `p.s.a[i] = v` are statements when the
  root is a `let mut` local or an `inout` struct parameter and the field is `[i64]`, `[u8]` or `[bool]`. The proof
  obligations are those of `x[i] = v`: `verify/index_oob` for the index and `verify/elem_range` for a `[u8]` value
  (plus `verify/refinement_assign` for a refined element). The verifier reads the field array, stores into it with
  the same length and rebuilds the record along the path. A proved index lowers to an unchecked write on both
  backends; `--no-verify` keeps the guards. A `str` field is `type/immutable_string`, an immutable root is
  `type/immutable`. The AST node `IndexAssign` has a new `fields` list (empty for `x[i] = v`; the Rust dump writes it
  only when non-empty, and `tests/diff_py_adapter.py` serializes it the same way).
- Array field move (SPEC.md R-6e-16). The existing partial-move rule for `Vec` fields now also covers buffer fields
  `[i64]`, `[u8]` and `[bool]`: `move d.a` is allowed when `d` is a `let mut` local and the field is assigned again
  before `d` is read, moved or goes out of scope, on every path. A missing restore is `type/foreign_move`, any other
  array field move stays `type/array_alias`. Rust lowers the move to `std::mem::take`; LLVM copies the field's
  `{ptr, len, cap}` into the destination and stores the empty array into the field without freeing it. The restore
  is an ordinary field replacement.
- Aggregate frame lemma (G-R03D-1). When a new aggregate (`sum`/`count`-style quantified term) has a body that differs
  from an earlier one of the same kind and arity only in array terms, both verifiers add: on a non-empty range the
  two aggregates are equal, or a witness slot in the range has different bodies. This lets a proof carry a sum over
  an array across a write outside the range, and it is true of any two aggregates over the same range, so it adds no
  false facts. Committed earlier in this bundle (676da97).

| Row | Status | What exists now |
|---|---|---|
| G-LG3-2 | closed | `s.a[i] = v` on a local or `inout` struct, nested paths, `[u8]`/`[i64]`/`[bool]`, both backends. `tests/runtime/lg4_rt_field_index` (both backends, proof and `--no-verify`, allocation balance checked); `tests/must_reject/lg4_field_index_oob`, `lg4_field_index_u8_range`, `lg4_field_index_immutable`. |
| G-LG3-1 | closed | `move d.a` of a buffer field under R-6e-16, both backends. `tests/runtime/lg4_rt_move_array_field` (both backends, both flag sets, allocation balance); `tests/must_reject/lg4_move_array_field_no_restore` (`type/foreign_move`) and `lg4_move_array_field_nested` (`p.a = copy(move p.a)`, `type/array_alias` in both compilers; the Python checker gave `foreign_move` before the bundle audit). |
| G-R03D-1 | closed | Aggregate frame lemma in `pruvc/verify.py` and `pruvc-rs/crates/pruv-proof/src/symbolic.rs`. `tests/runtime/lg4_rt_agg_frame`; `tests/must_reject/lg4_agg_frame_changed` (a write inside the range still fails the assertion). |

Status: LG4: G-LG3-2 closed (field element writes on local and inout structs, both backends, both compilers).
Status: LG4: G-LG3-1 closed (`move d.a` for `[i64]`/`[u8]`/`[bool]` fields under R-6e-16, both backends).
Status: LG4: G-R03D-1 closed (aggregate frame lemma in both verifiers).

## New gaps

| Gap | What | Why not here |
|---|---|---|
| G-LG4-1 | After a call `f(inout s)` with a struct argument, the verifier forgets the lengths of the struct's array fields. | `inout_finals` keeps the pre-call length only for array arguments; a struct argument gets a fresh value with a well-formedness fact only. The callee has to state the lengths in its `ensures` (as `lg4_rt_field_index` does). Keeping them needs a per-field rebuild of the fresh struct, a verifier change in both compilers outside this bundle. |
| G-LG4-2 | `alloc(n, v)` as the value of a field assignment (`d.a = alloc(3, 0)`) gets no element type hint from the field type for `[u8]`. | The field restore in `lg4_rt_move_array_field` uses `copy(t)` instead. Type hints for field places are a checker change unrelated to the move rule. |

## Tests run

- Python compiler full suite, both backends: `TOTAL 4625/4625`. Rust compiler full suite, both backends: `TOTAL 4637/4637`
  (both before the audit fix below).
- `cargo test --release`: 150 passed. `tests/diff_rust.py --fuzz-count 20`: all same.
- After the audit fix (Python `array_alias` for a nested array field move): `--only must_reject` 903/903 with both
  compilers. `tests/run_all_rows.txt` regenerated with `--list-rows` (18 new rows, all LG4 tests, no removals).
- Bundle audit (Codex gpt-6.2-astra, ultra): no must-fix findings; one should-fix (the diagnostic above), fixed.
