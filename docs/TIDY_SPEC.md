# T1 tidy package: keep the compilers, the spec and the tests in step

Status: draft (2026-11-18). Adversarial review verdict: accept with changes. §4 lists the changes and
overrides §2 and §3 where they differ.

## 0. Why

Since 2026-11-17 the compilers grew by about 4,300 lines (pruvc 23,158, pruvc-rs 49,296 at 4126096) while
SPEC.md stayed behind. Three read-only surveys (external practice, spec/implementation match, cleanup
candidates) found:

- one real gap at the trusted boundary: the verifier assumes every array parameter has `len <= 2^40`
  (`pruvc/verify.py:2957`, symbolic.rs same), but the C adapters accept any `len <= isize::MAX / size_of<T>`
  (`pruvc/llvmgen.py:4567`, `4628`, `4904`; `pruv-core/src/bindings.rs:704`, `764`, `1008`). A host caller
  between the two bounds enters a proved body whose precondition does not hold;
- at least 7 features or policies missing from SPEC.md (`opaque fn`, `reveal`, `opaque_total`, trusted owned
  `String`/`Vec<i64>` results, fixed-size stack arrays, caller stack buffer for short `[i64]` results, unsat
  proof cache) and several text/implementation mismatches (`result` is listed as a keyword but is a legal
  name; §6 automatic body facts vs §6f "callers never see the body"; `proc_read`/`net_read` max is
  `1..=2^40`, the spec says "positive"; `proc_args_push` length precondition; `1__0` vs `1_`; inline policy
  wording; `pkg` and `codegen` missing from the §8 stage list; native `--emit exe|obj` is Rust-driver only);
- 71 of 148 literal diagnostic codes have no name in SPEC.md;
- 21 of 95 catalog operations are never called from any `tests/must_reject` input;
- about 50 lines of dead code, 6 hand-synced data lists between the two compilers, 8 files with clean
  single-responsibility splits, and the top-15 branchiest functions.

The package does not change what any accepted program means, except step T0 (the boundary fix).

## 1. Rules for every step

- Each step is one commit-sized change with its own tests, at most about one hour of work.
- Pure moves and dead-code removal must leave generated output byte-identical: `tests/g8` frozen hashes,
  `tests/diff_compilers.py`, `tests/diff_rust.py --fuzz-count 20`, and the groups the moved code serves.
- Python change and its Rust twin go in the same commit.
- Shared data is limited to declarations and tables (names, signatures, op names, limits). Proof rules stay
  written twice; two independent encodings of the semantics are what diff testing checks.
- No step depends on the budget package files (bench/l3/budget), so this package can run beside the budget
  lanes P18–P27. P16 and P17a touch the same compiler files as T5; run them after T5, or before T5 starts.

## 2. Steps

### T0 soundness first (new meaning, audited)

1. Host length bound equals the verifier bound. Every C adapter entry (slice in, new handle, state rebuild)
   rejects `len > 2^40` with the same failure path as a negative length, in both compilers. SPEC §ABI states
   the bound for C callers. Positive test: length exactly `2^40` is accepted by the check (no allocation;
   test the predicate). Negative test: `2^40 + 1` takes the failure path. Update g8 hashes on purpose and
   say so in the commit.
2. Vacuity check family. For each proof test group, a generated variant whose body ends in `assert false`
   under satisfiable preconditions must be rejected by both verifiers; preconditions that are contradictory
   are listed, not silently accepted. Start with a small fixed set (10–20 functions across groups) in
   `tests/vacuity/`.
3. Paired accept/reject from the spec. First pair: `old(a)` — an entry-value postcondition passes, the same
   postcondition written on the current value fails at the stated location. Reuse `bench/g5/faults`.

### T1 spec sync (docs + tests, no audit)

1. Add the 7 missing items to SPEC.md, each with its diagnostic code and one accept/one reject test link.
2. Fix the mismatches listed in §0; where text and both compilers disagree, the compilers win unless the
   behavior is a bug, which becomes its own T0-style item.
3. Give each SPEC rule touched in this package a stable ID (`R-<section>-<n>`). `tests/spec_trace.py` checks
   that every ID named in SPEC.md has at least one accept and one reject test that ran in both compilers,
   and that every ID a test names exists. Rules not yet touched are listed as untracked, not failed.
4. Diagnostic codes: SPEC.md gets a code table (code, stage, one-line meaning) generated from the code
   literals; `tests/spec_trace.py` fails on a code that is emitted but not in the table.

