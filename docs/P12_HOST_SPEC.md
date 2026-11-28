# P12 host boundary: bundle plan and P12a design

P12 opens the host boundary (the Rust API a Pruv program exposes, and the LLVM C ABI) for the types that today stop at
`small_int_unsupported`, `tuple_module`/`const_module`/`alias_module`, `entry_signature`, `unsupported_type` and
`tuple_llvm_owned`. The Rust backend goes first; LLVM follows per type family.

P12a target: `pub fn reference(sid: [u16], ..)` compiles to a Rust wrapper a plain Rust caller calls as
`reference(&sid, ..)`, matching the hand form `pub fn reference(sid: &[u16], ..)` (bench/l3/budget/RESHAPE.md M12-11,
bench/l3/budget/m12_test/hand_life.rs:660). The same program on LLVM fails with a located
`codegen/small_int_unsupported`.

## Cost bound

Costs are per host boundary crossing, measured in the size of the data that crosses. Explicit `requires`, the body and
copies the caller asks for are counted separately. The target is zero copies. Each bundle states three numbers apart:
the descriptor handoff (O(1) when the layout is the same on both sides), the bytes copied into separate output storage
(0 unless the bundle row says otherwise), and the total pass cost including validation (O(n) in the data that crosses;
a UTF-8 check followed by one copy reads each byte twice and is still O(n)).

## Bundle order

| Order | Scope (rows closed) | Rejections lifted / kept | Acceptance tests and cost bound |
|---|---|---|---|
| P12a: Rust narrow ints and buffers | Every P3 width (`u8 u16 u32 u64 usize i8 i16 i32 isize`) as a parameter or result, directly or as a field of a public struct; borrowed `[u8]`/`[u16]` parameters. Rust-entry part of P2-4, P3-5; M12-11 `sid`. | Lift `small_int_unsupported` for Rust host entries only. Keep: LLVM host signatures, trusted rust fn signatures, fixed arrays `[T; N]` at a host entry. | Real Rust caller runs (R-6p-6 tests), LLVM and foreign negative tests. Scalars O(1), slice borrow plus one length check O(1), element copies 0. |
| P12b: Rust module tuples, consts, aliases | Tuple identity by resolved component types across modules; `pub const`, `pub type`, `m::NAME`, `m::Alias` resolved in the declaring module. P7-3, P9a-1, Rust part of P6-2. | Lift `parse/tuple_module`, `const_module`, `alias_module`. Public aliases are expanded before the boundary checks. A public tuple with no LLVM layout yet fails in LLVM codegen. | Tuple round trip across two modules, same-named consts in two modules, call through an alias; private-name leaks and alias cycles rejected. Const/alias cost 0; tuple linear in component size, O(1) for fixed scalars. |
| P12c: Rust owned collections and state | `Vec<T>`, `String`, `Vec<pub struct>` as read / `take` inputs and owned results; public state as `&mut State`. Rust side of the ABI row (docs/RUST_SHAPE_RESET.md) and Rust owned part of P6-2. | Lift the matching `type/entry_signature` cases. Keep invariant-carrying state the host can build, P11 type-argument limits, LLVM/raw C `unsupported_type`. | Empty values, non-ASCII strings, struct Vec round trip, single drop after `take`, state update and check failure. Move/borrow O(1); length checks over nested host-built buffers O(n) in total elements; copies 0. |
| P12d: LLVM narrow ints and buffers | P12a types on LLVM: fixed-width C integers, `[u16]` as aligned `const uint16_t *` plus `int64_t` count, owned results by the existing array return rule. LLVM part of P2-4, P3-5. | Lift the LLVM `small_int_unsupported` for these types. Keep owned general `Vec<T>`, unsupported aggregate fields, foreign limits. | SysV and WinX64 C callers and generated bindings: sign, surrogates, empty results, result free; negative and > 2^40 lengths rejected before the body. Scalars/borrows O(1), ownership handoff O(1), at most one O(output) pass if the array ABI needs separate output storage. |
| P12e: LLVM value tuples | P12b tuples on the existing aggregate word serialization, in component order. Consts and transparent aliases do not change layout. LLVM value part of P6-2. | Lift the value-tuple boundary rejection; keep `codegen/tuple_llvm_owned` for owned components. | Cross-module tuple results read through C bindings (order, sign, same-typed fields); `String` components still rejected. Linear in ABI words, O(1) for fixed size. |
| P12f: LLVM allocator and ownership ABI | Versioned owned-buffer contract (LB3/V4): per-element `data,len,cap`, element layout, canonical empty value, per-allocator create/free, handoff point. Keeps the array ABI and G6 handle free rules. | No user-type rejection is removed here; `unsupported_type` and `tuple_llvm_owned` still fire. | Free with the same allocator, size and alignment; `0 <= len <= cap`, `len <= 2^40` and out-of-range byte-size arithmetic checked before handoff; callee cleans up after handoff on recoverable failure; result ownership published only on success. Descriptor check/handoff O(1), partial cleanup linear in initialized elements. |
| P12g: LLVM owned types | On P12f: (1) `String`, scalar `Vec<T>`, integer/bool Vec `for`; (2) `Vec<struct>`, state, owned enums; (3) tuples with owned components. P4-5, P6-3, owned part of P6-2, ABI/V4, D83. | Remove `unsupported_type`/`tuple_llvm_owned` per type only. P4-2 non-scalar loop binders stay `type/for_iterable`. | Empty/multi-element Vec, loop sum and order, `String` tuples, struct with `[u8]` field, owned enum payload, mid-way failure with single free. Same-layout handoff O(1); UTF-8 check, nested layout change and drop O(n) in total data. |
| P12h: remaining boundary positions | Leftovers of P2-4/P3-5: narrow enum payloads (with P11), narrow ints and `[u16]` in trusted rust fn signatures, value tuples in foreign signatures (foreign part of P6-2). | Lift `small_int_unsupported`/`foreign_signature` only where implemented. P11 payloads, `char` validity and a separate `extern` syntax (`extern` is reserved) are not part of this row. | Both call directions: width min/max, returned length and foreign check mode, rejection of unsupported declarations. Direct values/borrows O(1), aggregate/return representation checks at most O(n). |

