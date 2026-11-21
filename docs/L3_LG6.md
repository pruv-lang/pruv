# L3 LG6: struct field `inout` places on LLVM, borrowed slice arguments (`subview`)

Scope: G-LG5-1 and G-LG5-2 from `docs/L3_LG5.md`; the borrowed slice parameter asked for by G-R04-2, G-R06-2,
G-DB6-2, G-R05-5 and G-DB7-1 (`docs/L3_RESTRUCTURE_SPEC.md` §7.5 / §7.5.1); G-LG3-4 if time allowed (it did not).
Both compilers (Python `pruvc/`, Rust `pruvc-rs/`) change with the same semantics and diagnostics, on both backends
(Rust source and LLVM). No `bench/` or `pkgs/` fragment is changed; G-R03D-2 and G-LG3-3 are out of scope.

## What landed

- Field places as `inout` arguments (G-LG5-1, G-LG5-2, SPEC.md §6g "Field places (LG6)"). `f(inout c.n)` and
  `f(inout d.c.n)` with a scalar field, and `f(inout d.a)` where `f` reassigns the array, run on both backends with
  both compilers. No compiler change was needed; the tests close the rows. Without a length clause in `f`'s
  `ensures` the caller cannot index the reassigned field (`verify/index_oob`).
- Borrowed slice arguments `subview(b, lo, hi)` (SPEC.md §6g2, R-6g-5 to R-6g-9). A read-only window `b[lo..hi]` is
  passed to a non-`inout` array parameter of a Pruv function without a copy. There is no slice type and no lifetime
  syntax: `subview` may appear only as a direct call argument (elsewhere `type/view_context`, new), so it cannot be
  stored, returned or bound. The callee is an ordinary array-parameter function; the existing borrowed-parameter rules
  stop it from writing (`type/immutable`), returning or storing (`type/array_alias`) or pushing (`type/foreign_owned_type`)
  the view. `subview(b, ...)` counts as a use of `b` (and of a struct holding it) for `type/inout_alias`; a `par` branch
  argument cannot be a view (`type/parallel_argument`).
  - Verifier: one obligation `0 <= lo <= hi <= len(b)` (`verify/index_oob`, "view range ...") before the callee's
    `requires`; the argument is then exactly `len(v) == hi - lo`, `v[p] == b[lo + p]` (array lambda).
  - Rust backend: `unsafe { b.get_unchecked(lo..hi) }` when verified; under `--no-verify` a checked range that calls
    `pruv_oob`.
  - LLVM backend: `sub hi, lo` and `getelementptr inbounds <elem>, ptr b, lo`, passed in the existing `{ptr, len}`
    array-parameter ABI; under `--no-verify` an `icmp` range check and `@pruv_panic_oob` before the GEP. No allocation
    or copy in the caller; the callee's proved index checks stay removed. Python and Rust LLVM IR are byte-identical.
  - The fragments named by the rows are not changed (out of scope for a language bundle); they can now replace their
    sub-range copies with `subview`.

| Row | Status | What exists now |
|---|---|---|
| G-LG5-1 | closed | `tests/runtime/lg6_rt_field_inout` (`bump(inout c.n)`, `bump(inout d.k)`, `bump(inout d.c.n)`, both backends, proof and `--no-verify`). |
| G-LG5-2 | closed | `tests/runtime/lg6_rt_field_inout` (`grow(inout d.a)` reassigning the field, and through `grow_in(inout d)`); `tests/must_reject/lg6_field_inout_reassign_len` (`verify/index_oob` without a length clause). |
| G-R04-2 | partial | Language has `subview`; fragment not changed. |
| G-R06-2 | partial | Language has `subview`; fragment not changed. |
| G-DB6-2 | partial | Language has `subview`; fragment not changed. |
| G-R05-5 | partial | Language has `subview`; fragment not changed. |
| G-DB7-1 | partial | Language has `subview`; fragment not changed. |
| G-LG3-4 | open | Not attempted: the 2 h bundle cap went to `subview` and its audit. |

