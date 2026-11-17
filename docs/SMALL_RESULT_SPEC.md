# Small array results on the stack (KD-M6-3)

Status: decided 2026-11-17 after an opposing-side debate (see Decisions). Rust-backend-only optimisation; the
verified program, its obligations and the proof cache are unchanged.

## Problem

`int_text(x) -> [i64]` (bench/l3/budget/bud_text.pruv) builds `String(x)` in one `alloc(n, 0)` of at most 17 units.
Profiling (bench/l3/budget/M6_NOTES.md, KD-M6-3) shows the heap allocation and free of that `Vec` are about 80% of
the per-call cost on macOS (27-32 ns vs 21-25 ns for V8's `String(n)`). The same pattern (a short text built once,
read, then dropped) is the common shape of the budget text helpers: `amount_text` builds `int_text(u)` and only reads
it through `cat`. A hand-edited copy of the generated Rust in the shape below measured 13.7-14.0 ns per call at
n = 1000 (same checksum), so the shape reaches the target; the compiler output is measured again in RESULTS.md.

## Rule

A call `f(args)` whose array result is short and does not outlive the caller's binding is compiled to write into a
fixed-size stack buffer owned by the caller instead of a heap `Vec`.

### When it applies

Only in the verified Rust mode (`verified` on, `trust_callers` off, no cost/budget threading): the mode in which
`alloc` lowers to `vec![..]` and the verifier has proved every contract. In every other mode, and in the LLVM
backend, nothing changes.

### Callee eligibility ("small function", bound `K`)

1. The return type is `[i64]` and both the result and the local `o` below use the plain `i64` storage layout
   (no narrowed element layout from the storage plan).
2. The source declaration (before specialisation) is not generic, not foreign, and has a top-level `ensures`
   conjunct `len(result) <= K` with `K` an integer literal, `1 <= K <= 64`. Specialisation drops `ensures` of
   non-foreign functions, so `K` is taken from the source declaration while specialising and carried to code
   generation as per-function metadata (`small_bound`); programs that are not specialised read it from `ensures`
   directly. The bound comes from the contract, never from inference.
3. The body declares exactly one `let mut o: [i64] = alloc(e_n, e_v);` as a top-level statement, its last statement
   is `return o;`, and there is no other `return` anywhere in the body (nested blocks included).
4. After its declaration `o` occurs only as `len(o)`, `o[i]` (read), `o[i] = e`, or a direct argument (by value or
   `inout`) of a call to a user function. Not in `e_n`/`e_v`, never assigned whole, never an argument of a builtin
   other than `len` (so `copy`/`truncate` moves never see it).
5. The body is transitively foreign-free: every user function it calls (recursively) is a non-foreign Pruv function.

Why the bound holds at the `alloc`: the verifier proves `len(result) <= K` on every normal return, and `o` is the
only returned value whose length cannot change (array `inout` parameters are generated as `&mut [T]`, which cannot
resize; whole assignment is excluded). With a foreign-free body, verified termination (`decreases`) means every run
that reaches the `alloc` returns normally unless a resource runs out (allocation failure, budget), so `e_n <= K` at
the `alloc`. Rule 5 closes the debate's counterexample: a trusted callee with `ensures false` that panics after an
oversized `alloc` would otherwise turn a later panic into an earlier one.

### Call-site eligibility

A binding `let r: [i64] = f(args);` (immutable, the whole initialiser is the call) is rewritten when `f` is a small
function and every later occurrence of `r` in its scope is `len(r)`, `r[i]` (read), or a direct by-value argument of
a call to a user function (not a builtin). The checker already rejects shadowing and implicit array aliasing, and a
callee that returns its parameter must `copy` it, so such a callee cannot keep `r`.

Not rewritten: calls in other positions (`return f(x)`, `g(f(x))`, `a = f(x)`), `let mut`, `r` passed `inout`,
written, returned, assigned, put in a struct or passed to a builtin such as `copy`; call sites inside functions that
are part of a recursion cycle; and further call sites in a function once its rewritten buffers would exceed 4096
bytes (source order). `limit_text` (returns `int_text(..)`) stays on the heap; `amount_text` and the perf loop are
rewritten.

### Cost bound

Callee eligibility: one pass over each body plus a fixpoint over the call graph for rule 5, O(program size).
Call-site eligibility: for each candidate binding (a `let` whose initialiser calls a small function) one pass over
the rest of its block, O(candidates x body size); candidates are few. Generated code: at most 4096 bytes of extra
stack per non-recursive function; the extra work per call is zeroing `K` words, linear in `K <= 64`.

## Rust backend shape

For each small function the backend emits, right after its existing body function, with the same `#[allow(..)]`
line (`base` is the mangled function name without `__pruv_body`):

