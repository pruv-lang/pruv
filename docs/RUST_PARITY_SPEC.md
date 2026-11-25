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
- P1-3: `u8`/`u16` used directly as an index or in `decreases` needs `i64(x)`; allow it in P3/P4.
- P2-1: `[u8]` <-> `[u16]` transcoding (UTF-8 <-> UTF-16) is not built in; add it as builtins when a workload needs it.
- P2-2: built-in slice order and hash for `[u16]` (hand Rust `a.cmp(b)` / `Hash for [u16]`) and `[u16]` map keys; today they are Pruv loops over `ref [u16]` (P11/P14).
- P2-3: `Vec<u16>`, `Option<[u16]>` and other `u16` type arguments stay rejected (P11).
- P2-4: `[u16]` in pub/extern/foreign signatures stays rejected (P12).
- P2-5: `find` (memchr) on `[u16]` stays rejected (`type_mismatch`); add a u16 search when a workload needs it.
- P2-6: missing `[u16]` tests: `--no-verify` runtime stores out of range (`resize` fill -1, `arr_push` 65536, struct field), and `[u16]` through `inout` calls, struct fields and `par` block boundaries (the `par` path is open but untested). Add with P3.
- P2-7: the `[u16]` scan runs for every program, and `[i64]` store proofs build an unused diagnostic string; measure compile time in P3 and drop both if they show.
