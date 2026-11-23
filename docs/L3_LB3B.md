# L3 LB3b: overlapping enum payloads, realloc growth and checked native push

Follow-up of [L3_LB3.md](L3_LB3.md). Goal: recursive values and `Vec<S>` on the LLVM backend reach hand Rust on time,
RSS and allocation count (1.0x is the floor). Both compilers emit byte-identical IR for every program below.

## Changes

### D-LB3-1: overlapping enum payloads (both compilers)

An enum gets the union code form when at least two variants carry payload fields and every payload leaf has an
alignment of at most 8:

```
%pruv_t_Tree = type { i64, [3 x i64] }              ; tag + W words, W = largest payload word bound
%pruv_v_Tree__Leaf = type { i64, i64 }               ; tag + the variant's fields, used for field access
%pruv_v_Tree__Branch = type { i64, { ptr, i64, i64 } }
```

- `W` is an upper bound in 8-byte words. Every scalar leaf counts one word, a native vector or buffer three, a nested
  aggregate the sum of its leaves. Each leaf is at most 8 bytes with alignment at most 8, so the bound holds on 32-bit
  targets too.
- Field access through a pointer uses a gep through the variant type (index `1 + i`). The tag stays at index 0 of
  both types.
- SSA construction builds the variant value and stores it through an entry-block alloca. SSA extraction stores the enum
  value and loads the field through the variant type. SROA removes the slots.
- The contract form `%pruv_c_E` keeps the flattened layout. The code-form and contract-form field indices are separate
  functions.
- Enums with at most one payload variant, enums with a wider leaf, and enums reachable from a host or foreign ABI
  signature keep the flattened layout. Their IR is unchanged.
- The union form is used only when it is not larger than the flattened form: `8 * (1 + W)` must be at most a lower
  bound of the flattened size on 64-bit targets (leaf byte lower bounds summed with the tag, rounded up to 8;
  `bool` 1, `i64`, boxes and handles 8, arrays and native vectors 24). An enum with eight `bool` fields in one
  variant and one in another stays flattened (24 B instead of 72 B); `Chain { End([i64]), Next(Chain) }` overlaps
  (32 B instead of 40 B). Bounds above 2^40 rule out overlap in both compilers, so they decide alike. Found by the
  audit and its confirm review. On 32-bit targets (not a public target) the union form can be larger.
- `Tree` nodes shrink from 40 B to 32 B. Rust's `Tree` is 24 B because it stores the tag in the `Vec` capacity niche.
  No niche is used here (open item D-LB3-1).

### D-LB3-2: native vectors grow through realloc

`@pruv_vpush__S` grows the buffer with the new runtime entry `pruv_node_realloc_v1(ptr, old_size, new_size, align)`.
Its layouts match `pruv_node_alloc_v1`/`pruv_node_free_v1`: system `realloc`, or `pruv_host_realloc_v2` with byte
counts under `PRUV_RUST_ALLOCATOR`. A zero-sized old block holds one byte, so it is resized, not replaced.
`@pruv_vclone__S` allocates its buffer without zeroing. The growth path has no `memcpy` and no separate free.

### Native push guards

`@pruv_vpush__S` emits the same length bound and budget charges as the G6 shim `vec_push`:

- With `--foreign-checks requires` or `all`, it checks `len < 2^40`. A failure calls
  `pruv_panic_vec_push_v1(0)`.
- With `all`, it also charges the contract budget like the shim. It charges the snapshot length before the push,
  then the prefix length, then 1. An exhausted budget calls `pruv_panic_vec_push_v1(1)`.
- With `off`, no check is emitted.
- On failure the vector is dropped. The moved element is dropped too while the vector does not yet own it.
- With `all`, the shim also re-checks its postconditions (new length, unchanged prefix, pushed element). The native
  push does not: it is compiled code, not a foreign implementation, and the re-check would cost O(len) per push.
  It still charges the same budget, so budget exhaustion is the same on both paths. SPEC.md records this exception.

## Tests

