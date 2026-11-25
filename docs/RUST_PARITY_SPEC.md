# Rust parity plan

Status: plan, 2026-11-25. Owner: main session.

## 1. Goal and rule

Pruv adds every hand-Rust feature that does not clash with proofs or with
"one way to do it", instead of waiting for Lair ports to expose each gap.
Code shape comes from hand Rust (see docs/RUST_SHAPE_RESET.md); Pruv adds proofs on top.

- The Python compiler `pruvc/` is frozen (bundle PF). Every bundle here changes
  `pruvc-rs/` only. "Rust-only" means the compiler implementation language;
  each feature still needs both the Rust and the LLVM backend before it counts as done.
- A bundle groups 2-3 related features and stays under the 2-hour cap.
  If a bundle cannot close in 2 hours, it is split before the next run, not marked done.
- An unfinished feature is rejected with a clear error; it never leaks into an accepted path.
- Every bundle ships accept, reject, and false-contract tests plus runtime value checks on both backends.
- Bundles that overlap a queued Lair gap bundle (U, V, ABI in RUST_SHAPE_RESET; G-B* rows) run together with it.
- Language bundles edit the same compiler files, so they run one at a time.

## 2. Already in Pruv (do not re-count)

i64/bool/unit, `[u8]`/`[i64]`/`[bool]` buffers, UTF-8 `str`, structs/enums with invariants,
explicit generics (bound `Element` only), Option, typed Result, `?`, Vec/HashMap/String with limited nesting,
recursive values with compiler boxing, recursion through non-generic Vec, `while` with `decreases`,
`for (k, v) in HashMap<String, V>` (read-only), `inout` and disjoint field borrows,
borrowed slices/structs with `from(...)` provenance (R-6g9-1..8), mutable place bindings,
borrowed Option/Result payloads, buffer growth/copy/search builtins, modules/effects/extern/IO,
static string literals, stack promotion of fixed `alloc`, 12 std helpers (no sort or binary search).

Some old L3 queue rows still list items that SPEC now has (collection fields, Vec return,
inout expressions). Check SPEC before queueing.

## 3. Excluded

| Feature | Reason |
|---|---|
| `unsafe`, macros, implicit conversions, operator overloading | User decision; break proofs or single interpretation |
| Explicit `Box<T>` | Recursive boxing is already the single way; revisit only if a non-recursive heap need appears |
| Shadowing | Kept forbidden; a new name or `mut` reassignment is the single way |
| Escaping closures, `dyn` traits, `async fn`/`await` | Deferred, not ruled out: need long-lived capture and suspension models. The readiness API stays |

## 4. Bundle order

The order follows what hand Rust in the repo already uses: JSON (`usize`, `const`, tuple errors, `impl`,
slice `for`) at bench/l3/json/hand/hand_json.rs, and M12 (`Vec<u16>`, `Vec<(Text, Value)>`, iterators,
mutable closures) at bench/l3/budget/m12_test/hand_life.rs. U's key order is `&[u16]` code-unit order
(bench/l3/budget/p01_keys/hand_keys.rs).

