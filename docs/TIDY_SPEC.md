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

Status (2026-11-19, on 148de2b):

- Done: `tools/health.py` (standard library only). Default (fast, ~1-2 s) prints one row in the health log
  columns: tracked line counts of `pruvc/*.py` and `pruvc-rs/*.rs` with the delta to the last state, top 5
  files, every PERF_HEADROOM summary item above 1.0x hand (no tolerance band; noise is a person's call from
  interleaved runs), last 3 CI runs on main, `work/` size and backup state (newest file in `work/` vs the last
  `../pruv-internal` commit; `backup.sh` is never run), disk free and Pruv folder size (§4 item 9).
  `--full` (~11 s) builds budget m4 through `m34_test/run.py build()` in a temp dir with pruvc and rustc under
  `/usr/bin/time` (proof time, compiler + in-process z3 peak RSS, rustc time and peak RSS, binary size), then
  runs the M4 driver against `m4_hand` (n = 1000, 3 alternating rounds) for time, peak RSS and allocation
  count (the counting global allocators already in `driver_m4.rs` and `m4_hand.rs`). `--save` appends to
  `work/health_state.json` in the main checkout; `--compare` prints only the lines past their thresholds
  (+1,000 lines per compiler, +20% for m4 times, sizes and memory, any item above 1.0x hand, failed CI,
  `work/` changed after backup, disk under 20 GB) against the last state, falling back to the line counts of
  the last `health_log.md` row. Unit test: `tests/health_tool.py` (not a `run_all.py` row).
- Done (list only): `--branchy` ranks the 15 functions with the most branch points across both compilers
  (Python by AST; Rust by a token count inside the fn braces, closures included). No invariant notes yet.
- Open: the invariant notes on those 15 functions.

**Merge gates (§4 item 15).** Two standard-library scripts turn merge rules into checks that exit 1.
`python3 tools/merge_check.py <branch>`, run on the main checkout before a merge: fails if the newest completed
push CI run on main failed (schedule runs are printed only), prints `git diff --stat main...<branch>`, and if the
branch touches `tests/` rebuilds the row list from the branch tree in a temporary worktree
(`tests/run_all.py --backend both --list-rows`) and fails if it differs from the branch's
`tests/run_all_rows.txt`; for code generation or `pruvc-rs/` changes it prints the g8_frontend, `cargo test`
and `diff_rust` reminders without running them. `python3 tools/perf_gate.py <perf_cells.json>...` applies the
perf rule: hand Rust 1.0x is the floor, only proportional cells at sizes M and L above 1.0x fail; fixed-share
and S cells above 1.0x are listed as debt for L3_RESTRUCTURE_SPEC §7.5; there is no noise band. New packages
write their measurements as `perf_cells.json` (a list of cells `name`, `axis` time|rss|allocs, `size` S|M|L,
`kind` proportional|fixed, `pruv`, `hand`, `unit`, optional `runs`); old slices are not converted. Example:
`tools/testdata/perf_cells_p31.json` (P31 M10b decide, §7.5 D28-D30).

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
    - Done: 16 accept tests `t2_cdNN_*.pruv` in the g6, g10, g10_proc, g10_net and g10_async proof groups.
      Each was checked against one scratch contract mutation (never committed), and every test failed under
      its mutation on both compilers. The mutations covered three kinds of wrong link: an observer reading
      the wrong state field, `map_new` size 1, `map_get` skipping the domain check, `string_to_str` byte
      and length, and `proc_args_len` returning 0; the `ev_proc_*` slot kind changed from 3 to 1; and for
      file handles, the open/create `ensures` linking `ok` to the other flag. Limits: a file observer
      swapped to the other field cannot be seen, because file fields are only defined through those
      observers in the catalog `ensures`, so the `ensures` link is the testable one. An `ev_exit` →
      `error` swap is not caught by any accept test, because `exit` is provably nonzero only where `ev_ok`
      also holds; `t2_cd40` catches `exit` → `ev_slot`/`last` instead.