### T2 negative tests for the 21 catalog operations (tests only)

One `must_reject` input per operation whose reject reason is that operation's own precondition (not a
parse or type error). `rustc compiletest` style: the expected diagnostic code and line are checked.

### T3 dead code (pure)

`llvm_unsupported_error`, `g3_unimplemented_error` (both), `is_generic_program` (both), `is_param`,
`type_eq` (both), unused `path` in `_cycle_through`. Rust `pub` items are removed only after a workspace
grep shows no user; the Python `str` diagnostic in `typeinfo.py:376` stays (reachable with default `skip`).

### T4 shared tables (pure, output-identical)

One source of truth under `pruvc/data/` (JSON), read by Python at import and turned into checked-in Rust by
`tools/gen_shared.py`; `tests/check_generated.py` fails when the Rust file is stale. Bundles, in order:
inline constants, monomorphization limits, G6 shim helper edges, bit/word builtins, contract-only and
observer classes, then the 95-entry foreign catalog declarations (order preserved; contract ASTs excluded).

Also inside one compiler: Python bit builtin tables derive from `check.BIT_BUILTINS`; the verifier catalog
`need` helper is extracted once (`verify.py:2250,2339,2493`).

### T5 file splits (pure moves)

Split by one invariant each, in this order: check graph (py, rs), inline policy (py, rs), proof trace
(`verify.py` → `proof_trace.py`, re-exporting the exception classes), symbolic catalog I/O
(`symbolic.rs` → `symbolic/catalog_io.rs`), LLVM G9 helpers (py, rs). Each split keeps emission order and
re-exports moved names; one split per commit.

### T6 health tooling

`tools/health.py` prints the AGENTS.md light-check numbers (line counts, top files, proof time, rustc time
and binary size for budget m4, items slower than hand Rust (above 1.0x beyond measured noise) from PERF_HEADROOM) as one row for `docs/notes/health_log.md`.
Later the top-15 branchy functions get a short invariant note each (GHC "Note" style) before anyone edits
them; no rewrite in this package.

### Not in this package

- Freezing the Python compiler. Criteria (from TypeScript 7 and rustc NLL practice): every required case
  passes in Rust, every remaining difference has a spec reason, and an independent expected-result set
  exists for wrong proof acceptance (T0.2, T0.3, T1.3). Decide at R12/P36 close, not here.
- Proof-cost recording per obligation (Dafny/Verus style) and small differential execution of Rust vs LLVM
  backends (Csmith/YARPGen style): queued as T7/T8 after this package.

## 3. Order and check

T0.1 → T3 → T1 → T2 → T0.2 → T0.3 → T4 → T5 → T6. T0 is audited once (new meaning); the rest are proved
by byte-identical output and run by group, with the full test suite once before the last commit.

## 4. Changes from the adversarial review (binding)

1. **T0.1 is about every point where an outside buffer first enters proved code, not only C adapters.**
   Both verifiers assume `len <= 2^40` for every array parameter without a `requires`
   (`pruvc/verify.py:2957`, `pruv-proof/src/symbolic.rs:4291`), so a function can prove `result <= 2^40`.
   Entry points that need the check: public Rust functions in default/legacy output (`pruvc/codegen.py:1688`,
   `3243`), direct LLVM ABI `pruv_abi_*` and `__typed` symbols (`pruvc/llvmgen.py:4267`, `4350`, `5077`),
   C slices and every slice inside a batch item, handle create and state rebuild. `raw_entries="crate"` only
   narrows visibility; `"unsafe"` forwards to the same function. If a trusted path skips the check, SPEC
   states the caller obligation for every outside buffer and every `str` byte length; `--trust-callers`
   does not waive it silently.
2. **C bound is `min(2^40, isize::MAX / size_of<T>)` for Pruv values only.** Batch count `n` is an adapter
   loop count and gets no Pruv array bound. Pointer end, alignment and overlap checks stay. Add C tests:
   oversize input gives `null`/`BAD_ARGS`, mid-batch failure reports `done`, failed rebuild keeps the old
   state, poison precedence holds. The `2^40` positive case tests the predicate only. Foreign
   `String`/`Vec<i64>` results already have a bound check (`pruvc/foreign_codegen.py:627`).
3. **Keep the verifier's `2^40` model.** Check once at entry (resident state at create/replace), never
   inside internal calls. Measure the cost on the M4 raw, C and batch paths before and after.