| # | Bundle | Features | Joins | Split risk |
|---|---|---|---|---|
| P1 | Small ints | width descriptor in one place; explicit casts (widening = equality, narrowing = checked or proven range); scalar `u8`/`u16` on Rust and LLVM | U | medium |
| P2 | UTF-16 buffers | owned `[u16]` with growth/copy; `ref [u16]`; key compare/hash with lone-surrogate tests | U, LB4 | medium |
| P3 | Index and other widths | `u32`/`i32`/`usize`/`isize`; `u64`/`i8`/`i16` | U, ABI | medium |
| P4 | Loops | half-open ranges and numeric `for` (bound evaluated once, measure generated); slice/Vec `for` with borrowed element binder | R03, R10, V | low |
| P5 | Loop control | `break`/`continue`; labels on nested loops (invariant, decreases, drop and borrow write-back on every exit edge) | G-B4-Q1 CFG work | medium |
| P6 | Tuples | value tuples with projection and destructuring; owned payloads incl. `Vec<(Text, Value)>` | V, ABI | medium |
| P7 | Constants and literals | `const` with a closed evaluator; `[T; N]` and array literals (scalar elements first); `char` plus char/byte literals | U, SX, SR | low |
| P8 | Patterns | `if let`/`let else`; `while let`; slice prefix/rest patterns (lowered to `match`) | R03, V | medium |
| P9 | Expressions and calls | `if`/`match` as values; `impl` blocks and methods (static resolution, read/inout/take receiver); type aliases | R03, V | low |
| P10 | B leftovers | last read inside a statement; whole-array place on LLVM; owned-field move out, `replace`/`swap` | G-B4-Q1, G-B5-R10, G-B2-L1 | medium |
| P11 | Owned wrappers | `Option<owned>`, `Result<owned, E>` (types, proofs, layout, drop/move); recursive HashMap; limited generic recursion | V | high |
| P12 | Host boundary | owned collections and typed state across the Rust host; LLVM allocator/ownership ABI design, then implementation per type family | ABI, V4, D83 | high |
| P13 | Closures | non-escaping closures: immutable capture, mutable capture, move/FnOnce (`edit_object` case) | V, B | high |
| P14 | Traits | trait contracts and impl conformance (static dispatch only); `Eq`/`Ord` with proven laws | V, std | medium |
| P15 | Iterators | slice/range iterators with `next`; `enumerate`/`zip`/`rev`; `map`/`filter`/`collect` (fused loops, allocation only in `collect`) | V, U | high |
| P16 | Mutable traversal and collections | `iter_mut`/`split_at_mut`; `clone_from`/`retain`/`drain`; map `entry` | B, C, V, P31 | high |
| P17 | Text | integer parse/format; typed formatting builder (no macros) | P27 | medium |
| P18a | f64 (exact FP) | FP64 sort (RNE, NaN kept), literals, IEEE compare, classify (`is_nan`/`is_finite`), selection (if, clamp, `min`/`max` encoded as ite), arithmetic with constant coefficients, explicit casts with range obligations (`i64(f)`: finite and in [-2^63, 2^63), truncate toward zero); no overflow obligation on f64 arithmetic; NaN payload and sign bit unobservable (no `to_bits`/`copysign`/`is_sign_negative`); Rust/LLVM codegen with no fast-math and no FMA contraction; f64 parse/format | V, P27, ABI | high |
| P18b | f64 range contracts | contracts over variable-by-variable arithmetic (lerp, midpoint, `a<=b => a+c<=b+c`) and mixed Int/FP invariants (sums, round trips); needs a redesign: sound interval facts from rounding monotonicity generated before the query; spike shows plain FP queries exceed rlimit 20M and 60s | P18a | high |
| P19 | Search and sort | `lower_bound`/`binary_search` with a sorted precondition; sort proven sorted and a permutation (one algorithm and type first, then `Ord` generic) | G8, std | high |

`i128`/`u128` are not scheduled until a Lair path needs them.

## 5. Risks

- **Integer widths are not just storage.** The proof model mixes Int arrays with 64-bit BV operations
  (pruvc-rs/crates/pruv-proof/src/model.rs, symbolic.rs). Bounds, signed shifts, contract arithmetic,
  and counterexample output must change together. Default to Int plus per-type range facts; use BV only for bit and
  wrapping operations, and measure proof time after P1.
- **`usize` is target dependent.** Keep the current `len` meaning and the `2^40` cap (TIDY_SPEC §4.2-3);
  state the 32/64-bit rule explicitly.
- **f64 in z3 is unmeasured.** No FP model exists. Modelling IEEE values as reals or silently excluding NaN is not allowed.
  Spike result (2026-11-25, z3 5.3.3, FP(11,53) RNE, rlimit 20M): clamp, compare, min/max, classify and constant-coefficient
  arithmetic prove in 0.4-470 ms; true contracts over variable-by-variable arithmetic and mixed Int/FP queries return unknown
  (over 60 s with no rlimit under default, qffp and bit-blast). FP queries cost 100-1000x integer ones. P18 is split into P18a
  (exact FP, those cheap forms) and P18b (redesign of range contracts).
- **Tuples, closures and iterators create new borrow provenance.** Borrow analysis is tied to names, branches, and the loop graph
  (pruvc-rs/crates/pruv-core/src/borrowed.rs). P5 should leave one core CFG that the later bundles reuse.