12. **T4 follow-up (queued at the T4 merge, 4b8be7f).** T4 shared four tables (`pruvc/data/`: inline policy,
    monomorphization limits, builtin type names, G6 shim edges) and derived the Python bit tables from
    `check.BIT_BUILTINS`. Not yet shared across the two compilers: bit/word builtins, contract-only and
    observer classes, and the 95-entry foreign catalog declarations (open; touches both compilers). Boundary
    tests for the shared values: done. Group `codegen_boundaries` (`tests/check_codegen_boundaries.py`, both
    compilers, `--no-verify` Rust output) compiles generated sources at each edge with the spec numbers
    hard-coded: `probe` gets `#[inline(always)]` at 8 lines (189 sites, so only `INLINE_SMALL` applies) and
    `#[inline]` at 9; `#[inline(always)]` at 1500 lines × 2 sites and `#[inline]` at 1501; 4096 distinct `Tag`
    applications are emitted and 4097 reject with `codegen/specialization_limit` ("more than 4096 distinct
    generic type applications"); a 65536-node type argument is accepted and 65537 rejects ("more than 65536
    nodes"). Mutation check: each of the four JSON values changed by +1 and by -1 (Rust side regenerated with
    `tools/gen_shared.py` and rebuilt) fails the matching pair on py and rs (16/16 runs).
13. **P18b follow-up (queued at the P18b merge, f0c5932; soundness).** A host that `include!`s a generated file
    at its crate root can call the unchecked inner bodies (`f__pruv_body`) directly and skip the entry checks
    (T0 length bound, `requires`). It predates P18b (a6fb98e) and `--proven-entries` did not widen it; SPEC
    states the guarantee only for hosts that attach the file as a `#[path] mod`. Fix: emit the inner bodies
    in a private module (or with private visibility the host cannot name) so only checked entries and
    `pruv_proven::*` are reachable; every generated file changes, so run it as its own step with
    `check_generated`, the 7 budget roots and a must-reject host test that names an inner body.
    Done (TIDY 4.13): both compilers wrap every Rust-backend output, last, as `mod pruv_impl { … }` plus
    `pub use self::pruv_impl::*;`, so a host reaches only `pub` items (checked entries, `pruv_proven::*`,
    public types) and, inside its crate, the `pub(crate)` raw entries. Naming a body fails with E0425
    (unqualified, or `gen::f__pruv_body` from a `#[path] mod`) or E0603 (through `pruv_impl::`). The audit found
    a second, older bypass: a host `macro_rules! panic` defined before the file is in textual scope inside it,
    and one that returns lets a failed `requires` run the body (reproduced: `g(-1)` returned 7). The module now
    defines `macro_rules!` forwarders for the 8 macro names the generator calls unqualified, which shadow host
    ones; they go to `core::` (`std::` only for `eprintln`, `println`, `vec`) through the extern prelude, so a
    `no_std` host still compiles (the confirm audit caught `::std::` breaking one) and edition 2015 works. Limit: one generated file per crate-root `include!` (fixed module name); several attach as
    `#[path] mod`s. `tests/l3/abi/check_proven_entries.py` adds 4 must-reject hosts (`include!` and
    `#[path] mod`, unqualified and through `pruv_impl`) × plain/`--proven-entries` × both drivers (16 rejects,
    each must be a rustc error with the expected code whose primary span is in the host file), 2 macro-shadow
    hosts per run (8 runs; `guarded(-1)`, whose body safely returns 7, must still panic with the `requires` message) and a `#[path] mod` accept
    host calling the checked entry and `pruv_proven::*`. Against the base compilers only the unqualified
    `include!` host is a real bypass (it built); `gen::f__pruv_body` was already E0603 and the two
    `pruv_impl::` hosts name a module that did not exist. py/rs output is byte-identical; `bridge/node/gen`
    regenerated (+20 lines each); g8 rust hashes updated after checking new == wrap(base). Gates on both
    compilers: g8_frontend, check_generated, l3_abi, the 7 budget roots, M4 300, M7 q, M11 Q6, `cargo test`,
    `diff_rust` fuzz 20; run_all rows unchanged. Perf (budget m4, base vs new, 5 alternating rounds):
    instructions and peak RSS distributions overlap on all three axes, `__text` the same size (the bytes
    differ by layout). The LLVM backend was already closed (`define internal` bodies).
    `bench/s5/perf/*_rust_*.rs` are frozen S5 records and were not regenerated.
