# L3 LB3: owned recursive values on LLVM, host ABI of owned collections

Language bundle LB3 of the Rust-shape reset (docs/RUST_SHAPE_RESET.md section 4, V blockers 3 and 4). Goal: the LB1
value shape (`enum Value { .., Array(items: Vec<Value>), Object(fields: Vec<Field>) }`, `struct Field { key:
Vec<i64>, value: Value }`) compiles on the LLVM backend in both compilers with byte-identical IR, each owned value
created, overwritten, moved and dropped exactly once; and SPEC states the host ABI of owned collections.

## Design (V3: LLVM layout)

### Native vectors of a recursive group (`Vec<S>`, R-6e-20)

`S` is a non-generic user struct/enum of the LB1 recursive group (`prog.g9["rec_vec_elems"]`).

- **Layout.** `Vec<S>` is the LLVM aggregate `{ ptr, i64, i64 }` (data, len, cap), the same shape as the existing
  `ARR`. Elements are `%pruv_t_S` values stored inline in the buffer, as in Rust's `Vec<Value>`; there is no box per
  element. The empty vector is `zeroinitializer` (null data, len 0, cap 0).
- **Allocator.** The buffer comes from `pruv_node_alloc_v1(cap * size, align)` and goes back to
  `pruv_node_free_v1(data, cap * size, align)` (runtime.c, marker `pruv_runtime_nodes_v1`), where `size`/`align` are
  the `g9_node_layout` values of `%pruv_t_S`. A vector with cap 0 owns nothing and is never freed.
- **Growth.** On push with `len == cap`: `ncap = cap == 0 ? 4 : cap * 2`; `ncap * size` is computed with
  `llvm.umul.with.overflow.i64` and an overflow takes the existing allocation-failure path; the new block is
  allocated, `len * size` bytes are copied with `llvm.memcpy`, and the old block (if cap != 0) is freed. The element
  bits are moved, not cloned, so nothing is dropped or cloned on growth.
- **Helpers** (emitted once per element type that is used, internal, fixed names so both compilers emit the same
  text):
  - `define internal void @pruv_vdrop__S(ptr %v)`: when `S` is extended, call `@pruv_drop__S` on each element
    `0 .. len`; then, when `cap != 0`, free the block. Does not reset `%v` (the owner is dead after drop).
  - `define internal void @pruv_vclone__S(ptr %src, ptr %dst)`: `len == 0` stores `zeroinitializer`; otherwise
    allocates exactly `len` elements (cap = len), clones each element with `@pruv_clone__S` (or copies it when S is
    not extended) and stores `{ data, len, len }`.
  - `define internal void @pruv_vpush__S(ptr %v, ptr %x)`: grows as above, then copies the `%pruv_t_S` bits at `%x`
    into slot `len` and stores `len + 1`. The caller has given up `%x` (moved), so `%x` is not dropped.
- **Use in G9 helpers.** In `@pruv_drop__T`, a `Vec<S>` field calls `@pruv_vdrop__S` on the field pointer; in
  `@pruv_clone__T` it calls `@pruv_vclone__S(field_src, field_dst)`. A type with a `Vec<S>` field is extended. The
  size helper and the run-time size contract fold element sizes as the Rust backend does (`pruv_csize__S`).

### Scalar vectors inside an aggregate (`Vec<i64>` / `Vec<bool>` fields)

A `Vec<i64>` / `Vec<bool>` field of a struct or enum payload keeps the G6 representation: an opaque `ptr` handle to
the shim-owned vector (`pruv_g6_v1_*`). Null means "moved out / inactive variant". The G9 drop helper calls the G6
drop symbol on a non-null handle; the clone helper calls the shim clone symbol on a non-null handle (null stays null).
This costs one extra allocation per key compared with Rust's inline `Vec<u16>`; see Open items.

### Operations

| source | LLVM |
|---|---|
| `vec_new::<S>()` | `{ ptr, i64, i64 } zeroinitializer` |
| `vec_push::<S>(move v, x)` | the owner's aggregate is taken (its place is reset to zero), `x` is spilled, `@pruv_vpush__S` runs on the taken aggregate in a temporary, the result is the new aggregate |
| `len(v)` | `extractvalue { ptr, i64, i64 } v, 1` (or a load of field 1 through the pointer of a read parameter) |
| `vec_get::<S>(v, i)` | a borrowed aggregate read at `data + i * size` (gep of `%pruv_t_S`); no clone. When the index is not proved (`--no-verify` or unproved), a bounds check takes the existing G6 `vec_get` failure path |
| `match vec_at_mut::<S>(inout v, i) { Some(b) => .., None => .. }` | bounds check; the `Some` arm binds `b` to the element pointer, the `None` arm runs otherwise |
| `*b = e` | `@pruv_drop__S(b)` (when extended) then store `e` |
| `b.f = e` | `assign_path` on the element place: the new value is complete before the old field is dropped |
| `move place` | load the value, then store `zeroinitializer` (vector) or `null` (handle) into the place, so a later drop or overwrite of the place does nothing for it |

