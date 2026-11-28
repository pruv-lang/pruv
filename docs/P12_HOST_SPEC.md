# P12 host boundary: bundle plan and P12a design

P12 opens the host boundary (the Rust API a Pruv program exposes, and the LLVM C ABI) for the types that stopped at
`small_int_unsupported`, `tuple_module`/`const_module`/`alias_module` (lifted on the Rust backend by P12b, SPEC R-6p-7), `entry_signature`, `unsupported_type` and
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
| `x: u8` / `-> u8` (likewise `u16 u32 u64 usize i8 i16 i32 isize`) | `pub fn f(x: u8) -> u8` |
| read `a: [u8]`, `sid: [u16]` | `pub fn f(a: &[u8], sid: &[u16])` |
| `take s: [u16]` / `inout s: [u16]` | `s: Vec<u16>` / `s: &mut [u16]` (the `[i64]` rules) |
| owned result `-> [u16]` (a `take` parameter returned with `move`, or `copy()`) | `-> Vec<u16>` (tests/spec/r6p_6_host_owned) |
| `inout x: u8` (any integer) | `type/entry_signature`, as for `i64` (R-6g-4) |

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
  of legacy files without `pub` items (their narrow-signature functions stay private items). On LLVM a child-module
  `pub` function is internal too and keeps narrow types (tests/spec/r6p_6_host_module_llvm).
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
5. Trusted rust fn with `u64` or `[u16]`: `parse/small_int_unsupported` (tests/spec/r6p_6_host_reject_foreign,
   r6p_6_host_reject_foreign_u16, tests/must_reject/p3_pub_u64).

## P12b Rust module tuples, consts and aliases

Paths are the existing absolute item paths (`crate::m::NAME`, `crate::m::Alias<..>`); inside its own module an item is
used by its bare name.

- Constants. `const` and `pub const` are allowed in every file of a module program. A constant belongs to its declaring
  module and is evaluated there (its initializer names only constants of the same file). `crate::m::NAME` reads that
  module's constant; a non-`pub` one read from another module is `private_item`. Same-named constants of two modules
  are different items and do not collide in the generated Rust. Cost: 0 at run time (the value is a literal).
- Aliases. `type` and `pub type` are allowed in every file. The right side is resolved in the declaring module, so a
  bare name in it means the declaring module's item wherever the alias is used. `crate::m::Alias<..>` expands to the
  right side with the arguments substituted; a non-`pub` alias used from another module is `private_item`. Public
  aliases are expanded before the public-interface checks: a `pub type` or a `pub fn` signature that reaches a private
  type through an alias is `public_interface`. A cycle through module paths is `type/alias_cycle`. Cost: 0 (aliases
  are transparent and print expanded).
- Tuples. Tuple identity is the list of resolved component types: `(Pair, i64)` in module `a` and
  `(crate::a::Pair, i64)` in module `b` are one type, two modules' own `Pair` give two types. The Rust backend keeps
  native Rust tuples, so a root host entry `-> (i64, u8)` is that Rust tuple. Cost: a tuple crosses by value, linear in
  its component size, O(1) for fixed scalars; no conversion.
- LLVM. A root host signature with a tuple has no C layout until P12e and fails in LLVM codegen; tuples inside a module
  program on LLVM follow the single-file rules (owned components stay `codegen/tuple_llvm_owned`).

## P12c Rust owned collections and state (design)

| Pruv position | Generated Rust public signature |
|---|---|
| read `v: Vec<T>`, `T` = `i64`, `bool` or an eligible public struct | `v: &Vec<T>` |
| `take v: Vec<T>` / owned result `-> Vec<T>`, same `T` | `v: Vec<T>` / `-> Vec<T>` |
| `inout v: Vec<T>` | stays `type/entry_signature` at a host entry |
| `Vec<N>` for a non-`i64` integer `N` | no public signature (P11 type-argument limit; `[u8]`/`[u16]` are array types) |
| `Vec<Vec<T>>` | no public signature (`type/foreign_owned_type`) |
| read `s: String` / `take s: String` / `-> String` | `s: &String` / `s: String` / `-> String` |
| read `s: str` / owned result `-> str` | `s: &str` / `-> String`; `take s: str` and `inout` strings stay unavailable |
| first parameter `inout st: State`, public non-generic struct | `st: &mut State`, updated in place |

- Representation. The wrapper and body use the same Rust types: a public `&[T]` cannot feed the body's `&Vec<T>`, and
  `&str` cannot feed `&String`, without building an owner. Native read signatures avoid a host shim.