14. **Item 10(c) decision (two-compiler scope, adversarial debate 2026-11-19).** Decision: option ② (Rust
    compiler first; Python kept as the reference and proof-differential oracle), effective only after the
    R12/P36 close. Until then option ① stays: both compilers are full peers and every change is ported in the
    same package. Evidence: 17 of the last 20 compiler-touching commits changed both compilers; `diff_rust`
    caught a real G6 false reject; both verifiers shared the 2^40 assumption, so differential testing alone
    misses shared mistakes. Follow-up rows:
    (a) Before R12: make the Rust verifier judge independent expected results directly (g3_proof on Rust,
        `spec_trace --only-rs`, `vacuity --compiler rs`, the 19 assert-false points, 3 contradictions, old
        pairs), reporting false reject, false accept, wrong location and timeout separately.
    (b) Before R12: fill the independent-expectation gaps (item 11 style accept twins) and measure the porting
        cost per change.
    (c) At the R12/P36 close, the ①→② gate: all required cases pass on Rust, every remaining difference has a
        SPEC basis, an expected-result set exists that alone catches false proof acceptance, and the 4 combos
        x 2000 baseline is recorded. Otherwise keep ①.
    (d) After the gate: change the README policy and the CI compare scope together. New proof rules are still
        written in both compilers; output/linker features close through spec execution values and perf. Keep
        the corpus, the shared JSON (`pruvc/data/`) and `pruvc/runtime.c`.

    **Result of (a) (2026-11-19, tests only, no compiler change).** Switches added: `g3_proof` with
    `--compiler rs` now compiles each file with the selected Rust compiler (verification on, `--backend rust`;
    before, both `--compiler` values ran the Python verifier through `tests/g3/verify_only.py`, which the py side
    still uses because the Python code generator rejects generics); new `run_all` group `vacuity` (one row,
    `tests/vacuity/run.py --compiler py|rs`; the `g3_proof` rows keep their key, so `run_all_rows.txt` only gains
    that row); `tests/vacuity/run.py --kinds reach,contradiction,pair`; both
    `vacuity/run.py` and `spec_trace.py` now put every failure in one class (`false_reject`, `false_accept`,
    `wrong_location`, `timeout`, `other`) and print per-compiler `CLASSES` counts (`vacuity` also per-set `SET`
    lines); `spec_trace` checks `line`/`col` when an expectation JSON gives them. Expected values unchanged.
    Rust results (release `pruvc` built from b2fb727, the last pruvc-rs commit), wall time per run on this machine:

    | set | cases | pass | false reject | false accept | wrong location | timeout | other | wall |
    |---|---|---|---|---|---|---|---|---|
    | `g3_proof` (`run_all --only g3_proof --compiler rs`) | 24 | 24 | 0 | 0 | 0 | 0 | 0 | 0.17 s |
    | `spec_trace --only-rs` (32 tests, 228 checks) | 32 | 32 | 0 | 0 | 0 | 0 | 0 | 0.67 s |
    | `vacuity` reach (19 `assert false` points) | 19 | 19 | 0 | 0 | 0 | 0 | 0 | 0.99 s |
    | `vacuity` contradiction | 3 | 3 | 0 | 0 | 0 | 0 | 0 | 0.11 s |
    | `vacuity` pair (`old` accept/reject) | 2 | 2 | 0 | 0 | 0 | 0 | 0 | 0.08 s |

    Whole `vacuity` group on rs: 1.24 s. The py side is unchanged: `g3_proof` 24/24, `spec_trace` (both
    compilers) 260/260, `vacuity --compiler py` 24/24 with all classes 0. No disagreement between Rust and an
    independent expected result was found. Classifier check (scratch, not committed): the Rust compiler wrapped
    with `--no-verify` gives `false_accept` 19/19 on reach, 1 on the pair, 7 on `spec_trace` (its 7 verify-stage
    rejects), and contradictions still pass, as they must.
    Gaps found (queued follow-ups, no fix here):
    - **Old-pair reject location.** `tests/vacuity/old_increment_reject.pruv` fails `verify/ensures` at line 2
      col 1 (the `fn` header) on both compilers; only the message names line 4 (the `ensures`). `cases.json`
      checks the message, so a wrong structured location is not caught. SPEC (§diagnostic table,
      `verify/ensures`) states no location for an `ensures` that fails at a fall-through end of the body, while
      an explicit `return` reports the `return` line (`tests/must_reject/cd20_map_new.json`, line 4). Follow-up:
      write the location rule in SPEC by hand, then put `line`/`col` in the pair expectation.
    - **Sets that cannot see a false accept.** `g3_proof` has only accept cases and the 3 contradictions expect
      accept on both sides (they passed under `--no-verify`). Only reach, the reject half of the pair and the 7
      verify-stage `spec_trace` rejects catch false proof acceptance on generics-free code; reject twins for G3
      belong to (b).
    - **No structured location in `spec_trace` expectations.** None of the 32 JSONs gives `line`/`col`; adding
      them (hand-fixed from SPEC) is part of (b).

    **Result of (b) (2026-11-19, tests and one SPEC sentence, no compiler change).** All three gaps above are
    closed. Expected values were written by hand from SPEC, never copied from compiler output.
    - **G3 reject twins.** `tests/g3/reject/pNN_*.pruv` + `.json`, one per `tests/g3/proof` file (24). Each twin
      differs from its original by exactly one code line (checked with `diff`; line 1/2 comments aside), keeps
      the line numbers, and names the expected diagnostic: 20 `verify/ensures` at the failing `return`
      (line and the column of `return`), plus p07 `alloc_negative` (lower bound on `n` dropped, line 7), p11
      `call_requires` (callee needs `len(a) > 1`, 14:10), p15 `overflow` (upper bound on `t.tag` dropped,
      line 20) and p16 `invariant_preserved` (`out[i] = a[0]`, no location: SPEC gives none for it). They run
      in group `g3_proof` as mode `verify_reject` (same compile path as the accept rows), classed
      `false_accept` / `wrong_location` / `timeout` / `other`. Result: `g3_proof` 48/48 on py and on rs.
      Always-accept check (scratch, not committed): Rust wrapped with `--no-verify`, and `verify_only.py` with
      its verification errors dropped, each give 24/24 reject rows `false_accept` while the 24 accept rows
      still pass (24/48 on both). So `g3_proof` alone now catches a verifier that accepts everything.
    - **Rule R-7-6 (SPEC §7).** A `verify/ensures` diagnostic for a written `ensures` points to the `return`
      keyword on a path that returns, and to the `fn` keyword on a path that reaches the end of a `()` body
      (synthesized refinement `ensures` and `verify_unknown` keep their own rules). This is
      what both compilers already do and what `tests/must_reject/l3_hf_frame.json` (11:1) and the 20 `return`
      cases in `must_reject` expect, so no compiler change and no follow-up row. Tests:
      `tests/spec/r7_6_ensures_fallthrough_ok.pruv` (accept), `r7_6_ensures_fallthrough` (reject, `pub fn`, 3:5 = the `fn` keyword),
      `r7_6_ensures_return` (reject, 7:3). Audit (Codex gpt-6.2-astra ultra): two required
      wording fixes (scope to written `ensures`; name the `fn` keyword, since `pub fn` starts earlier), applied. The old pair now checks it: `cases.json` `old_increment_reject`
      expects 2:1, and `tests/vacuity/run.py` compares `line`/`col` for a pair reject when the case gives them.
    - **`spec_trace` locations.** Line and column where SPEC fixes the point: `r1_1_result_scope` 4:12,
      `r1_2_trailing_underscore` 3:24, `r6l_1_read_max_big` 5:10, `r6l_1_read_max_zero` 5:10,
      `r6l_2_args_push_unbounded` 3:10, `r6m_1_read_max_zero` 5:10; line only (the construct is fixed, the
      column is not): `r6o_2_no_reveal` 8, `r6o_3_reveal_target` 8; plus the two R-7-6 rejects. Left without a
      location: `r6_1_body_facts_requires` (SPEC does not say whether the call on line 7 or the `assert` on
      line 8 carries it), `r6d_1_array_result`, `r6o_1_opaque_i64`, `r6o_4_total`, `r7_5_emit_exe_entry`,
      `r8_1_codegen` (SPEC names no location). `spec_trace` (both compilers): 35 tests, 271/271, all classes 0.
      `vacuity` 1/1 on py and on rs.
    - **Porting cost per change.** The last 30 non-merge commits touching `pruvc/` or `pruvc-rs/`
      (b2fb727..9147ae4), lines added + removed under `pruvc/` and under `pruvc-rs/` (`git show --numstat`):

      | | commits | Python lines (total / median) | Rust lines (total / median) |
      |---|---|---|---|
      | both compilers | 27 | 2935 / 63 | 4939 / 147 |
      | Python only | 2 (bdfcd54, 0ff7795) | — | — |
      | Rust only | 1 (e0690f0) | — | — |

      Over the 27 shared changes the Rust side is 1.43x the Python side per commit (median) and 1.68x in total;
      the shared `pruvc/data/` tables were touched by 4 commits, 1 line each. Spot checks by hand: ca6cba2 is
      8 files / 190 lines Python vs 19 / 563 Rust, 0aaf6b6 6 / 218 vs 8 / 311. Option ① therefore costs about
      1.4–1.7x the Python work again on every change; this is the number the (c) gate weighs.