Read parameters of type `Vec<S>` are passed as a pointer to the aggregate (as other aggregates). An owned local is
dropped at the end of its scope by `free_scope`; a value moved out was reset, so the drop is a no-op.

### What stays rejected on LLVM

- Borrowed views and borrowing structs (R-6g-11): `codegen/unsupported_type` (not in this bundle; see Open items).
- A host entry (`pub fn` / foreign export) whose signature has a `Vec<S>` or a type with owned collection fields:
  `codegen/unsupported_type` (V4 below: such values do not cross the C ABI).

## V4: host ABI of owned collections

SPEC.md (ABI section) states the rule: on the LLVM backend, a native vector and a G6 handle never cross the C ABI;
the allocator that allocated a block frees it (runtime node allocator for native vectors, the shim's Rust allocator
for G6 handles; they are never mixed); ownership of moved arguments has passed to the callee. A real allocation
failure in the node allocator aborts the process (as Rust's `handle_alloc_error`); only an overflow of the
`cap * size` multiplication takes the cleanup path, where the callee drops what it owns before it reports the
failure. On the Rust backend, values cross by Rust ownership (passed by
value, the receiver owns and drops).

## Tests

| test | backend | expectation |
|---|---|---|
| tests/spec/r6e_21_llvm_tree.pruv | llvm | accept; `Tree` with `Vec<Tree>` and `Vec<Field>` (Field has a `Vec<i64>` key), push, `vec_at_mut` + `*b = ..`, decreasing recursion; IR contains `@pruv_vdrop__Tree`, `@pruv_vpush__Tree`, `@pruv_node_alloc_v1`; prints `6`, `pruv-g6-live: 0` |
| tests/spec/r6e_21_host_entry.pruv | llvm | reject `type/entry_signature`: a `pub fn` taking a recursive owned value is not a C ABI entry (V4) |
| tests/spec/r6e_1_llvm_vec_field.pruv | llvm | accept; a struct with a `Vec<i64>` field (shim handle) prints `5`, `7`; IR identical in both compilers |
| tests/spec/r6e_1_llvm_field.pruv | llvm | reject `codegen/unsupported_type`: a `String` field still needs the Rust backend (the old `Vec<i64>` case is now accepted) |
| tests/spec/r6e_20_*.pruv accept tests | rust and llvm | the LB1 accept tests that LLVM used to reject with `codegen/unsupported_type` now compile on LLVM; same output as the Rust backend |
| bench/l3/budget/m12_test/value_ops.pruv | llvm | output equals value_ops_hand.rs (`3 7 9 2 -1 3 5 8 59`); `pruv-alloc-balance: 7 7`, `pruv-g6-live: 0` |

Leak check: `leaks --atExit` cannot attach in this sandbox ("Couldn't get task port"), so leaks are counted with
the runtime's counting allocator (`-DPRUV_COUNT_ALLOCS`, every node and buffer allocation and free) plus the G6
live counter of the bindings. All runs below end with equal allocs and frees and `pruv-g6-live: 0`.

### Measurements (LLVM backend, macOS arm64, 8 MB main stack)

Programs: `Tree { Leaf(n), Branch(children: Vec<Tree>) }`; deep_N builds a chain of N branches by push, wide_N one
branch with N leaves; main clones the value, then prints `depth` and `count` (both `decreases size(t)` recursion).
Hand twin: same algorithm in Rust (`Vec::new` + `push`, derived `Clone`, recursive `&Tree` traversal).

| case | pruv LLVM | pruv Rust backend | hand Rust |
|---|---|---|---|
| deep, largest N that runs | 120000 (140000 overflows) | 70000 (80000 overflows) | 70000 (80000 overflows) |
| deep_N allocs/frees (COUNT) | 2N / 2N (N = 1000 .. 120000) | - | - |
| wide_100000 allocs/frees | 17 / 17 | - | - |
| deep_60000 time | 24.78 ms | - | 17.04 ms (1.45x) |
| deep_60000 build + drop only | 11.26 ms | - | 11.13 ms (1.01x) |
| deep_60000 build + clone + drop | 19.51 ms | - | 14.83 ms (1.32x) |
| wide_100000 time | 10.02 ms | - | 9.30 ms (1.08x) |
| deep_60000 / wide_100000 max RSS | 18.0 MB / 15.0 MB | - | 16.0 MB / 8.1 MB |
| proof + compile time (deep_1000, deep_100000, wide_100000) | Python 0.38 / 0.43 / 0.37 s; Rust 0.42 / 0.23 / 0.21 s | | |

The proof time does not depend on N: the recursion is proved once per function (`decreases size(t)`), and N appears
only as a loop bound in main. The perf gate (tools/perf_gate.py) fails on these cells; see Open items.

### Input depth policy (proposal)

Drop, clone, and the user's recursive functions recurse once per nesting level, as in Rust. Measured stack per level is
about 64 bytes on LLVM and about 120 bytes on the Rust backend (and in hand Rust). Proposal: a program that builds a
recursive value from outside input (a parser) bounds the nesting depth, and the bound is a proof obligation, the
same way `decreases` bounds recursion. The default limit is 10000 levels, about 8x below the smallest measured
overflow (70000 with an 8 MB stack). serde_json uses a limit of 128, so 10000 is safe on the main thread and on 2 MB
threads (about 16000 levels at 120 B). Deeper inputs need an explicit stack size or an iterative drop (Open items).

## Open items

- **Speed below hand Rust (perf gate fails).** Root cause, measured: building and dropping cost the same (1.01x), but
  clone and wide values do not. 1) `%pruv_t_Tree = { i64, i64, { ptr, i64, i64 } }` is 40 bytes, while Rust's enum
  overlaps variant payloads and stores the tag in the Vec niche (24 bytes). Every element and copy is about 1.7x larger
  (RSS of wide_100000 1.85x). 2) Growth allocates a new block, copies and frees the old one. Rust uses `realloc`,
  which often grows in place. Fix: overlap enum payloads (G9 layout change in both compilers), and grow through
  `pruv_host_realloc_v2` with the same layout pairing. Both are layout changes and are left for the next bundle.
