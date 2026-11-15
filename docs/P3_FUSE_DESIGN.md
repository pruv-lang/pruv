# P3-F1 design: fused requires check for element-wise map entries

Motivation: `add(a, b) -> c[i] = a[i] + b[i]` with `requires forall t in 0..len(a): 0 <= a[t] && a[t] <= 1000` (same
for b) runs 80 ms per 200 calls of 1M elements against 50 ms for plain Rust. The two prefiltered forall checks cost
28.6 ms and are already memory-bound (two extra passes over 16 MB). A Rust prototype that reads each element once,
ORs the negated element predicates into a flag without branches, writes the result and tests the flag once at the end
measures 52 ms; one branch per element measures 82 ms (no vectorization); chunked check-then-compute measures 62 ms.
This batch emits the first form inside the checked entry. Language semantics, types, effects, the verifier, panic
messages and budget behaviour do not change. Paths: `core/` means `pruvc-rs/crates/pruv-core/src/`.

## Decisions

1. **Matcher.** Python `codegen.fuse_map(f)` and Rust `core/emit_common.rs` `fuse_map(f)` return a plan or None.
   The plan is: `start` (index of the first fused requires), `arrs` (the fused forall arrays, requires order),
   `n_arr` (the loop bound array X), `elem` (the element expression E) and `idx` (the loop variable name). It exists
   only when all of these hold:
   - `f` has requires, no `cost` clause, no inout parameter, no refinement on any parameter or on the result, and
     returns `[i64]` (Type name `array`, no args, no owner).
   - The trailing requires `f.requires[start:]` (`start < len(f.requires)`, maximal suffix) are each
     `Quant(kind="forall", lo=IntLit(0), hi=Call("len", [Var x]), body=P)` where `x` is a parameter of type `[i64]`
     (Type name `array`, no args, no owner), and `fast_forall(q, arr_ok)` returns exactly `[(x, 0)]` with `arr_ok`
     accepting only parameters of type `[i64]`. Requires before `start` can be anything.
   - `f.body` is exactly four statements:
     1. `Let(name=c, type=[i64], value=Call("alloc", [Call("len", [Var X]), IntLit v]), refine=None)`;
     2. `Let(name=i, type=i64, value=IntLit(0), refine=None)` with `i != c`;
     3. `While(cond=Binary("<", Var i, Call("len", [Var X])), invariants=any, decreases=any, body=[
        IndexAssign(name=c, index=Var i, value=E, elem_refine=None), Assign(name=i, value=Binary("+", Var i,
        IntLit(1)))])`;
     4. `Return(Var c)`.
   - `X` is one of `arrs`. `E` is built only from `Binary` with op `+`, `-` or `*`, `IntLit`, and `Index(Var x,
     Var i)` where `x` is in `arrs`. Every name in `arrs`, `X`, `c` and `i` is distinct where required (`c` and `i`
     are not parameter names).
   Mutability of the two lets does not matter. The matcher is purely syntactic; it does not consult the verifier.
2. **When it applies.** Only a checked entry that evaluates requires (not under trust_callers, not a cost entry) of a
   verified build (`verified` true; `--no-verify` never fuses, so unverified arithmetic keeps its checks). Rust source
   backend: `Gen.entry` when `checked` and `s.verified`. LLVM backend: `entry_of` when not `trust` and not `cost` and
   the program is verified (pass the flag in). The `__pruv_par_chk` copies exist only unverified, so they never fuse.
