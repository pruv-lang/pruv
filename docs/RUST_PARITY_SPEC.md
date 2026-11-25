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
| P18 | f64 | first a z3 FP spike (proof time on sample contracts); then FP64 sort, literals, classify, arithmetic/compare, explicit casts; then Rust/LLVM codegen with no fast-math and a fixed NaN payload rule; then f64 parse/format | V, P27, ABI | high |
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
  P18 starts with a measured spike; if proofs are too slow, the plan for P18 changes before any implementation.
- **Tuples, closures and iterators create new borrow provenance.** Borrow analysis is tied to names, branches, and the loop graph
  (pruvc-rs/crates/pruv-core/src/borrowed.rs). P5 should leave one core CFG that the later bundles reuse.
- **JSON objects stay `Vec` in V.** Hand Rust relies on order, duplicate keys and last-match updates;
  recursive HashMap support (P11) does not change that representation choice.

## 6. Measuring the plan

Record wall time per bundle and per feature in docs/notes/main_state.md. After three bundles, compare against
the B-bundle average (about 80 minutes per feature) and adjust bundle size.

## 7. Open rows carried in

- PF-1: `check_bundle` rs path checks less than the old py path; bring it to parity (tidy).
- PF-2: `bridge/node/regen.py` and `.github/workflows/node-bridge.yml` still use the Python compiler and z3; move to rs.
- PF-3: multi-file `--input` replay in the golden harness is recorded, not yet covered.
- B8-1: moving a borrowed match target reports `borrow_conflict` (py) vs `foreign_borrow` (rs); the golden keeps rs. Decide the code in P8.
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
- P6-1: tuple patterns in `match` arms, `Some((a, b))` and `E::V((a, b))` are `parse/tuple_match_pattern`; add them with `if let`/`let else` in P8.
- P6-2: tuple types in programs with `pub` or `mod` items are `parse/tuple_module` (the synthetic tuple structs have no module identity yet); tuples at the pub/extern/foreign boundary join P12.
- P6-3: tuples with a `String` (or another owned collection LLVM cannot store as a struct field) on LLVM are `codegen/tuple_llvm_owned` (P12); array, `Vec<i64>` and `Vec<bool>` components run on LLVM like struct fields.
- P6-4: tuple structs `struct P(A, B);` are `parse/tuple_struct`; add them when a workload needs one.
- P6-5: `for x in v` over `Vec<(A, B)>` with owned components stays `type/for_iterable` (P15/P16, same as P4-2).
- P6-6: tuple types inside generic declarations are `parse/tuple_generic`; `str` components cannot be moved out (not a field-movable type), use `String`.
- P6-7: a projection move of a buffer component `let k = move t.0;` (t.0: String) emits `::std::mem::take(&mut t.0)` (the existing field-move shape) where hand Rust writes `let k = t.0;` (a tuple-typed component and destructuring temps use the partial move); a struct literal component prints with extra parentheses `((S { .. }), 4)`; counterexamples show the internal tuple struct name.
- P6-8: destructuring decides moves by the component spelling: a named struct with an owned field is read through a clone (needs `! alloc`, and the fold back to `let (a, b) = t;` is lost), and a user type named `String`/`Vec` counts as owned; move them by name once struct ownership is tracked per type (P8).
- P6-9: components outside the field-move set cannot be moved out by destructuring: `Vec<String>`, `str` and array-holding tuples (`([i64], i64)`) are `type/foreign_move`, and such tuples as `Vec` elements are `type/foreign_owned_type` (P15, with the owned-collection widening).
- P6-10: a view origin path `from(t.0)` is `parse/syntax` (only named fields); `t.0x1` and `t.1_` give `parse/syntax` and `parse/bad_int` instead of `parse/tuple_index`; a component-count mismatch against a known tuple type reports `type/tuple_needs_type`; `for x in v` over `Vec<(i64, bool)>` is `type/for_iterable` (only integer and bool elements iterate).
- P6-11: `move t.1` of a tuple component behind an `inout` tuple parameter still lowers to `std::mem::take`, so generated Rust fails rustc (E0277) when the component type has no `Default` (e.g. a user enum or struct); a local tuple uses a partial move. Hand Rust would use `std::mem::replace` with the reassigned value; queued for the move-lowering bundle.
- CI-1: closed (2026-11-25). `tests/inference/inf_budget.pruv --infer` was load dependent: the 500 ms per-query and 5 s per-function wall-clock limits of inference turned queries `unknown`/`timeout` under load, so the trace hash and `errs` changed between runs (main CI 45560012284). Rule: in inference the z3 resource limit (`RLIMIT` 100000) decides each query and `MAX_QUERIES` (512, with the fallback round) decides each function; the wall clocks (`TIMEOUT_MS` 10000 per query, `MAX_SECONDS` 60 per function, `pruvc-rs/crates/pruv-proof/src/infer.rs`) are safety nets far above any test (inf_budget, 1024 queries, runs in about 3 s), so a test that hits them is a bug. Unloaded results are unchanged (all 41 inference cases equal modulo the recorded timeout); 3 runs under 6 busy processes plus background QoS give the same trace hash. The `LOAD_DEPENDENT` interim in `tests/diff_trace.py` is removed. The frozen Python `pruvc/` keeps 500 ms / 5 s. CI-1a and CI-1b: closed (2026-11-25). `tests/must_reject/lg4_agg_frame_changed.pruv` flaked because its third attempt (counterexample retry with aggregate definitions) hit the 8000 ms verification wall clock, and `tests/runtime/g10a_r21_backpressure.pruv --infer` printed varying counterexample values because the 2000 ms wall-clock cap of the shared-context first check in `sys::Solver::check` (LG21) decided which context, and so which model, answered. Rule: in verification the z3 resource limit (`symbolic::RLIMIT` 20M) decides each query, and the shared-context first check is capped by rlimit (`SHARED_FIRST_RL` 10M) instead of time; the wall clock (`symbolic::TIMEOUT_MS` 60000) is a safety net far above any test. 20M was measured: the largest decided verification query of the diff_trace groups (all 1989 inputs, with and without `--infer`, plus the l3/spec probes) uses about 7.8M (g10a_r21 `--infer`), so 2.5x headroom; inference keeps its rlimit (`infer::RLIMIT` 100000, below the cap), but its shared-context first check now gets the full 10 s wall-clock safety net instead of the 2 s cap. z3 reports an rlimit stop as `canceled` or a resource-limit reason (matched case-insensitively); `pruvc-rs/crates/pruv-proof/tests/rlimit_check.rs` pins the capped retry and full-rlimit exhaustion with a pigeonhole query. No verdict changes: old and new binaries give equal attempt results and error codes/lines/messages on all 1989 inputs, except lg4, whose third attempt now ends `unknown` by the rlimit (note reason `canceled`) instead of sat-or-timeout depending on load; same error. Every trace digest changes through the head `(set-option :timeout 60000)` / `(set-option :rlimit 20000000)` and `timeout_ms`; goldens refreshed with equal obligation counts. 3 runs each under 6 busy processes give one trace hash for lg4 and g10a_r21 `--infer` (the old binary gives two for g10a_r21). The frozen Python `pruvc/` keeps 8000 ms.