| test | checks |
|---|---|
| tests/spec/r6e_21_llvm_realloc | vpush grows through `pruv_node_realloc_v1`; no `memcpy` or `pruv_node_free_v1` in vpush; 20 pushes sum to 190 |
| tests/spec/r6e_21_llvm_push_checked | `--foreign-checks all --contract-budget 1`: the `2^40` bound, both budget charges and both panic calls |
| tests/spec/r6e_21_llvm_union | `%pruv_t_Tree = type { i64, [3 x i64] }`, the variant types, a three-shape enum built, matched, cloned and dropped; a single-payload enum keeps the flattened layout |

## Measurements

Bench programs and hand Rust twins are in `work/lb3b` (not committed). The cells are in `work/lb3b/perf_cells.json`.
Method: 11 alternating runs, median wall time including process start, max RSS from `/usr/bin/time -l`. Allocation
counts come from `-DPRUV_COUNT_ALLOCS` for Pruv and a counting `GlobalAlloc` for Rust; reallocs are not counted.
The base column is the LB3 merge (646ad4a5), measured the same way, with 9 runs.

| case | base pruv/hand | LB3b pruv | hand Rust | LB3b pruv/hand |
|---|---|---|---|---|
| deep_60000 (build, clone, depth, count) time | 1.18x | 9.41 ms | 8.76 ms | 1.07x |
| deep_60000 RSS | 18.0 / 16.0 MB | 15.0 MB | 16.0 MB | 0.94x |
| wide_100000 time | 1.10x | 4.05 ms | 3.98 ms | 1.02x |
| wide_100000 RSS | 15.0 / 8.1 MB | 10.2 MB | 8.1 MB | 1.26x |
| wide_100000 allocs | 17 | 2 | 8 (+15 realloc) | 0.25x |
| deep build+drop time | 1.03x | 5.45 ms | 5.36 ms | 1.02x |
| deep build+drop RSS | 14.1 / 11.2 MB | 12.1 MB | 11.2 MB | 1.08x |
| deep build+clone+drop time | 1.20x | 7.93 ms | 7.71 ms | 1.03x |
| deep allocs (all cases) | 1.00x | 120000 / 60000 | 120006 / 60006 | 1.00x |

The perf gate still fails on six M cells, between 1.02x and 1.26x. They are recorded as debt in
L3_RESTRUCTURE_SPEC.md §7.5.

## Open items

- **D-LB3-1 rest (tag niche).** `Tree` is 32 B against Rust's 24 B. This is the remaining RSS gap on wide_100000
  (1.26x). Storing the tag in an impossible `cap` value of a `Vec` payload (`cap > isize::MAX`) would close it. It
  needs a niche-aware tag read and write in both compilers and in the contract conversion.
- **Remaining time gap (1.02x to 1.07x).** The cause is not measured yet. Allocation counts are equal, so the next
  step is a profile of deep_60000: the clone and drop recursion and the spill slots of the union form.
- **D-LB3-3 (scalar `Vec` fields inside aggregates).** Not done in this bundle. `Vec<i64>` and `Vec<bool>` fields
  still use G6 handles.
- **Borrowed views on LLVM (R-6g-11/12).** Not done in this bundle.
- **Union spill per binder (audit).** A `match` on an enum value without an address stores the whole value once per
  bound field. With word-sized leaves SROA removes the slots in the measured benches (pruv IR is unchanged after
  the audit fix and the cells hold); with `bool` leaves a byte store can stay in a loop. Fix: spill once per match
  and reuse the slot, or pass constructor fields straight to the arm.
- **Runtime checks of the new tests.** The new spec tests check IR text. Running the programs (outputs, exit 101 and
  allocation balance of the budget failure) is still to add.

## Delegations

- Items D-LB3-2 and push guards (both compilers, runtime, tests): Codex gpt-6.2-sol (write delegation).
- Item D-LB3-1 union layout (both compilers, test): Codex gpt-6.2-sol (write delegation).
- Encoding decisions, the zero-size realloc fix, measurements and this note: Claude Opus 5.5.
- Audit: Codex gpt-6.2-astra (ultra). Required findings fixed in a follow-up commit: union size bound, the `all`
  postcondition exception (documented), stale design text. The spill per binder is kept open (see above).