15. **Merge gates as scripts (TIDY 4.15).** Done: `tools/merge_check.py` (merge preflight: CI on main, diff
    stat, run_all row list rebuilt from the branch tree, code generation reminders) and `tools/perf_gate.py`
    (exit 1 on a proportional M/L cell above hand 1.0x, debt list for the rest) with the `perf_cells.json`
    format and examples in `tools/testdata/`; described under T6 "Merge gates". No test rows changed.

16. **Health check 2026-11-28 (TIDY 4.16).** Broad check and blind-spot review at a206043a. Fixed at once:
    `tools/merge_check.py` now judges only `ci.yml` push runs and prints WAIT when the judged run is not for
    main HEAD; `tools/perf_gate.py` fails an input with zero cells. Queued, one compiler bundle after P11-zh
    (rows HC-1..HC-4, same compiler files, run one after another):
    - HC-1 proof gap: `ev_remove` of a listener slot (kind 2) still proves `ensures ev_ok(result)` although the
      generated code records error 2 when the wake fails (`tests/g10_async/proof/ev_remove.pruv:4`,
      `pruv-proof/src/symbolic.rs:5065`, `pruv-core/src/codegen.rs:8336`, `docs/G10_ASYNC_SPEC.md:891`).
      Needs a reject twin (listener) next to the timer accept case.
    - HC-2 wrong LLVM IR: a fixed array `[i64; 2]` with the sortedness `requires` (SPEC 3177/3770) emits
      `icmp slt i64 %t1, None` in safe mode (`pruv-core/src/llvmgen.rs:1538,7591,8550`).
    - HC-3 Rust name capture: a parameter named like a called function (`bump: i64` and `bump()`) emits
      `return bump();` on the parameter (`pruv-core/src/codegen.rs:6594`). Needs a rustc-built twin.
    - HC-4 R-7-3 mismatch (also 4.4zh-IP, in P11-zh): explicit `#[inline]` cuts the recursion graph and
      `__pruv_fp` uses the adapter's call count (`pruv-core/src/inline_policy.rs:48,156`).
    Queued, tests/harness bundle (no compiler change, may run beside a compiler bundle):
    - HC-5 `tests/golden.py:212` and `tests/diff_rust.py:280` skip stored keys that were not run this time;
      `--quick` must fail on requested-but-missing `(seed, index)` keys.
    - HC-6 `bench/l3/budget/m9_test/run_m9.py:395` returns known-difference for a whole record before the
      event order and final state are compared; limit it to the number field.
    - HC-7 `bench/l3/claude/entry.pruv:10` `parse_line` contract holds for an always-`Skip` body; add the
      success condition for a plain user line and a mutant in `q_mutants.py` that must be rejected by it.
    - HC-8 `.github/workflows/windows-abi.yml:7` path filter omits the compiler, so committed `.win.ll` can go
      stale; run `tests/abi/winci/prepare.py --check` when ABI codegen changes.
    - HC-9 `tests/spec_trace.py:57` does not fail on SPEC sections without any rule ID (10 of 42); add IDs or
      an explicit exemption list for the example section.
    Recorded, not queued yet: LLVM `par` per-thread 64 KiB block cache not returned at thread exit
    (`pruvc/runtime.c:208`, `parallel_shim.rs:34`), measure under repeated `par` before a row; trusted
    declarations 7 -> 42 in 14 days (26 in budget), counted at every broad check; uncalled
    `g6_unimplemented_error` / `g9_unimplemented_error` / `take_array_abi_error` go to the next dead-code pass.

Order after the review: T0①–⑥ → T3 → T1 → T2 → T4 → P16, P17a → T5 (inline policy) → T6.
