# L3 LG8: inout Vec on LLVM, moves out of match binders and record fields, copy of a subview

Scope: G-LG7-1, G-DB16-1, G-DB16-2, G-R09-4 and (time permitting) G-R06-1 from `docs/L3_RESTRUCTURE_SPEC.md`
§7.5.1, plus the follow-up question whether G-R09-3 is closed. Both compilers (Python `pruvc/`, Rust `pruvc-rs/`)
change with the same semantics and diagnostics, on both backends (Rust source and LLVM). No `bench/` fragment is
changed; G-LG6-2, G-R03D-2, G-LG3-3 and G-DB6-1 are out of scope.

## What landed

- `inout Vec<T>` parameters on LLVM (G-LG7-1). An internal function's `inout` scalar-element `Vec<T>` parameter
  (`Vec<i64>`, `Vec<u8>`, `Vec<bool>`, ...) is now accepted by the LLVM backend; it was rejected before. The caller
  passes only the `ptr` of its Vec slot (the opaque handle cell), no separate `i64` length, so `vec_push`,
  `vec_reserve`, `vec_set` and friends in the callee act on the caller's Vec. Host-entry (`#[export]`) functions keep
  the old rule. Code: `pruvc/llvmgen.py` (inout reject exemption), `bindings.rs`, `llvmgen.rs` `ev_call_inner`.
  The old rejection test `tests/must_reject/lg3_inout_vec_reserve` is removed (the program is now valid).
- Moving an array out of a match binder (G-DB16-1, SPEC.md R-6e-12a). `let t = move b;` of an array binder `b` of a
  user-enum arm is allowed when the scrutinee is a plain owned local (not a parameter) of a non-generic enum that
  no arm names. A match where some arm moves a binder takes the scrutinee by value: it is moved on entry to every
  arm and after the match, and the arm's array binders are owned bindings. A later read is `type/foreign_move`
  ("use after move" / "binding may have been moved"); in a loop the scrutinee must be live again
  (`type/foreign_loop_move`). Every other
  binder move stays `type/array_alias`. Rust backend: the match takes the scrutinee by value (also under cost
  metering) so the binder is an owned `Vec<T>`. LLVM: the binder's `{ptr, len, cap}` is taken and the empty array is
  stored into that field of the scrutinee's slot, so the scope-end drop does not free the moved buffer.
- `copy(subview(a, lo, hi))` (G-DB16-2, SPEC.md R-6g-6 "Exception (LG8)"). A subview may be the only argument of the
  builtin `copy`; the result is a fresh owned array of `hi - lo` elements. The bounds obligation is that of
  `subview` (`hi > len(a)` is `verify/index_oob`). Rust backend emits `(<subview>).to_vec()`; LLVM needed no change.
- Consuming field move (G-R09-4, SPEC.md R-6e-16 consuming paragraph). `let t: T = move d.f;` of a buffer field
  (`[i64]`, `[u8]`, `[bool]`) with one field step and no restore in the next statement moves `d` wholly when `d` is a
  `let mut` local record (a field move from a `take` record parameter or a match binder, also a `map_take` binder, is
  `type/foreign_move`). A later use of `d`, also in an `assert`,
  invariant or `decreases`, is `type/foreign_move`; assigning `d` again revives it. With a restore in
  the next statement the old form is unchanged. `String` / `Vec` fields, nested paths, `inout` and plain roots,
  `p.g = move p.f;` and `f(move d.a)` keep their errors. `tests/must_reject/lg4_move_array_field_no_restore` now
  rejects at the later use (`type/foreign_move`, line 7) instead of at the missing restore.
- Whole reassignment of an `inout` array parameter (G-R06-1). Already allowed since LG5 (resizable `inout`); the
  SPEC §6g text still said `len(a) == len(old(a))` always holds. The `old`, frame and loop paragraphs now say that
  holds only for a fixed-length `inout` parameter, and a resizable one relates the lengths only through `ensures`.
- Rust move tracker parity fix: `foreign.rs` now tracks every non-generic struct and enum local (Python tracks every
  non-generic struct), and `expr_o` checks the liveness of every tracked local on a read, like Python's `use` on every
  `Var`. Without it the Rust compiler accepted the new use-after-move programs.
- Audit fixes (see Tests run): a binder moved in one branch of an arm and read after it (M1), a field move in one
  branch (M2), a consumed record read in an `assert` (M3), the scrutinee read after an arm that moves a binder and
  returns early (M4), and the consuming field move from a `take` record parameter, whose LLVM pointer is read-only
  (M6), and the consuming field move from a `map_take` match binder (recheck), are now rejected in both compilers.
  Tests: `tests/must_reject/lg8_match_move_branch_binder`, `lg8_match_move_early_return`,
  `lg8_move_field_consume_branch`, `lg8_move_field_consume_assert`, `lg8_move_field_take_param`,
  `lg8_move_field_map_take_binder`.

Generated code of existing programs is unchanged: the old (HEAD) and new Python compiler give the same Rust source,
LLVM IR and bindings for all 1009 `.pruv` files under `examples/`, `bench/` and `tests/runtime/` (LG8 tests
excluded; 813 compile, the rest fail the same way in both). Python and Rust output is equal by the diff tools below.