3. **Emission.** The entry evaluates `f.requires[:start]` exactly as today (same code, same order). Then, before the
   code of `f.requires[start:]`, it emits the fused path, and after it the unchanged code of `f.requires[start:]` and
   the body call (the slow path). The fused path:
   - Guard (no side effects): `len(x) == len(X)` for every x in `arrs`, and `m * len(X) <= budget` where `m =
     len(arrs)` and budget is the current remaining requires budget (i128 arithmetic, so no overflow). When the guard
     fails, go to the slow path.
   - Allocate the result with the same runtime call the body uses for `alloc(len(X), 0)` (fill 0; every element is
     overwritten below, so the source fill value v does not matter).
   - One loop `j` from 0 to `len(X)` (i64): for each x in `arrs` in order, load `x[j]` once (unchecked; in bounds by the
     guard) and OR `!P_x` into a bad flag, where `P_x` is the forall body evaluated branch-free exactly like the
     prefilter (`cfast`, the quantified variable bound to j); store `E` at index j of the result with wrapping
     arithmetic (LLVM `add`/`sub`/`mul` without nsw; Rust `wrapping_add`/`wrapping_sub`/`wrapping_mul` on i64,
     literals as `Ni64`), reading `x[j]` for `Index(x, i)`. Reads of the same x reuse the loaded value.
   - After the loop: if the flag is clear, return the result (no budget change is observable: the entry returns).
     Otherwise free the result and fall through to the slow path, which re-checks the trailing requires and panics
     with today's message (or budget exhaustion), since some element violates one.
   Correctness: on the fast return every trailing requires holds and the prefix requires were checked, so all
   requires hold; the verifier proved E does not overflow under them, so wrapping E equals the body's E, and the
   result equals the body's result. Memory safety of the fused loop rests only on the length guard, not on requires.
   When the fused path falls through nothing observable happened (allocation is freed, budget untouched).
4. **Rust source shape** (one line, inserted as its own `s.emit(1, ...)` line after the prefix check lines), for the
   add example (`n = 2` arrays, `pruv_f` names are fixed; there is at most one fused path per entry):
   `{ let pruv_fn: usize = a.len(); if b.len() == pruv_fn && (2i128 * (pruv_fn as i128)) <= (pruv_budget as i128) {
   let mut pruv_fc: ::std::vec::Vec<i64> = vec![0i64; pruv_fn]; let mut pruv_fbad = false; let mut pruv_fj: usize = 0;
   while pruv_fj < pruv_fn { let pruv_fx0: i64 = unsafe { *a.get_unchecked(pruv_fj) }; let pruv_fx1: i64 = unsafe {
   *b.get_unchecked(pruv_fj) }; pruv_fbad |= !(P0); pruv_fbad |= !(P1); unsafe { *pruv_fc.get_unchecked_mut(pruv_fj)
   = E; } pruv_fj += 1; } if !pruv_fbad { return pruv_fc; } } }`
   where `pruv_fxK` is the K-th array of `arrs`, the guard lists `arrs` in order (`X` itself included, it is trivially
   equal), `Pk` is the prefilter's branch-free text of the forall body with `x[t]` reading `pruv_fxk` (as `(pruv_fxk as
   i128)`) and the quantified variable as `(pruv_fj as i128)`, and E reads `pruv_fxk`. The `vec!` drop on fall-through
   frees the result. Emit exact text; the Rust compiler must produce byte-identical output.
5. **LLVM shape.** Same structure in `entry_of`, placed after the prefix requires: guard blocks (`icmp eq i64` per
   array, i128 `mul`/`icmp sle` against the loaded budget), `@pruv_alloc` call as the body emits it, loop with i64
   counter, i1 `or` accumulation of `xor i1 P, true`, `getelementptr inbounds i64` loads/store, return of `{ ptr, i64 }`
   on a clear flag, `@pruv_free` on fall-through. Use the existing helpers (`s.label`, `s.op`, `s.ins`, `s.allocas`,
   `cfast`) so names follow the existing counters; no new global state.
6. **Tests.** New `tests/runtime/p3_f1_map.{pruv,rs,json}`: the add function plus a second map with `-` and `*` and a
   literal (`c[i] = a[i] * 3 - b[i]`), a prefix scalar requires `len(a) == len(b)`, and calls that (1) succeed and print
   a few elements, (2) violate an element of the second array at a late index (expect today's message for that
   requires), all under flags `[[], ["--no-verify"]]`, exit 101. Plus `tests/runtime/p3_f1_order.{...}`: the first
   array's forall fails at a late index and the second's at an early one; the message must name the first requires.
   Plus `p3_f1_budget` with `--contract-budget` so the two foralls do not fit (expect budget exhaustion as today) and
   `p3_f1_len`: lengths differ with no prefix length requires, so the guard falls through (a forall over b reads only
   b; expect the normal result or message as today). `ll_contains`/`gen_contains` check `pruv_fbad` (Rust) and a
   fused marker label in LLVM in the verified row only if the format supports per-row checks; otherwise skip that key.
