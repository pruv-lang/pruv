# L3 LG5: struct length frame, field alloc hint, inout array reassignment, scalar inout on LLVM, copy_range overlap

Scope: G-LG4-1 and G-LG4-2 from `docs/L3_LG4.md`, G-DB4-1, G-DB1-2 and G-DB1-3 from `docs/L3_RESTRUCTURE_SPEC.md`
§7.5.1, and G-LG3-4 if time allowed (it did not). Both compilers (Python `pruvc/`, Rust `pruvc-rs/`) change with the
same semantics and diagnostics, on both backends (Rust source and LLVM). No `bench/` or `pkgs/` fragment is changed.

## What landed

- Struct array field length frame (G-LG4-1, SPEC.md section on `inout` struct arguments). After `f(inout y)` the
  caller keeps `len` of every array field of `y`, through nested struct fields, that `f` cannot change: `f` has no
  field assignment, move or resizing `inout` pass of that field or of an enclosing field, transitively through the
  functions it calls. The checker computes the changed field paths per (function, parameter); both verifiers assume
  the old length for every other field of the fresh post-call struct. A changed field gets no length fact.
- Field `alloc` element type hint (G-LG4-2). `p.f = alloc(n, v)` takes its element type from the field type, as a
  typed `let` does: `p.a = alloc(4, 200)` is a `[u8]` value, and `alloc(3, 300)` for a `[u8]` field is
  `verify/elem_range`.
- Whole reassignment of an `inout` array parameter (G-DB4-1, SPEC.md R-6g). `input = repack(input, hi)` and
  `a = alloc(len(old(a)) + 2, 7)` are allowed. Such a parameter is resizable: the caller learns its new length only
  from `ensures`, and the verifier gives the new value only the entry bounds `0 <= len <= MAX_LEN` (also for a
  generic `[T]`, under the instantiation of the call, inside `par` too). A host entry with such a parameter is
  `type/entry_signature`. The Rust backend passes `&mut Vec<T>`; the LLVM backend passes a pointer to the caller's
  `{ptr, len, cap}` slot and frees the old buffer on reassignment. `truncate(a, m)` of a resizable parameter copies,
  because the slot belongs to the caller (the in-place move is only for owned locals). The old
  `tests/must_reject/g5_whole_param_assign` (which expected `type/immutable`) is removed.
- Scalar `inout` on LLVM (G-DB1-2). An `i64` or `bool` `inout` parameter is a pointer on the LLVM backend (load on
  read, store on write); `codegen/unsupported_type` is gone. The verifier rules are unchanged.
- `copy_range` with `src` equal to `dst` (G-DB1-3, SPEC.md `copy_range`). `copy_range(inout a, i, a, j, n)` with the
  same variable or identical field path has memmove semantics: Rust `copy_within`, LLVM `llvm.memmove`. The bounds
  obligations are unchanged. Index arguments naming `dst` (including `len(dst)`) and any other `src` mentioning `dst`
  stay `type/inout_alias`; `tests/spec/r5_1_copy_range_alias` now checks the index case.

| Row | Status | What exists now |
|---|---|---|
| G-LG4-1 | closed | Per-field length frame after `f(inout s)`, nested fields, transitive through callees. `tests/runtime/lg5_rt_struct_len_frame` (both backends, proof and `--no-verify`); `tests/must_reject/lg5_struct_len_changed` (`verify/index_oob` when the callee changes the field). |
| G-LG4-2 | closed | `p.f = alloc(n, v)` uses the field element type. `tests/runtime/lg5_rt_field_alloc_hint`; `tests/must_reject/lg5_field_alloc_u8_range` (`verify/elem_range`). |
| G-DB4-1 | closed | Reassignable `inout` array parameters, both backends. `tests/runtime/lg5_rt_inout_reassign` (includes `truncate` of a resizable parameter), `tests/runtime/lg5_rt_generic_resize` (generic `[T]` in `par`); `tests/must_reject/lg5_inout_reassign_len` (`verify/index_oob` without a length clause), `lg5_inout_reassign_entry/` (`type/entry_signature`). |
| G-DB1-2 | closed | Scalar `i64`/`bool` `inout` on LLVM. `tests/runtime/lg5_rt_scalar_inout`; `tests/must_reject/lg5_scalar_inout_ensures` (`verify/index_oob`). |
| G-DB1-3 | closed | `copy_range` overlap with memmove semantics. `tests/runtime/lg5_rt_copy_range_overlap`; `tests/must_reject/lg5_copy_range_overlap_oob` (`verify/index_oob`), `lg5_copy_range_self_idx_alias` (`type/inout_alias`). |
| G-LG3-4 | open | Not attempted: the 2 h bundle cap was used by the five items above and the audit fixes. |