- What is lifted. Direct vector inputs (read and `take`) today hit `type/entry_signature`; lift those. Lift the result
  rejection for a vector of an otherwise admissible public struct, and the ordinary read `String` and owned `String`
  result. First-position `&mut State` already passes; the position rule and scalar `inout` rejection stay.
- Element eligibility. The existing element checks run first. Host-built state with an effective invariant, and vector
  inputs whose elements carry one, stay `type/entry_signature` (no runtime invariant checker in P12c). These entries stay
  unavailable to LLVM, standalone C bindings and `raw_entries="c"` (`codegen/unsupported_type`).
- Wrapper order. Validate the host representation, then evaluate `requires` against borrowed inputs, then call the body.
  Every reachable host-built buffer (vector elements, UTF-8 bytes, fields of state and of each vector element) is
  checked against the `2^40` length bound before any contract runs or state changes. `--trust-callers` may skip
  `requires` but keeps the representation checks.
- Ownership. A `take` argument is owned by the wrapper, borrowed for the checks, then moved once into the body; on a check
  panic it is dropped by unwinding. The original allocation is forwarded, never copied.
- Acceptance tests (R-6p-8, verified and `--no-verify`, Rust caller): empty `Vec<i64>`/`Vec<bool>`/`String`/`str`;
  UTF-8 `é🦀` through `&String` and `&str` (six bytes) and a taken `String` returned with the same allocation; a vector
  of public records with owned byte fields round-tripped with allocation identity; a counted drop of a taken allocation
  on consumption, on a caught `requires` failure and after the caller drops a result; `&mut State` mutation and an
  unchanged state after a false `requires`; rejections for invariant-bearing state and vectors, vector `inout`,
  non-first state, `Vec<u16>`, `Vec<Vec<i64>>`, and LLVM/raw C entries.
- Cost. Descriptor handoff O(1); 0 bytes copied into separate storage; the boundary pass is O(1) for flat scalar vectors
  and strings and O(n) worst case over nested elements visited for the length bound. Contracts, body work and copies the
  caller asks for are counted separately.

## P12d LLVM narrow ints and buffers (design)

| Pruv position | C prototype of the LLVM host entry | Generated Rust binding |
|---|---|---|
| parameter/result `u8`, `u16`, `u32`, `u64`, `usize` | `uint8_t`, `uint16_t`, `uint32_t`, `uint64_t`, `uint64_t` | the Rust integer of the same name |
| parameter/result `i8`, `i16`, `i32`, `isize` | `int8_t`, `int16_t`, `int32_t`, `int64_t` | the Rust integer of the same name |
| eligible public struct `S` with such fields | input `const int64_t *words`; result `void` plus a last `int64_t *out_words` | `s: S` / `-> S` |
| read `sid: [u16]` | `const uint16_t *sid, int64_t sid_len` | `sid: &[u16]` |
| owned result `-> [u16]` | `void` plus a last `PruvArrayU16 *out` (`uint16_t *data`, `int64_t len`) | `-> Vec<u16>` |

