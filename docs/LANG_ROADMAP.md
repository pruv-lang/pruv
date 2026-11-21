# Pruv language roadmap (gaps against Rust)

Status: design estimate from a 2026-11-22 design pass. Nothing here is implemented, tested or measured unless a later
bundle doc (docs/L3_LG*.md) says so. Bundle counts and speed gains are estimates.

## Gap table

| Gap | Bundles | Wave |
|---|---:|---|
| 3. Typed error Result (E1-E3) | 12 | 1 |
| 9. LLVM parity (L01-L32) | 32 | from 1, in parallel |
| 6. General references (B1-B6, `struct Line<'a>`) | 24 | 2 read, 5 write |
| 2. Struct and collection composition (C1-C5) | 20 | 2 value records, 4 owned |
| 8. Formatting and characters (S1-S4) | 16 | 3 / 5 |
| 1. Integers, char, f64 (N1-N7) | 28 | 3 for N1-N5, 8 for N6-N7 |
| 7. Type argument inference (I1-I3, F only) | 3 | 4 |
| 4. Generic interfaces (T1-T4) | 16 | 6 |
| 5. Closures, iterators, for (H1-H7) | 28 | 5 for H1-H2, 7 for H3-H7 |
| Total | 179 | one feature = four bundles F, P, R, L; about 2 hours per bundle |

First porting targets: JSON Lines reading (bench/l3/pkgs/lines/lines.pruv) and RPC failure returns
(bench/l3/codex/rpc.pruv).

## Key points per feature

- E (typed errors): `Result<T, E>`; the existing `Result<T>` is normalized to `Result<T, i64>` (ABI kept). One Z3 ADT per
  (T, E). The recursive size computation that only looks at the Ok side must also look at the Err side. E1 value-record
  errors, then E2 owned String errors, then E3 error records that own buffers.
- L (LLVM parity): L01-04 Vec clear reuse, extend_from_slice, Vec to slice (code and contracts). L05-08 string_eq,
  Vec<String> elements, trusted Vec<String> results, string-keyed maps. L09-13 map insert, remove, take, get_mut, for.
  L14-18 owned collection fields, enum payloads, nested holders, Vec<Record>, push/reserve. L19-21 `--no-verify`
  contracts for v[i], map_lookup, map_updated. L22-25 take record slot, host take [T], take String, Vec results. L26-30
  resident inout S handles, buffer replacement, owned record/enum results, record arguments (G-R07-1 and G-LG3-3 close
  with L26+L27). L31-32 trusted Option/Result arguments and results.
  G-LG8-5 is a static finding only (no recorded run failure); confirm it, then do the G-LG8-3 field move. Do not assume
  Rust Vec/String/enum layouts are C layouts.
- B (references): the type checker tracks root, path, index and lifetime; Z3 sees root SSA plus projections. B1 local
  read of a field, B2 element, B3 returning a borrow of an input (lifetime) and fields, B4 write to a field and reborrow,
  B5 write to an element with disjoint indices, B6 writes across calls, branches and loops.
- C (composition): `Option<Point>` already exists (only the stale wording in SPEC §6a needs a fix; see
  tests/runtime/g3_r15_option_point.pruv). C1 arrays of value structs, C2 replacement and [T], C3 Option with owned
  payload, C4 Result with owned success value, C5 collections inside arrays. Never copy owned elements silently.
- S (formatting): `format("...{}", a)` with `! alloc`, `for c in chars(s)`. S1 literal, str and String insertion; S2
  integers and bool; S3 char and UTF-8 cursor (needs N5); S4 passing the scanner-range UTF-8 fact (G-DB15-1, needs the
  same bytes and same range linked). Watch the sign of i64::MIN.
- N (numbers): explicit conversion `to::<U>(x)`. Integers are Z3 Int plus a proof of the target range; bit operations use
  BV. The usize width comes from the target and is part of the cache key. f64 is FloatingPoint(11,53) with RNE; no
  fast-math, no implicit FMA, no Real substitute. N1 u8, N2 i32/u32, N3 u64, N4 usize, N5 char, N6 f64, N7 FP to/from
  integer.
- I (inference): arguments are decided by structural unification; Z3 never picks types. Unannotated None/Ok/Err keep
  their i64 compatibility. I1 arguments, I2 let, return and constructors, I3 nesting and ambiguity.
- T (interfaces): named calls such as `Ordered::less(a, b)`, one impl per type, no `+` overloading. The impl is proved to
  keep the contract. f64 is not a total order. T1 equality, T2 contract linking, T3 order laws, T4 arithmetic premises.
- H (iteration): an array iterator is (snapshot, cursor, end). A closure is code plus captures; effects propagate through
  the signature. H1 for over arrays, H2 next and termination, H3 closures without captures, H4 contract- and
  effect-aware higher-order calls, H5 read borrows and move captures (needs B), H6 map_iter, H7 filter and single-use
  loops.

## Order and parallel lanes

| Wave | Features | In parallel |
|---|---|---|
| 1 | E1, E2 | L01-05, L22 |
| 2 | B1-B3, C1-C2 | L06-08, L14, L26-27 |
| 3 | S1-S2, N1-N5 | L09-12, L15-18 |
| 4 | C3-C5, E3, I1-I3 | L19-21, L23-25, L28-32 |
| 5 | B4-B6, H1-H2, S3-S4 | L13 |
| 6 | T1-T4 | P/R/L of earlier features |
| 7 | H3-H7 | P/R/L split |
| 8 | N6-N7 | FP encoding and both emitters |

## Lane file ownership (one worker per lane at a time)

- F (front end): pruvc/{vast,vparser,check,typeinfo,mono}.py and pruv-core/{ast,lexer,parser,check,typeinfo,gtype,mono}.rs.
  Pairs: E1.F+L01, I2.F+L19, H1.F+L13.
- P (proof): pruvc/verify.py and pruv-proof/{symbolic,model,obligation,sys}.rs. B2.P and N1.P run in series.
- R (Rust output): pruvc/codegen.py and pruv-core/codegen.rs.
- L (LLVM output): pruvc/llvmgen.py and pruv-core/{llvmgen,bindings,abi}.rs, runtime and shim. L26+L27 and L26+C1.L run
  in series (bindings also live in llvmgen.py).
- SPEC.md and the shared test lists are never edited by two workers at once. Workers hand in rule text and cases; the
  merging side applies them.

## Done rule

A feature is done when both compilers reject the same invalid programs, both backends give the same results, and every
check claimed as removed is gone from the generated code.