Status: LG5: G-LG4-1 closed (array field lengths framed after `f(inout s)` unless the callee changes the field, both compilers).
Status: LG5: G-LG4-2 closed (`p.f = alloc(n, v)` takes the field element type, both compilers).
Status: LG5: G-DB4-1 closed (whole reassignment of `inout` array parameters, `&mut Vec<T>` / slot pointer, both backends).
Status: LG5: G-DB1-2 closed (scalar `i64`/`bool` `inout` on the LLVM backend).
Status: LG5: G-DB1-3 closed (`copy_range` with `src` equal to `dst`, memmove semantics, both backends).
Status: LG5: G-LG3-4 open (inout `Vec<T>` on LLVM not attempted in this bundle; time cap).

## New gaps

| Gap | What | Why not here |
|---|---|---|
| G-LG5-1 | Scalar `inout` of a struct field place (`f(inout c.n)` with `n: i64`) on the LLVM backend has no test. | The LG5 tests pass scalar locals only; the field-place pointer path was not checked within the time cap. |
| G-LG5-2 | A resizable `inout` array parameter passed from a struct field (`f(inout d.a)` where `f` reassigns `a`) has no runtime test on LLVM. | The LG5 tests pass array locals and parameters; the frame rule marks the field as changed, but the LLVM field-slot pointer was not exercised within the time cap. |

## Tests run

- LG5 tests with each compiler, both backends: runtime 24/24, must_reject 7/7, g5 must_reject 34/34.
- `--only g8_frontend`: Python 27/27, Rust 39/39. `cargo test --release`: 150 passed. `tests/diff_rust.py
  --fuzz-count 20`: all same.
- Python compiler full suite, both backends: `TOTAL 4653/4656`. Rust compiler full suite, both backends:
  `TOTAL 4665/4668`. The three failures in each were stale tests, fixed afterwards without compiler changes:
  - `g5_reject/g5_whole_param_assign` (two rows): the removed test was still listed in `tests/g5/fixtures.json`;
    the entry is removed. After the fix `--only g5_reject` gives 64/64 with both compilers.
  - `spec_trace`: `tests/spec/r6g_4_scalar_llvm` and `r6g_4_reject_unused_generic_scalar_llvm` expected
    `codegen/unsupported_type` for scalar `inout` on LLVM. They are now accept tests (the second renamed to
    `r6g_4_accept_unused_generic_scalar_llvm`; `docs/L3_RESTRUCTURE_SPEC.md` still names the old file and is outside
    this bundle). After the fix `--only spec_trace` passes with both compilers.
- `tests/run_all_rows.txt` regenerated with `--list-rows`: 31 new rows (all LG5 tests), 3 removed (the deleted
  `g5_whole_param_assign`).

## Delegations

- Research scan: Codex gpt-6.2-astra. Python compiler: Codex gpt-6.2-sol. Rust compiler: Codex gpt-6.2-sol. Tests and
  SPEC.md text: Claude sonnet. Bundle audit: Codex gpt-6.2-astra, ultra (items 1 and 3).
- Audit result: struct length frame has no miss. Three must-fix findings, all fixed in both compilers with tests:
  the LLVM `truncate` move emptied the caller's slot of a resizable parameter; the `par` branch finals ignored the
  generic instantiation; the new length of a resizable generic `[T]` had no bounds.
