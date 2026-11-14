# P3-S1 design: fixed-size local arrays on the stack

Motivation: after P3-N1 sudoku peak RSS is still 1.071-1.082 of matched Rust (RESULTS.md 119). The element widths
now match; the rest is heap placement. A Rust program with the same `Vec` allocations measures 1632 KiB against
1536 KiB with stack arrays. The matched Rust keeps `sr`, `sc`, `cr`, `cc` and the tables in fixed-size arrays.
This batch puts small, non-escaping local arrays with a constant length on the stack in both backends. Language
semantics, types, effects and the verifier do not change.
Paths: `core/` means `pruvc-rs/crates/pruv-core/src/`.

## Decisions

1. **Eligible declaration.** `let x: [E] = alloc(C, v)` or `let mut x: [E] = alloc(C, v)` in a function body (any
   block depth, including loop bodies) qualifies when all of these hold:
   - `C` is a non-negative integer literal and `E` is a scalar element type (`i64` with or without refinement, `u8`,
     `bool`). Struct, enum, collection and nested array elements do not qualify.
   - `C * elem_size(layout)` is at most 16384 bytes. `layout` is the P3-N1 storage layout of the binding, so under
     `--no-verify` the size is computed with 8-byte elements.
   - The running total of qualifying arrays in the function, in source order, stays at most 65536 bytes. A
     declaration that would pass the total stays on the heap; later smaller ones may still qualify.
   - The function has a body, is not in a recursive call SCC (self-recursion included), has no `cost` clause when
     compiled unverified (`--no-verify` keeps a runtime cost counter; verified builds erase `cost`, and the generated
     code must stay identical to the clause-free program, so the clause is ignored there) and is not a generic body before specialization (the analysis runs after monomorphization, like P3-N1). Host-exported
     entries qualify: only their locals move, their parameters and results keep the host ABI.
2. **Non-escaping uses only.** Every use of the binding `x` in its scope must be one of:
   - `x[i]` as a read, `x[i] = e` as a store target, `len(x)`;
   - a direct argument `x` or `inout x` to a call of a user Pruv function whose parameter is an array (`&[T]` /
     `&mut [T]` in Rust source, pointer and length in LLVM). Foreign calls, builtins other than `len`, host ABI
     calls and `par` argument lists disqualify;
   - a contract, invariant or assertion expression reading `x[i]` or `len(x)`.
   Any other occurrence disqualifies the declaration: returning it, a reassignment `x = ...`, using it as an
   initializer or right-hand side, a struct or enum literal field, `copy`/`truncate`, any capture by a closure or
   `par` body, and any expression kind the analysis does not know. Bindings are resolved with the same lexical
   scoping as `pruvc/storage.py` / `core/storage.rs`; a shadowing binding is a different binding.
3. **One analysis in both compilers.** Python `pruvc/stack.py` (`plan_stack(prog, storage_plan)`) and Rust
   `core/stack.rs` (`plan_stack`) return the set of qualifying declarations, keyed like the P3-N1 storage plan
   (node identity in Python, node address of the analysed AST in Rust). Both code generators consult it.
4. **Rust source backend.** A qualifying declaration emits a fixed-size array instead of a `Vec`, keeping the
   evaluation order of the fill value:
   `let mut sr: [u8; 729] = { let pruv_v: i64 = 0i64; [pruv_v as u8; 729] };` (no `as` for i64 storage; `bool`
   elements as today). Index, store, `len` (`x.len() as i64`) and argument passing (`&x` / `&mut x`, unsized
   coercion to the slice parameter) keep their current spelling where it already compiles on arrays; anything that
   names `Vec` for the binding must use the array type instead. `mut` follows the existing rule.
5. **LLVM backend.** A qualifying declaration becomes `alloca [C x <storage type>], align <alignment(layout)>` in the
   function entry block (one per declaration, entry order = source order), filled at the declaration point by a new
   runtime helper `void pruv_fill_v2(ptr, i64 len, i64 value, i64 size)` (`pruvc/runtime.c`, all variants; it
   truncates `value` to `size` bytes, like `pruv_alloc_narrow`). The slot gets the array pointer and length `C` and
   is not registered in the scope's owned arrays, so no `pruv_free`/`pruv_free_v2` is emitted on any exit path
   (normal end, `return`, panic paths). Allocation counting is unchanged for the other arrays;
   `alloc_balance` still holds because stack arrays neither allocate nor free.
6. **Tests.** New `tests/runtime/p3_s1_stack.{pruv,rs,json}`: qualifying arrays of several layouts with nonzero and
   negative fills, a declaration inside a loop (refilled every iteration), early `return` out of scopes holding stack
   arrays, `inout` and read-only passing to helpers, `len` and contracts on stack arrays; plus
   non-qualifying cases that must stay on the heap (returned, reassigned, over 16384 bytes, over the 65536-byte total,
   recursive function, `copy` source). Generated-code checks: `gen_contains` for `[u8; ` style declarations and
   `gen_absent` for the heap cases' fixed-array spelling where practical; `ll_contains` `alloca [` and
   `pruv_fill_v2`. The same fixture under `--no-verify` through `flags_matrix`.
7. **Expected effect (to be measured).** sudoku: `sr`, `sc`, `cr`, `cc`, `grid` in `solve` and `rows`, `cols`,
   `nr` in `bench` move to the stack (`out` is returned and stays on the heap). Other bench programs: only arrays
   that pass the rules (check the generated code of all nine; time and RSS must stay <= 1.05).

## Files to change

- Python: new `pruvc/stack.py`; `pruvc/codegen.py` (declaration emission, any `Vec`-typed spelling of the binding);
  `pruvc/llvmgen.py` (entry alloca, fill call, slot not owned, runtime declaration); `pruvc/runtime.c`
  (`pruv_fill_v2`).
- Rust: new `core/stack.rs` (registered in `core/lib.rs`); `core/codegen.rs`, `core/llvmgen.rs` counterparts.
- Unchanged: checker, refinement expansion, verifier, monomorphizer, P3-N1 storage plan, ABI layouts.

## Risks

- A missed escape gives a dangling pointer (LLVM) or a compile error (Rust source). Anything unknown must keep the
  heap path.
- Stack growth in deep non-recursive call chains is bounded by the per-function total; `par` worker threads have
  smaller stacks, which the 65536-byte cap keeps safe.