- Representation. Direct parameters and results use an LLVM integer of the declared width. On SysV (x86-64 Linux, aarch64 Apple) the 8- and
  16-bit values carry `zeroext` (unsigned) or `signext` (signed) on the `pruv_abi_*` definition, as Clang emits (the call
  side belongs to the host's C compiler; Pruv emits no call to these wrappers);
  on WinX64 no such attribute is set and only the low bits of the declared width are used. Neither side assumes the
  whole 64-bit register is extended.
- What is lifted. LLVM `codegen/small_int_unsupported` is lifted for the positions of the table only. A struct keeps the
  existing word serialization in field order, one 64-bit word per integer field: sign- or zero-extended on write,
  truncated to the declared width on read; 64-bit fields keep all bits.
- What stays rejected. Owned general `Vec<T>`, aggregate fields rejected today, fixed arrays `[T; N]`, scalar `inout`
  and the foreign signature limits. `[u16]` inputs gain the read form only; `take` and `inout` `[u16]` get no new C
  prototype.
- Wrapper order. A non-empty `[u16]` points at 2-byte aligned readable storage; the length is the element count.
  Every buffer length is checked before any memory read, `requires` or the body: `0` and `2^40` pass, negative and
  larger values stop through `pruv_panic_repr(len, "array length")` (exit 101). The check stays under
  `--trust-callers`. Next to an existing `inout` buffer the overlap check uses 2 bytes per element. Every `u16` value,
  surrogates included, passes unchanged.
- Ownership. A result whose capacity differs from its length is shrunk with `pruv_shrink_v2(data, cap, len, 2, 2)`
  before the handoff. The C caller frees a non-empty result exactly once with `pruv_free_v2(data, len, 2, 2)` and does
  not dereference an empty one.
- Bindings. `--rust-bindings` declares `extern "C"` functions with the same widths. A read slice passes pointer and
  length. A non-empty result is taken over with `Vec::from_raw_parts(data, len, len)` and freed by `Drop`; an empty
  one is `Vec::new()`.
- Acceptance tests. On x86-64 SysV and WinX64 (tests/abi/winci, new `.pruv` and `calls.json` entries, regenerated
  `out/`), through a direct C caller and through the generated Rust bindings: min/max of each width, negative values,
  unsigned values with the top bit set, a mixed-width struct round trip, `[0xD800, 0xFFFF]`, an empty result, a result
  with capacity above its length (values and a single free). Direct C calls with lengths `-1` and `2^40+1` stop with
  the representation error and exit 101 before the body, in a separate process.
- Cost. Scalars, slice borrows and the result handoff are O(1); 0 input elements copied. Struct conversion is linear
  in its word count; at most one O(output) copy when the result shrink reallocates.

## P12e LLVM value tuples (design)

A value tuple is a tuple whose components, after alias expansion, are all scalars admitted by P12d (`i64`, `bool`,
`u8 u16 u32 u64 usize i8 i16 i32 isize`; not `char`), eligible public structs, or value tuples. Tuple identity is the P12b rule (resolved component
types), so the same tuple named from two modules has one layout.

| Pruv position | C prototype of the LLVM host entry | Generated Rust binding |
|---|---|---|
| parameter `p: (A, B, ..)` | `const int64_t *p_words` | `p: (A, B, ..)` |
| result `-> (A, B, ..)` | `void` plus a last `int64_t *out_words` | `-> (A, B, ..)` |
| struct field `f: (A, B, ..)` | the field's words inlined in the struct words, in field order | `f: (A, B, ..)` |
| nested tuple / struct component | the component's words inlined, depth first | the Rust tuple / struct of that component, e.g. `(crate::a::Pair, i64)` |

- Representation. A tuple uses the existing aggregate word serialization of P12d structs: components in order, one
  64-bit word per scalar component (`bool` as 0/1, narrow integers sign- or zero-extended on write and truncated on
  read), struct and tuple components flattened depth first. The word count is fixed by the type; the unit tuple has no
  host form. A tuple is never passed or returned as an LLVM first-class aggregate, so no target-specific aggregate
  classification (SysV register pairs, WinX64 by-reference rule) applies; SysV `zeroext`/`signext` stays limited to
  direct narrow scalars of P12d.
- Bindings. `--rust-bindings` packs and unpacks the words into the native Rust tuple in the same order; the binding
  declares the word count as a fixed array length (`[i64; N]`) so a short buffer is a Rust type error.
- Reading host words. A `bool` word other than 0 or 1 stops with `pruv_panic_repr(word, "bool")` (exit 101) before
  `requires` and the body, also under `--trust-callers`; integer words are truncated as for P12d struct fields.
- What is lifted. `codegen/tuple_host_unsupported` for value tuples in parameters, results and public struct fields of
  root host entries (and the implicit entries of legacy files). Consts and transparent aliases do not change layout.
- What stays rejected. A tuple with an owned component (`[u8]`, `[i64]`, `[u16]`, `String`, `Vec<T>`, owned enums)
  keeps `codegen/tuple_llvm_owned` until P12g; a tuple holding a fixed array, `inout` tuples and tuples in trusted
  rust/foreign signatures (P12h) keep their current codes. Standalone C bindings and `raw_entries="c"` gain the same
  word form only if they already accept structs.
- Acceptance tests (new SPEC row, verified and `--no-verify`, LLVM through a direct C caller and the generated
  bindings, plus tests/abi/winci for WinX64): a cross-module tuple result `(crate::a::Pair, i64)` read in component
  order; `(i8, u64, bool)` with min/max, negative values and the top bit set; two same-typed components that must not be
  swapped; a nested tuple inside a struct field; a bool word of 2 from C stopping with exit 101 before the body;
  rejections for `(String, i64)`, `([u8], i64)` and a tuple in a trusted rust fn signature. The existing
  `p12b_host_tuple*_llvm` reject tests move to owned-component forms.
- Cost. O(words) per crossing, O(1) for a fixed-size tuple of scalars; no allocation and no host shim per tuple type.

## Next bundle

P12e (LLVM value tuples, design above), then P12f (allocator and ownership ABI), P12g (LLVM owned types), then P12h.