- **JSON objects stay `Vec` in V.** Hand Rust relies on order, duplicate keys and last-match updates;
  recursive HashMap support (P11) does not change that representation choice.

## 6. Measuring the plan

Record wall time per bundle and per feature in docs/notes/main_state.md. After three bundles, compare against
the B-bundle average (about 80 minutes per feature) and adjust bundle size.

## 7. Open rows carried in

- PF-1: `check_bundle` rs path checks less than the old py path; bring it to parity (tidy).
- PF-2: closed in PF-2/PF-3: `bridge/node/regen.py` and `.github/workflows/node-bridge.yml` use pruvc-rs only (no pruvc/, no z3-solver). `gen/` was stale since the 2026-11-19 codegen changes and is regenerated; `test.mjs` output is identical before and after. `bench/l0/image_shrink.pruv` renames its helpers `u16`/`u32` (now reserved type names) to `read_u16`/`read_u32`.
- PF-3: closed in PF-2/PF-3: `tests/diff_compilers.py --input` saves a source that declares `mod x;` as a project (whole directory), so `--replay` finds its sibling modules; `tests/check_input_replay.py` (m3_all stage `input_replay`) captures, corrupts and replays examples/g4_geometry/root.pruv. It fails on the old lone-input.pruv save and on a saved case without geo.pruv.
- B8-1: closed in P8a: `type/foreign_borrow` for a value borrowed and moved in one `match`, `if let` or `let else` target (tests/must_reject/p8_borrow_target_{match,if,let}); `while let` joined in P8b (tests/must_reject/p8_borrow_target_while).
- B8-2: B8's verify/llvmgen/proof changes were not audited; audit them with P10.
- P1-1: the type table is built twice when pub/host/foreign functions exist; build it once (tidy).
- P1-2: the LLVM harness accepts a function in proof mode if it has a single `nuw`; check every elided-check site.
- P1-3: closed in P3 (an unsigned index and any-width `decreases` measure, R-6p-5).
- P2-1: `[u8]` <-> `[u16]` transcoding (UTF-8 <-> UTF-16) is not built in; add it as builtins when a workload needs it.
- P2-2: built-in slice order and hash for `[u16]` (hand Rust `a.cmp(b)` / `Hash for [u16]`) and `[u16]` map keys; today they are Pruv loops over `ref [u16]` (P11/P14).
- P2-3: `Vec<u16>`, `Option<[u16]>` and other `u16` type arguments stay rejected (P11).
- P2-4: `[u16]` in pub/extern/foreign signatures stays rejected (P12).
- P2-5: `find` (memchr) on `[u16]` stays rejected (`type_mismatch`); add a u16 search when a workload needs it.
- P2-6: closed in P3 (runtime p2_r_u16_paths, p2_r_u16_fill_oor, p2_r_u16_push_oor, p2_r_u16_field_oor on both backends).
- P2-7: closed in P3: neither showed in compile time (within run-to-run noise); the `[i64]` store proof no longer builds the string and the LLVM `[u16]` scan runs only when no cheaper check already needs DECLS_U8.
- P3-1: arrays of the P3 widths (`[u32]`, `[i32]`, `[usize]`, `[isize]`, `[u64]`, `[i8]`, `[i16]`) stay rejected (`small_int_unsupported`); add them through the element table when a workload needs one.
- P3-2: unary `-`, `!`, bitwise operators and shifts on non-`i64` widths stay `small_int_op`; add with a BV model per width (and wrapping ops) when a workload needs them.
- P3-3: a proven array store with a non-`i64` index converts it through `i64` before `get_unchecked_mut`; emit `as usize` directly (Rust shape only, no extra check).
- P3-4: `Vec`, `str` and hash map indexes, and signed non-`i64` array indexes, still need `i64`; the range-bound part is closed in P4 (`for i in a..b` takes any P3 width, R-6q-1).
- P3-5: non-`i64` widths at the host boundary (pub/extern/foreign signatures) stay P12.
- P3-6: a `u64` index above i64 max prints a negative value in the out-of-bounds message, and the overlap `uadd` declaration filter path has no test (P3 audit recommendations).
- P4-1: `enumerate`, `zip`, `rev`, `.iter()` and other adapters are rejected (`type/for_enumerate`, `parse/for_iterable`); write `for i in 0..len(a)` with `a[i]` until P15.
- P4-2: `for x in v` over `Vec<T>` with a non-scalar `T` (`Vec<str>`, `Vec<Vec<T>>`, structs) is rejected (`type/for_iterable`); the borrowed binder of an owned element joins P11/P16.
- P4-3: inclusive ranges `a..=b` are rejected (`parse/for_iterable`); add them with an end-at-max rule when a workload needs one.
- P4-4: the binder of `for x in a` over `[u8]`/`[u16]` is `i64` (the type of an index read today), so the Rust loop is `for &x in a.iter() { let x = x as i64; ... }`; hand Rust keeps `x: u8`. Same machine code after the widen; change it when element reads keep their width.
- P4-5: `for x in v` over `Vec<T>` is Rust only until LLVM has `Vec` (`codegen/unsupported_type`, P12).
- P4-6: closed in P5 (`break`/`continue` in `for`, R-6r-1; Rust emits native jumps in the native `for`, so the hidden counter and bound stay unread).
- P4-7: an outer loop's `index(x)` is not in scope in an inner loop's invariant; state the outer position through the outer counter or a local copy.
- P4-8: an invariant ending in an uppercase name before `{` parses as a struct literal; parenthesize it until the invariant is parsed without struct literals.
- P4-9: `for x in a` over a generic `[T]` (T an integer or bool after monomorphization) is accepted but not yet stated in R-6q-2.
- P4-10: assigning the slice binder reports the `let mut` hint of an immutable local; a for-binder message is queued.
- P4-11: missing tests from the P4 audit: reverse range `a > b` at runtime, `u8` 255 / `i8` / `i64` max bounds, `Vec<u8>`/`Vec<bool>`/`Vec<u32>` runtime, nested for over the same collection, binder-assignment reject.
- P5-1: `break` with a value is `parse/break_value` and `loop { .. }` is `parse/loop_unsupported` (P15); write a measured `while` with a result local.
- P5-2: a `break`/`continue` that would leave a map iteration `for (k, v) in m` or a mutable collection `Some` arm is `type/loop_jump_map`; lift it when map iteration and the `Some` arm write-back have per-edge exits.
- P5-3: iterator adapters with `break` (`take_while`, `position`, `find`) stay P15.
- P6-1: closed in P8a: tuple patterns in `match` arms, `Some((a, b))`, `E::V((a, b))` and a top-level `(a, b)` arm (R-6u-1); `parse/tuple_match_pattern` is retired.
- P6-2: tuple types in programs with `pub` or `mod` items are `parse/tuple_module` (the synthetic tuple structs have no module identity yet); tuples at the pub/extern/foreign boundary join P12.
- P6-3: tuples with a `String` (or another owned collection LLVM cannot store as a struct field) on LLVM are `codegen/tuple_llvm_owned` (P12); array, `Vec<i64>` and `Vec<bool>` components run on LLVM like struct fields.
- P6-4: tuple structs `struct P(A, B);` are `parse/tuple_struct`; add them when a workload needs one.
- P6-5: `for x in v` over `Vec<(A, B)>` with owned components stays `type/for_iterable` (P15/P16, same as P4-2).
- P6-6: tuple types inside generic declarations are `parse/tuple_generic`; `str` components cannot be moved out (not a field-movable type), use `String`.
- P6-7: a projection move of a buffer component `let k = move t.0;` (t.0: String) emits `::std::mem::take(&mut t.0)` (the existing field-move shape) where hand Rust writes `let k = t.0;` (a tuple-typed component and destructuring temps use the partial move); a struct literal component prints with extra parentheses `((S { .. }), 4)`; counterexamples show the internal tuple struct name.
- P6-8: destructuring decides moves by the component spelling: a named struct with an owned field is read through a clone (needs `! alloc`, and the fold back to `let (a, b) = t;` is lost), and a user type named `String`/`Vec` counts as owned; move them by name once struct ownership is tracked per type. Not done in P8a (2 h cap); P8b kept to slice patterns and `while let`, so this row is carried to P10b (the move bundle).
- P6-9: components outside the field-move set cannot be moved out by destructuring: `Vec<String>`, `str` and array-holding tuples (`([i64], i64)`) are `type/foreign_move`, and such tuples as `Vec` elements are `type/foreign_owned_type` (P15, with the owned-collection widening).
- P6-10: a view origin path `from(t.0)` is `parse/syntax` (only named fields); `t.0x1` and `t.1_` give `parse/syntax` and `parse/bad_int` instead of `parse/tuple_index`; a component-count mismatch against a known tuple type reports `type/tuple_needs_type`; `for x in v` over `Vec<(i64, bool)>` is `type/for_iterable` (only integer and bool elements iterate).
- P6-11: `move t.1` of a tuple component behind an `inout` tuple parameter still lowers to `std::mem::take`, so generated Rust fails rustc (E0277) when the component type has no `Default` (e.g. a user enum or struct); a local tuple uses a partial move. Hand Rust would use `std::mem::replace` with the reassigned value; queued for the move-lowering bundle.
- P8a-1: a tuple pattern in a match arm binds only integer, `bool` and `char` components (`type/tuple_pattern_owned`); owned components go through a destructuring `let` (R-6s-3). Lift with P6-8's per-type ownership.
- P8a-2: `if let`/`let else` on a borrowed (non-Copy) enum or collection target, on a mutable collection target (`map_get_mut`, `vec_at_mut`) and on `str`/view-error arms print a full `match` in Rust (the match_ext/match_map paths), and tuple patterns on view or string-error arms print the binder plus component `let`s; same behaviour, not the hand-Rust shape.
- P8a-3: `else if let` prints as `else { if let .. }` in Rust (rustfmt-equivalent, one extra block).
- P8a-4: the checker clones the `if let`/`let else` else block into each complement arm (one copy per missing variant of a user enum); the continuation after `let else` is held once. A chain of `else if let` copies grows exponentially in the LLVM IR and the proof (audit: an 8-variant enum chained 5/6/7 deep gives 2.4 MB / 17 MB / 123 MB of IR, depth 7 verifies in 38 s); the Rust output stays the source form. Share the else block (one arm with a wildcard) before deep chains are common.
- P8a-5: the design's per-binder borrow provenance for tuple components (borrowed.rs `build` beyond the first binder) is not needed while components are scalars (P8a-1); revisit with P8a-1.
- P8b-1: a slice `match` prints in Rust as the checker's chain of `len(..)` tests with element `let`s and a `subview` rest, not the source slice pattern (same behaviour; the bounds checks are elided by proof).
- P8b-2: a slice pattern target must be an array place (`[i64]`, `[u8]`, `[u16]`, `[bool]` or a view of one); a call result needs a `let` first and `Vec<T>` targets are not accepted (`type/match_type`).
- P8b-3: slice patterns only appear as top-level `match` arms: not in `if let`/`let else`/`while let`, not nested in a constructor or tuple pattern, and with no suffix or middle rest (`[.., x]`, `[a, .., z]` are `parse/slice_pattern_rest`).
- P8b-4: `while let` prints the source form in Rust except in cost mode and on the full-`match` paths of P8a-2 (borrowed or mutable collection targets, `str`/view-error arms), where it prints `while true { match e { P => { .. } _ => break } }`. In cost mode the iteration unit is charged at the top of the pattern arm (a one-pass `while true { break; }`), not on the loop line, so a failed head costs no iteration in either backend (tests/runtime/p8_rt_while_let_cost.pruv).
- P8b-5: a `while let` pattern that matches every value (the only variant of an enum) is `type/irrefutable_pattern`, where Rust accepts it with a warning as an endless loop.
- P8a-6: `let crate::m::E::A(x) = e else { .. }` is rejected by the lexer (`crate` after `let` stays a reserved name,
  lexer.rs `g4_token`, mirrored from vparser.py), and so is `if let crate::..`; `match` arms accept the path. Open until
  the lexer rule and its Python mirror are changed together.