4. **T0.2 becomes a reachability regression set.** An `assert false` at the end of a body is skipped after
   an earlier `return`. Insert it at the first statement of a function with a concrete input witness, or
   just before a return that input reaches; the original must pass first. Both compilers must reject with
   `verify/assert` at that line; a timeout, another diagnostic or a weaker counterexample is a failure.
   Intended contradictions are listed as their own cases; no extra SAT query in normal builds.
   T0.3 uses a relation that really changes (an increment before/after), not `a[k] == old(a)[k]`, which
   becomes an identity when `old` is removed.
5. **Expected results are fixed independently of the two compilers.** Do not bulk-update g8 hashes for C
   changes (`tests/g8/check_bundle.py:123` covers two scalar examples and one type error). `map_new` and
   `map_get` have no precondition to violate: total operations get true/false contract tests, partial ones
   get precondition-violation tests with code and line. Cache, inline and stack-buffer policies are linked
   to observable effects (cache hit, generated code), not to invented diagnostics. Diagnostic meanings and
   stages are written by hand, not only extracted from code literals.
6. **T4 shares only tables whose mistakes some test can catch.** Share: inline thresholds,
   monomorphization limits (they have boundary tests), names and display order; G6 helper edges only with
   an LLVM link test. Keep written twice: bit-name to Z3/Rust/LLVM operator maps, the contract observer
   classes, and the catalog `take`/effect/generic constraints. The 95 declarations are shared only after
   ownership, effect and type expectations are fixed independently.
7. **P16 and P17a run before T5. Python is not frozen.** Freezing now would lose the positive/negative
   proof comparison and LLVM output comparison for exactly those two features and would change P36's
   four-combination done condition; the cost of waiting is two Python changes and their tests. In T5 do
   the inline-policy extraction first; graph split only if both users gain; proof trace, symbolic catalog
   I/O and LLVM G9 moves are re-judged after the feature work.
8. **One-hour steps are cut by checkable result.** T0: ① C slices and batch items ② create/rebuild
   ③ direct LLVM ABI and typed ④ Rust public entries and raw contract ⑤ boundary perf measurement
   ⑥ reachability set and the `old` pair. T0 is done only after ①–⑤. Then T1 one rule group per step,
   T2 three to five operations sharing a precondition per step, T4 one table per step; T3 can merge alone.
9. T6 output includes the last three CI runs and the `work/` size and backup state.
10. **T0 follow-ups (queued at the T0 merge, 417dd56).** (a) Budget bench: the M4 bench adapter's allocations
    have no `2^40` bound, and m4 sessions are 1-2% slower than before T0; both go to L3_RESTRUCTURE_SPEC row
    P13f. (b) T0.4 (new meaning, audited): widen the verifier's `[i64]` length assumption from `2^40` to
    `2^60 - 1` so the host check can match `isize::MAX / 8`; T0 found `root_m4` then fails at `lit`,
    `tail_word` and `arena_put`, so this needs proof work in the budget sources first. (c) Two-compiler
    scope review: decide how much of the Python compiler to keep (reference, test oracle, or frozen), with
    an adversarial debate, before the R12/P36 freeze decision in "Not in this package".

11. **T2 follow-up (queued at the T2 merge, 95f734b).** The 13 total-operation rejects (`cd20`-`cd22`,
    `cd24`, `cd25`, `cd27`, `cd30`, `cd32`-`cd34`, `cd38`-`cd40`) compare an operation result with the same
    operation, so they check contract consistency only. Add paired accept tests on independent values
    (for example `ev_event_slot == -1` right after `ev_new(1)`) so a wrong state-field link is caught. Also run
    the satisfied-precondition accept twin for `ev_proc_write`, `ev_proc_close_stdin`, `ev_proc_wait`.

12. **T4 follow-up (queued at the T4 merge, 4b8be7f).** T4 shared four tables (`pruvc/data/`: inline policy,
    monomorphization limits, builtin type names, G6 shim edges) and derived the Python bit tables from
    `check.BIT_BUILTINS`. Not yet shared across the two compilers: bit/word builtins, contract-only and
    observer classes, and the 95-entry foreign catalog declarations. The shared values also have no boundary
    tests: add accept/reject pairs at `MAX_TYPE_APPS`/`MAX_TYPE_NODES` and at the inline thresholds (8/9 and
    1500/1501), on both compilers, so a wrong value in the JSON is caught.

Order after the review: T0①–⑥ → T3 → T1 → T2 → T4 → P16, P17a → T5 (inline policy) → T6.