- **Borrowed views on LLVM (R-6g-11, R-6g-12).** Not done in this bundle (time limit). They stay rejected with
  `codegen/unsupported_type`. A view is a `{ ptr }` into the owner's place; the LB2 provenance rules already
  guarantee the owner outlives it, so the lowering needs no new run-time checks.
- **`Vec<i64>` / `Vec<bool>` inside aggregates use G6 handles.** That is one extra allocation and one indirection per
  key, compared with Rust's inline `Vec<u16>`. The native `{ ptr, i64, i64 }` lowering should also cover scalar
  element types.
- **Deep values beyond the stack.** Drop and clone are recursive, as in Rust. An iterative drop (Rust's `Drop`
  workaround for linked lists) would remove the depth limit of drop, but not of user recursion.
- **Native push skips two checked-mode guards.** The G6 shim push checks the catalog length bound (`len < 2^40`) and
  charges the contract budget; `@pruv_vpush__S` does neither (found by the audit). The length is still bounded by
  memory (the `cap * size` overflow check), but the checked-mode guards of the shim and the native path differ.
  Fix: emit the same bound and budget charge in the native push in both compilers.
- **Temporaries on a stopping failure.** A failure that returns to the caller (for example an unproved `vec_get`
  out of range) drops the statement temporaries, including the moved vector of a pending `vec_push`. A failure that
  stops the process (`pruv_panic_arith` under `--no-verify`) does not run that cleanup; like Rust's `panic=abort`,
  nothing is observable after it. The confirm audit noted this path; it is the same for every temporary.
- **Leak checker.** `leaks --atExit` could not run here (no task port). The counting allocator covers every
  runtime allocation, but not allocations made inside the Rust shim by the standard library.

## Who did what

- Python compiler implementation (pruvc/llvmgen.py, foreign_shim.py, mono.py, runtime.c): Codex gpt-6.2-sol (write delegation).
- Rust compiler implementation (pruvc-rs llvmgen.rs, foreign_shim.rs, bindings.rs): Codex gpt-6.2-sol (write
  delegation), then aligned to the Python IR byte for byte by a second Codex gpt-6.2-sol write delegation.
- Layout and encoding design, SPEC R-6e-21, tests, measurements and this note: Claude Opus 5.5.
- Audit: Codex gpt-6.2-astra (ultra). The first attempt was stopped by a content filter and was rerun with plain
  compiler-correctness wording. Required findings fixed by Claude Opus 5.5 in both compilers: the moved vector of
  `vec_push` is a temporary while the element is evaluated (dropped on a failure there); `type_sccs` is renamed to
  the concrete names after monomorphization; `size(v[i])` in a run-time contract is rejected in Rust as in Python.
  The push guards and the allocation-failure wording became the Open item and the V4 text above.