Status: LG6: G-LG5-1 closed (scalar `inout` of a struct field place runs on LLVM and Rust, both compilers; test only, no compiler change).
Status: LG6: G-LG5-2 closed (resizable `inout` array passed from a struct field, `f(inout d.a)`, runs on both backends; test only).
Status: LG6: G-LG3-4 open (inout `Vec<T>` on LLVM not attempted in this bundle; time cap).
Status: LG6: G-R04-2 partial: language has the view; fragment not changed.
Status: LG6: G-R06-2 partial: language has the view; fragment not changed.
Status: LG6: G-DB6-2 partial: language has the view; fragment not changed.
Status: LG6: G-R05-5 partial: language has the view; fragment not changed.
Status: LG6: G-DB7-1 partial: language has the view; fragment not changed.

## New gaps

| Gap | What | Why not here |
|---|---|---|
| G-LG6-1 | `inout Vec<T>` parameters on the LLVM backend (carried from G-LG3-4). | Not attempted within the 2 h cap. |
| G-LG6-2 | The fragments of G-R04-2, G-R06-2, G-DB6-2, G-R05-5 and G-DB7-1 still copy sub-ranges; they should pass `subview(...)` and be re-measured. | Fragment rewrites are outside this language bundle (the brief forbids them). |

## Tests run

New tests: `tests/runtime/lg6_rt_field_inout`, `tests/runtime/lg6_rt_view` (4 rows each: both backends, proof and
`--no-verify`); 19 `tests/must_reject/lg6_*` (one field-place, 18 view); 11 `tests/spec/r6g_{5..9}_view_*` (accept and
reject for each of R-6g-5 to R-6g-9). `tests/run_all_rows.txt` gains these 27 rows and loses none.

- lg6 groups, both compilers, both backends: runtime 8/8, must_reject 19/19.
- `--only g8_frontend --backend both`: Python 27/27, Rust 39/39.
- `cargo test --release`: 150 passed, 0 failed.
- `tests/diff_rust.py --fuzz-count 20`, `tests/diff_llvm.py`: all same.
- `tests/diff_compilers.py --backend both`: 4642 same, 0 diff (50 differ only in counterexample values).
- Full suite (`tests/run_all.py --backend both`): Python 4681/4681, Rust 4693/4693 (after the `subview` rename).

## Delegations

- Design, SPEC text, item 1 tests, spec tests, the soundness probes and the two audit fixes: Claude Opus 5.5 (this
  conversation).
- `subview` implementation in both compilers, the runtime test and the first must_reject set: Codex gpt-6.2-sol (write
  mode, applied after review).
- Release build: a background shell command.
- Audit (one pass): Codex gpt-6.2-astra, effort ultra. Two must-fix findings: a temporary array source
  (`subview(copy(b), ..)`, which copied) and a `subview` nested inside another view's operands through a call. Both fixed in
  both compilers (`type/view_context`; tests `lg6_view_temp_source`, `lg6_view_nested_call`).
- Recheck of the two fixes: Codex gpt-6.2-astra, effort ultra. Both fixes hold; one more must-fix: with a `subview` nested in the
  source operand (`subview(subview(b, 0, 0), 0, 0)`) the error pointed at the outer view. Fixed in both compilers (the source
  is checked with the nested-view guard on before the outer error); test `lg6_view_nested_source`. Audit budget used
  (one audit, one recheck), so this last fix was not audited again.
- Rename: the builtin was first called `view`. The first full suites failed one row in each compiler
  (`g6_proof/p12_string_to_str` defines `fn view`, so it hit `type/reserved`), and `bench/l3/budget/bud_lifecycle.pruv`
  and `bud_tlife.pruv` also define `pub fn view`. The builtin was renamed to `subview` (no program in the repository
  uses that name). The renamed code was not audited again; the checks above were rerun after the rename.