```rust
fn base__pruv_small(pruv_sb: &mut [i64; K], x: i64) -> usize {
    let n: i64 = int_len(x);
    let o: &mut [i64] = { let pruv_n: i64 = n; let pruv_v: i64 = 0i64; let pruv_s: &mut [i64] = &mut pruv_sb[..pruv_n as usize]; pruv_s.fill(pruv_v); pruv_s };
    put_int(&mut *o, 0i64, x, n);
    return o.len();
}
```

The rest of the body is emitted by the same statement emitter, with `o` treated like an `inout` slice (`&mut *o`
when passed on). The slice is bounds-checked by Rust. An eligible call site becomes three lines:

```rust
let mut pruv_sb_r: [i64; K] = [0i64; K];
let pruv_sl_r: usize = base__pruv_small(&mut pruv_sb_r, (x + d));
let r: &[i64] = &pruv_sb_r[..pruv_sl_r];
```

and later uses of `r` are emitted unchanged (`r.len()`, `r.get_unchecked(..)`, `&r` coerces to `&[i64]`). `pub`
entry wrappers keep returning `Vec`, so the external ABI does not change. The variant is emitted for every small
function (unused ones fall under the existing `dead_code` allow).

## LLVM backend shape (not implemented)

LLVM arrays are owned `{data, len, cap}` values freed at block end. The matching shape is an `alloca [K x i64]` in
the caller's entry block, a `base__pruv_small(ptr %sb, ..) -> i64` variant, and a caller local marked not-owned so
block-exit and statement-end frees skip it. M6 measures only the Rust pairs, so this is a follow-up; this change is
not an LLVM speed-up.

## Verification meaning

None changes. The rewrite runs after verification on the program code generation receives; obligations, proof cache
keys and the trust report are identical. Code generation now relies on a verified `ensures len(result) <= K` and on
rule 5; this is why it is off in every non-verified mode.

## Alternatives considered

| option | effect on real flows | decision |
|---|---|---|
| Destination passing for loop-local results (one buffer hoisted out of the loop) | only results bound inside a loop; single calls such as `amount_text` keep the allocation | rejected: benchmark-loop shaped |
| Per-size thread-local free lists in generated code | every allocation; `Vec` drop always calls the global allocator, so it needs a custom array type for locals (wrappers could convert back to `Vec`) | rejected: changes the array representation throughout codegen; TLS access on every alloc/free |
| Small-vector enum (inline up to K, else heap) | same cases as this design plus escaping results | rejected: changes local types, moves and `copy`/`truncate` paths in both compilers |
| Auto-generated `inout` output-buffer variant | with a heap buffer it is destination passing; with a stack buffer and a contract size it is this design | merged into this design |
| Infer the bound (interval analysis of the `alloc` length) | no contract edit | rejected for now: new inference with its own soundness argument; `int_text`'s length comes from `int_len(x)`, not a literal |

## Decisions (debate 2026-11-17, opposing side: holds with changes)

1. Bound metadata survives specialisation (`small_bound` set from the source declaration). Adopted: the M6 program is
   specialised and would otherwise never qualify.
2. Off in every non-verified mode. Adopted: a false bound would add a new panic where the heap path succeeds.
3. Bound at the `alloc` vs at the return. Adopted rule 5 (transitively foreign-free body) so the verified postcondition
   implies the bound at the `alloc`.
4. Borrowed bindings must not meet the `copy`/`truncate` move optimisation. Adopted: `o` and `r` may reach only user
   functions and `len`; `o` is passed as `&mut *o`.
5. Performance claim. Adopted: the 12 ns row was buffer reuse, not this shape; the hand-edited stack shape measured
   13.7-14.0 ns, and completion requires compiler output at or below TS at n = 100/500/1000 plus a measurement of
   `amount_text` before and after (outside bench/l3/budget, so the M3-M4 work there is untouched).
6. Stack total: 4096 bytes per function and no rewriting inside recursion cycles (advisory, adopted).
7. TLS, a new array type and LLVM are out of scope for this change.

## Source change

`int_text` gains `ensures len(result) <= 17` (already implied by `int_len`'s contract; the verifier proves it). This
is the only edit under bench/l3/budget besides the M6_NOTES entry.

## Test plan

1. Runtime test case (both compilers through tests/run_all.py): a small function and call sites that are rewritten
   (loop binding, single binding read through a user function) and not rewritten (returned, `let mut`, `inout`,
   passed to `copy`, nested call, no bound, `K` = 65, callee calling a foreign function), pinned with
   `gen_contains`/`gen_absent`, and results at lengths 1 and `K`.
2. tests/diff_rust.py on that group and `--fuzz-count 20`; `cargo test --release`.
3. M6: `run_m6.py` 300/300 on py/rust and rs/rust; `--perf` at n = 100/500/1000 with every op at or below TS.