## P12a Rust API

| Pruv position | Generated Rust public signature |
|---|---|
| `x: u8` / `-> u8` (likewise `u16 u32 u64 i32`) | `pub fn f(x: u8) -> u8` |
| read `a: [u8]`, `sid: [u16]` | `pub fn f(a: &[u8], sid: &[u16])` |

- Each parameter and the result use the table on their own. `[u16]` is 16-bit code units; `0xD800` passes unchanged. No
  borrowed result or `ref` marker is added.
- Order in the wrapper: representation check, then `requires`, then the body. Rust scalar and slice element types carry
  their own range, so the only representation check is one `.len()` check per buffer (element count, `2^40` allowed, the
  existing `pruv: invalid representation: ... (array length)` path). No element walk, no UTF-16 check, no copy into
  `Vec<i64>`.
- A public entry checks `requires` from outside callers in verified and `--no-verify` builds alike. Integers are lifted
  straight to `i128`; going through `i64` would change the truth of a contract on `u64::MAX`. A false contract stops
  with `pruv: requires violated: ...` before the body. `--trust-callers` keeps its rule (contracts may be skipped, the
  length check stays).
- The same table applies to a single-file `pub fn` and a module program's root host entry. Child-module `pub` functions,
  dependency package roots, methods and generic instances get no host symbol; P12a does not widen the implicit public set
  of legacy files without `pub` items (their narrow-signature functions stay private items).
- A public struct with narrow fields crosses as the Rust struct with the same field types (no conversion;
  `pair_sum(p: Pair)` in tests/spec/r6p_6_host_accept). A narrow enum payload keeps the P11 rule. On LLVM a narrow
  field or a `[u16]` field is rejected like a direct one.
- Trusted rust fn signatures are the opposite call direction and keep the existing allowlist (`[u8]` reads stay valid).
- LLVM, standalone bindings and `raw_entries="c"`: the newly opened types fail with `codegen/small_int_unsupported`; the
  message names a later P12 bundle. Existing `[u8]` signatures and internal LLVM narrow-int and `[u16]` code are
  unchanged.

## P12a acceptance tests (R-6p-6)

1. Generated Rust compiled with a separate caller and run: `u8 u16 u32 u64 i8 i16 i32`, `u64::MAX` through
   `requires x > 0`, negative `i32`, `[u16]` with `0xD800`, `[u8]` indexing, a struct with `u8`/`u16` fields
   (tests/spec/r6p_6_host_accept), in verified and `--no-verify` builds.
2. Wrapper `requires` false stops before the body with exit 101 (tests/spec/r6p_6_host_requires).
3. Module program: root `pub fn reference(sid: &[u16], k: u8)` calls a child module's `[u16]` function
   (tests/spec/r6p_6_host_module).
4. LLVM host signature with a narrow int or `[u16]`: `codegen/small_int_unsupported` (tests/spec/r6p_6_host_reject_llvm,
   tests/must_reject/p1_pub_sig, p2_u16_pub_sig).
5. Trusted rust fn with `u64`: `parse/small_int_unsupported` (tests/spec/r6p_6_host_reject_foreign,
   tests/must_reject/p3_pub_u64).

## Next bundle

P12b (Rust module tuples, consts, aliases), then P12c, then the LLVM families P12d to P12g, then P12h.