| Row | Status | What exists now |
|---|---|---|
| G-LG7-1 | closed | Internal `inout Vec<T>` on LLVM in both compilers; `tests/runtime/lg8_rt_inout_vec` (both backends). |
| G-DB16-1 | closed | R-6e-12a in both compilers and backends; `tests/runtime/lg8_rt_match_move`, `tests/must_reject/lg8_match_move_{use_after,borrowed,branch_binder,early_return}`. |
| G-DB16-2 | closed | `copy(subview(...))` in both compilers and backends; `tests/runtime/lg8_rt_copy_subview`, `tests/must_reject/lg8_copy_subview_hi_gt_len`. |
| G-R09-4 | partial | Consuming form for buffer fields of a `let mut` local record; `tests/runtime/lg8_rt_move_field_consume`, `tests/must_reject/lg8_move_field_consume_{use_after,branch,assert}`, `lg8_move_field_{take_param,map_take_binder}`. `String`/`Vec` fields and `f(move d.a)` remain (G-LG8-1). |
| G-R06-1 | closed | Allowed since LG5; SPEC §6g length wording fixed. No new test (LG5 tests cover the reassignment). |
| G-R09-3 | partial | The compiler side (`vec_reserve` on a passed `Vec` on LLVM) now works; the bench `line` buffer is still a fixed `[u8]` (G-LG8-2). |

Status: LG8: G-LG7-1 closed (internal `inout Vec<T>` on LLVM passes only the handle ptr; vec_push/vec_reserve in the callee act on the caller's Vec).
Status: LG8: G-DB16-1 closed (move of an array binder from a match on an owned local enum consumes the scrutinee; later use is type/foreign_move).
Status: LG8: G-DB16-2 closed (`copy(subview(a, lo, hi))` gives a fresh owned array; bounds obligation of subview).
Status: LG8: G-R09-4 partial (`let t = move d.f` of a buffer field with no restore consumes a `let mut` local record; String/Vec fields and `f(move d.a)` open as G-LG8-1).
Status: LG8: G-R06-1 closed (whole reassignment of an `inout` array was allowed since LG5; SPEC §6g length wording now limits `len(a) == len(old(a))` to fixed-length parameters).
Status: LG8: G-R09-3 partial (compiler support for vec_reserve on a passed Vec on LLVM landed; the bench line buffer is still a fixed [u8], bench out of scope, see G-LG8-2).

## New gaps

| Gap | What | Why not here |
|---|---|---|
| G-LG8-1 | Consuming field move for `String` / `Vec` fields, nested paths, and as a call argument `f(move d.a)`. | Not needed by the DB16 rows; the brief asked for array fields; kept the existing restore rule. |
| G-LG8-2 | Rest of G-R09-3: switch the bench `line` buffer from a fixed `[u8]` (`len(line) == cap`) to a `Vec<u8>` grown with `vec_reserve`. | `bench/` is out of scope for LG8. |
| G-LG8-3 | Consuming field move from a `take` record parameter (`take p: Packet` with a `[u8]` field). Rejected on purpose: a field move from it is `type/foreign_move` even with a restore (`tests/must_reject/lg8_move_field_take_param`). | On LLVM the `take` record is passed as a read-only pointer to the caller's slot, so emptying the field there is not allowed; needs a by-value or writable `take` record ABI first. |
| G-LG8-4 | `docs/L3_LG3.md:30` still cites the removed test `tests/must_reject/lg3_inout_vec_reserve`. | That document is outside the files this lane may edit. |
| G-LG8-5 | LLVM caller of a `take` record parameter whose record has a buffer field: the caller passes its slot address after nulling the first word (pre-existing, `pruvc/llvmgen.py` ~2433/2886, `llvmgen.rs` ~3675/4339). Found by the LG8 audit (static reading, M7); no failing program was run. | Pre-existing `take` record ABI, not part of the LG8 rows; changing it alters existing generated code. |

## Tests run

- Full suite, `tests/run_all.py --backend both`: Rust compiler 4737/4737, Python compiler 4725/4725 (after the last
  shared-code change; the Python compiler has fewer rows, e.g. g8_frontend 27 vs 39).
- `--only g8_frontend --backend both`: Python 27/27, Rust 39/39.
- LG rows on both compilers (`--sample-prefix tests/runtime/lg`, `tests/must_reject/lg`): runtime 100/100 each,
  must_reject 56/56 each.
- `cargo test --release`: 150 passed, 0 failed.
- `tests/diff_rust.py --fuzz-count 20`: 25415 cases, all same. `tests/diff_llvm.py`: 27071 cases, all same (Python and
  Rust LLVM IR byte-identical). `tests/diff_compilers.py --backend both`: 4702 cases, all same (52 differ only in
  counterexample values).
- Rows list regenerated (`--list-rows`): 4712 -> 4737 rows; the diff is only the 4 new LG8 runtime tests (16 rows),
  the 10 new LG8 must_reject tests and the removed `lg3_inout_vec_reserve`.
- Audit (read-only): one pass by Codex gpt-6.2-astra ultra with 7 must-fix findings (M1-M4 and M6 fixed with
  tests, M5 fixed by SPEC wording, M7 recorded as G-LG8-5); one recheck: M1-M6 fixed, plus 2 new items (a `map_take`
  binder passed the `let mut` limit; SPEC R-6e-16 `take` wording), both fixed (`lg8_move_field_map_take_binder`).