- P8a-7: a payload pattern with more binders than the variant has fields under `Some`/`Ok`/`Err` (`Some(a, b)`) is
  `parse/syntax` ("expected ')'") rather than `type/match_arity`; this predates P8 (same on main) and stays open.
- P7-1 A: done (2026-11-26). `[T; N]` is a local value (`let` annotation only) with scalar integer, `char` or `bool` elements, the literals `[a, b, c]` / `[x; N]` and indexing (R-6t-4). Rust emits `[T; N]` by value and `get_unchecked` for proven indexes (no widening of narrow reads); LLVM a `[N x E]` stack slot (bool `i8`, char `i32`, `memcpy` copies, the right side of a self-assignment fully evaluated first); the proof models an SMT array with the constant length `N`, so constant indexes drop their bounds checks. 64 KiB per array (`type/fixed_array_length`), `N = 0` allowed. Owned elements are `type/fixed_array_element`, whole-array `==` `type/fixed_array_operation`. A zero-length literal is `[x; 0]`: `[]` stays the static empty view, so `let e: [i32; 0] = []` is `type/type_mismatch`.
- P7-1 B: in progress. Step 1 done (2026-11-26): `[T; N]` parameters and results by value, `inout a: [T; N]` (element range kept across the call, `old(a)`/`old(a)[i]`), the call-duration lend of a `[T; N]` as a read-only `[T]` (`[i64]`, `[u8]`, `[u16]`, `[bool]`) to a Pruv function's plain parameter (not `take`, `inout`, extern, a `from` function or a call in a contract: `type/fixed_array_operation`), and `type/fixed_array_position` for a fixed array in a `pub`/host entry/extern signature (legacy mode keeps such functions internal). Rust passes `[T; N]` by value and lends `&a[..]`; LLVM passes pointers (fresh caller copy, caller slot for `inout`, caller storage for the result) and lends the slot with the constant length. Step 2a done (2026-11-26): `[T; N]` fields of non-generic structs (by value; Rust a `[T; N]` field, LLVM `[N x E]` inline; proof through the `Buf(len, arr)` field encoding with `len == N` and the element range; field-path reads/writes, `inout` and nested struct paths, `old(s).f[i]`; the host-signature check looks through fields). Step 2b done (2026-11-26): `for x in a` over a `[T; N]` local, parameter or field (R-6q-2; mutating `a` in the body is `type/recursive_borrow`, a literal iterable `parse/for_iterable`). Step 2c done (2026-11-26): fixed-array literals `[a, b]` / `[x; N]` in requires, ensures, assert, invariant, decreases and quantifiers (Rust an `[i128; N]` contract array, LLVM the elements in contract form with a select on the index; the index must be proven in range, `contract_domain` in requires). Step 2d done (2026-11-26): `let d = inout if c { a } else { a }` and a choice between fixed fields of one struct on LLVM (it was an internal error), passing a place binding on as `inout d`, the lend to generic `[T]` functions, and the codes for `copy(a)` (`type/type_mismatch`), `move a` (`type/foreign_move`) and `par` arguments (`type/parallel_argument`); the extra LLVM copies are P7-7. Remaining: generic/enum fields, tuple components, type arguments, nested arrays, slice patterns, and the extra proof tests (negative and `u64 > i64::MAX` indexes, zero-length reads, overflow in length expressions, literals in invariants/quantifiers).
- P7-2: a constant is emitted with its evaluated value (`const HALF: i64 = 2048;`), not the source expression (`SIZE / 2`) hand Rust keeps; uses are spelled by name in code, by value in run-time contract checks.
- P7-3: `pub const` and constants in programs with `pub` or `mod` items are `parse/const_module` (module paths `m::NAME` and visibility of constants are not designed); join P12 with the tuple module row P6-2.
- P7-4: only integer-width and `char` constants exist; `bool`, tuple, struct and string constants are `type/const_type`, and calls (including future `const fn`) are `type/const_expr`. A constant `/` or `%` with a negative operand is `type/const_expr` (Rust's `const` truncates, e.g. `-7 / 2` is `-3`; Pruv keeps its run-time domain).
- P7-5: `char` is not yet a type argument (`Option<char>`, `Vec<char>`: `parse/small_int_unsupported`, P11), an array element (`[char]`, P3-1) or a match-literal pattern; `print(c)` prints the code point (`println!("{}", c as i64)` on Rust, `pruv_print_i64` on LLVM) where hand Rust prints the character; `string_push_char` still takes an `i64` code (R-6g8-2) instead of a `char`. A byte literal `b'a'` is `u8` while `[u8]` and `str` elements read as `i64`, so a comparison writes `i64(b'a')`.
- P7-6: `parse/const_shadow` is stricter than Rust: it also rejects a struct field, struct or enum name equal to a constant name, which Rust keeps in other namespaces. A constant name must be upper case (`parse/const_name`), where Rust only warns.
- P7-7 (perf debt): LLVM passes a by-value `[T; N]` argument as a pointer to a fresh caller copy and returns a result through caller storage that `let c = g(a)` then copies into `c`'s slot. After inlining `opt -O3` removes the caller copies (a 64-element probe keeps one 512-byte `memcpy`, the callee's own `let mut b = a`), but a call that is not inlined pays both. Skip the argument copy when the callee never writes the parameter and no `inout` argument of the same call aliases it, and write the result straight into the destination slot.
- CI-1: closed (2026-11-25). `tests/inference/inf_budget.pruv --infer` was load dependent: the 500 ms per-query and 5 s per-function wall-clock limits of inference turned queries `unknown`/`timeout` under load, so the trace hash and `errs` changed between runs (main CI 45560012284). Rule: in inference the z3 resource limit (`RLIMIT` 100000) decides each query and `MAX_QUERIES` (512, with the fallback round) decides each function; the wall clocks (`TIMEOUT_MS` 10000 per query, `MAX_SECONDS` 60 per function, `pruvc-rs/crates/pruv-proof/src/infer.rs`) are safety nets far above any test (inf_budget, 1024 queries, runs in about 3 s), so a test that hits them is a bug. Unloaded results are unchanged (all 41 inference cases equal modulo the recorded timeout); 3 runs under 6 busy processes plus background QoS give the same trace hash. The `LOAD_DEPENDENT` interim in `tests/diff_trace.py` is removed. The frozen Python `pruvc/` keeps 500 ms / 5 s. CI-1a and CI-1b: closed (2026-11-25). `tests/must_reject/lg4_agg_frame_changed.pruv` flaked because its third attempt (counterexample retry with aggregate definitions) hit the 8000 ms verification wall clock, and `tests/runtime/g10a_r21_backpressure.pruv --infer` printed varying counterexample values because the 2000 ms wall-clock cap of the shared-context first check in `sys::Solver::check` (LG21) decided which context, and so which model, answered. Rule: in verification the z3 resource limit (`symbolic::RLIMIT` 20M) decides each query, and the shared-context first check is capped by rlimit (`SHARED_FIRST_RL` 10M) instead of time; the wall clock (`symbolic::TIMEOUT_MS` 60000) is a safety net far above any test. 20M was measured: the largest decided verification query of the diff_trace groups (all 1989 inputs, with and without `--infer`, plus the l3/spec probes) uses about 7.8M (g10a_r21 `--infer`), so 2.5x headroom; inference keeps its rlimit (`infer::RLIMIT` 100000, below the cap), but its shared-context first check now gets the full 10 s wall-clock safety net instead of the 2 s cap. z3 reports an rlimit stop as `canceled` or a resource-limit reason (matched case-insensitively); `pruvc-rs/crates/pruv-proof/tests/rlimit_check.rs` pins the capped retry and full-rlimit exhaustion with a pigeonhole query. No verdict changes: old and new binaries give equal attempt results and error codes/lines/messages on all 1989 inputs, except lg4, whose third attempt now ends `unknown` by the rlimit (note reason `canceled`) instead of sat-or-timeout depending on load; same error. Every trace digest changes through the head `(set-option :timeout 60000)` / `(set-option :rlimit 20000000)` and `timeout_ms`; goldens refreshed with equal obligation counts. 3 runs each under 6 busy processes give one trace hash for lg4 and g10a_r21 `--infer` (the old binary gives two for g10a_r21). The frozen Python `pruvc/` keeps 8000 ms.
