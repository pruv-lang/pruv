# Pruv language spec v0.1

Pruv is a tiny language designed to be written by AI. Humans read only the contracts
(`requires` / `ensures`). The compiler proves every function correct against its contracts
with an SMT solver (z3), proves every array access in bounds and every arithmetic
operation free of i64 overflow, then emits Rust with all those runtime checks removed.
If anything cannot be proven, compilation fails with a JSON error containing a counterexample.

Design rules: one way to do each thing; everything a function does is visible in its
signature; no null, no exceptions, no implicit conversions, no globals, no overloading.

## 1. Lexical
- Comments: `// ...` to end of line.
- Identifiers: `[a-zA-Z_][a-zA-Z0-9_]*`.
- R-1-2 Integer literals: a decimal digit followed by digits and `_` separators (`1_000_000`; repeated separators such as `1__0` are accepted and mean `10`). A literal ending in `_` is rejected (`parse/bad_int`). Tests: [accept](tests/spec/r1_2_underscore.pruv), [reject](tests/spec/r1_2_trailing_underscore.pruv).
- Keywords: `fn let mut if else while return match true false cost spent invariant decreases requires ensures assert forall exists in Some None Ok Err struct enum is_variant variant_field async`.
- R-1-1 `result` is not a keyword: it is an ordinary name (a local may be called `result`), and inside `ensures` it names the return value. Used as the return value where there is none yet (for example in `requires`), it is rejected (`type/result_scope`). Tests: [accept](tests/spec/r1_1_result_name.pruv), [reject](tests/spec/r1_1_result_scope.pruv).
- Names starting with `pruv_` or containing `__`, the name `_`, and Rust keywords (`type`, `loop`, `self`, `for`, ...) are reserved; the parser rejects them (`reserved_name`).

## 2. Types
| Pruv | meaning | Rust |
|---|---|---|
| `i64` | 64-bit signed int | `i64` |
| `bool` | boolean | `bool` |
| `()` | unit | `()` |
| `[i64]` | array of i64 | param: `&[i64]`, local/return: `Vec<i64>` |
| `[u8]`, `[bool]` | array of bytes 0..=255 / of bools (section 6b) | `&[u8]`/`Vec<u8>`, `&[bool]`/`Vec<bool>` |
| `inout a: [i64]` (also `[u8]`, `[bool]`, `[T]`) | parameter mode, not a type (section 6g) | `&mut [i64]` (`&mut [u8]`, `&mut [bool]`) |
| `str` | read-only UTF-8 text, indexed by byte (section 6b) | param: `&str`, local/return: `String` |
| `Option<i64>` | `Some(v)` or `None` | `Option<i64>` |
| `Result<i64>` | `Ok(v)` or `Err(code)`, code is i64 | `Result<i64, i64>` |
| `Result<T, E>` | `Ok(v)` or `Err(e)`, e is an `E` (section 6c, LG10 E1) | `Result<T, E>` |
| `Point`, `Event` | user struct / enum (section 6a) | `pub struct` / `pub enum` (Copy) |

Parameters cannot be reassigned. Array parameters are read-only views, except an `inout` array parameter (section 6g),
which is an exclusive mutable view: its elements can be assigned, but the parameter itself never can. Otherwise only
`let mut` locals can be reassigned or have elements assigned.

## 3. Functions
```
fn name(p1: T1, p2: T2) -> R ! effect1, effect2
  requires <bool expr>
  ensures <bool expr>      // may use `result`
{
  ...
}
```
- Effects: `io` (may call `print`), `alloc` (may call `alloc`/`copy` or a function with `alloc`).
  A function must list every effect it uses, directly or through calls. No `!` part means pure.
- Any number of `requires` and `ensures` lines, combined with `&&`.
- Every path must end with `return e;` (for `()` functions `return;` may be omitted at the end).
- Program entry, if present, is `fn main() -> () ! io ...`.
- A parameter may be written `inout name: [T]` for an array the function updates in place; the call must repeat
  the mark (`f(inout a)`). See section 6g.
- Calls are checked modularly: the caller must prove the callee's `requires`, and then may assume its `ensures`.
- Recursion (direct or mutual) is allowed when every function in the cycle has a `decreases` clause (section 6e).

## 4. Statements
```
let x: T = e;            // type annotation required
let mut x: T = e;
x = e;                   // only mut locals
a[i] = e;                // only mut local arrays and inout array parameters
if c { ... } else { ... }        // else optional; `else if` allowed
while c
  invariant <bool expr>          // zero or more
  decreases <i64 expr>           // exactly one, required
{ ... }
return e;  /  return;
assert <bool expr>;              // proven at compile time, erased
print(e);                        // e: i64 or bool; needs `io`
match e {                        // e: Option<i64>, Result<i64> or a user enum (6a); all arms required
  Some(v) => { ... }
  None => { ... }
}
match r { Ok(v) => { ... } Err(c) => { ... } }
f(args);                 // call as statement only if f returns ()
f(inout a, args);        // mutating call: subject to the expression rules of section 6g
```
Read-only map iteration is specified by R-6e-13. A value of type `Result<i64>` or `Option<i64>` must be used (let-bound, returned, matched);
it can never be silently dropped.

## 5. Expressions
- Literals `123`, `true`, `false`; variables; `f(a, b)`; `a[i]`; `len(a)` (i64).
- Arithmetic `+ - * / %` on i64. `/` and `%` require proving divisor `> 0` and dividend `>= 0`.
- Bitwise `& | ^ << >>` and unary `~` on i64.
- Comparison `== != < <= > >=` (i64, and `== !=` on bool). In contracts `== !=` also work on Option/Result.
  Arrays can never be compared with `==`; compare `len` and elements with `forall`.
- Logic `&& || !` (short-circuit). Unary `-`.
- Constructors `Some(e)`, `None`, `Ok(e)`, `Err(e)`.
- `alloc(n, v)`: new array of length n (0 <= n <= 2^40 must be proven) filled with v. Needs `alloc`.
  To grow an array, bound the input: `requires len(a) <= 1_000_000` before `alloc(len(a) + 1, 0)`.
- `copy(a)`: new array equal to a. Needs `alloc`.
- `truncate(a, m)`: new array holding the first m elements of `a` (`0 <= m <= len(a)` must be proven, `verify`
  error `index_oob`). Needs `alloc`. Arrays only (`str` is a type error). When `a` is a local at its last use the
  generated code (Rust and LLVM) shortens it in place instead of copying; this is not observable.
- `resize(a, n, v)`: new array of length n whose first `len(a)` elements are those of `a` and whose remaining
  `n - len(a)` elements are v. `len(a) <= n <= 2^40` must be proven (`verify` error `index_oob`); for `[u8]` also
  `0 <= v <= 255` (`elem_range`, even when `n == len(a)`). The verifier knows `len(result) == n`,
  `result[k] == a[k]` for `k < len(a)` and `result[k] == v` for `len(a) <= k < n`. Needs `alloc`, code only
  (`effect_in_contract` in a contract). `a` is `[i64]`, `[u8]` or `[bool]` and the result has the type of `a`; v has
  its element type. Generic `[T]` and `str` are type errors. Arguments are evaluated a, n, v. The generated code makes
  one allocation of n, copies the prefix once and fills only the tail, so every element is written once. In the Rust
  backend the statement `x = resize(x, n, v)` on a `let mut` local or a resizable `inout` array grows `x` in place
  (`Vec::resize`, no prefix copy) unless cost accounting is on; LLVM still copies (G-LG16-1). An element read
  directly off the call (`resize(a, n, v)[k]`, also through `copy`/`truncate`) keeps the array's storage at i64.
- `band(a, b)`, `bor(a, b)`, `bxor(a, b)`: `(i64, i64) -> i64`, bitwise and/or/xor on the 64-bit two's-complement
  representation. Negative operands are allowed (`band(-1, 255) == 255`); the complement of `x` is `bxor(x, -1)`
  (`== -x - 1`). No overflow obligation on the result. Pure, allowed in code and contracts; the names are reserved
  (no user function of that name). Arguments are evaluated left to right, once. Wrong argument types are
  `type_mismatch`, a wrong argument count is `arity`. The verifier knows `y >= 0 ==> 0 <= band(x, y) <= y`, so
  `a[band(x, 255)]` is in bounds when `len(a) >= 256`.
- `wrapping_shl(a, k)`, `ushr(a, k)`: `(i64, i64) -> i64`; `wrapping_shl` is the low 64 bits of `a << k` (two's
  complement), `ushr` is the logical right shift of the 64-bit pattern (zero fill; `ushr(-1, 1) == 9223372036854775807`).
  In code the verifier must prove `0 <= k < 64` (`verify` error `shift_range`). Pure, allowed in code and contracts,
  names reserved. Known facts (code): `ushr(x, 0) == x`, `wrapping_shl(x, 0) == x`; `x >= 0 ==> 0 <= ushr(x, k) <= x`;
  `k > 0 ==> ushr(x, k) >= 0`; for a literal `c` in 1..63, `ushr(x, c) <= 2^(64-c) - 1`, so `t[ushr(x, 60)]` is in
  bounds for `len(t) >= 16`. Concrete constant expressions are decided exactly.
- `a & b`, `a | b`, `a ^ b` have the same signed 64-bit meaning as `band`, `bor`, `bxor`. `~a` is the
  complement, equal to `-a - 1` in mathematical integers. These operators have no overflow obligation.
  In code, a mask literal `m = 2^k - 1` for `1 <= k <= 62` on either side of `&` gives the verifier
  `a >= 0 ==> (a & m) == a % 2^k`. In contracts both arguments of `& | ^` must be inside i64.
- `a << k` is the exact signed product by `2^k`; `a >> k` is the arithmetic right shift (sign fill),
  equal to division by `2^k` rounded down, e.g. `-7 >> 1 == -4`. In code both shifts require
  `0 <= k < 64` (`shift_range`), then `<<` requires its result inside i64 (`overflow`). In contracts
  both shifts require `a` inside i64 and `0 <= k < 64`; `<<` also requires its result inside i64
  (`contract_domain`). Contract `~a` is the total mathematical operation `-a - 1`.
  All these bitwise operators take operands of type i64 (`type_mismatch` otherwise) and evaluate them left to right.
- `wrapping_add(a, b)`, `wrapping_mul(a, b)`: `(i64, i64) -> i64`, the low 64 bits of the exact sum / product as a
  two's-complement i64 (`wrapping_add(9223372036854775807, 1) == -9223372036854775808`). No overflow obligation on the
  result (ordinary `+ - *` inside the arguments keep theirs). Pure, allowed in code and contracts, names reserved. Known
  fact (code): the result is in i64; concrete constant expressions are decided exactly.
- `load_be(a, i, n)`: `a: [u8]`, `i: i64`, `n` an integer literal in 1..=8; an `i64`, the bytes `a[i..i+n]` read
  big-endian. For `n < 8` the value is in `0..2^(8n)`; for `n = 8` the 64 bits are read as two's complement. Allowed in
  code and contracts. A non-literal `n`, or a literal outside 1..=8, is `type_mismatch`. In code the verifier must prove
  `0 <= i && i + n <= len(a)` (`verify` error `index_oob`).
- `store_be(inout a, i, w, n);`: a whole expression statement (`a` a mutable `[u8]`). It
  writes the low `8n` bits of `w: i64` big-endian into `a[i..i+n]` and leaves the other elements unchanged. `n` and the
  obligation are as for `load_be`. Not allowed in contracts. `a` may also be a `[u8]` field path `inout x.f` (`x.f.g`)
  of a mutable local or `inout` struct parameter, with the destination rules of `copy_range` (LG22): only that field
  changes, the other fields of `x` keep their values, and `i` or `w` mentioning an overlapping path of `x` is
  `type/inout_alias`. No struct invariant is checked at the store (invariants are checked at handoffs).
- `load_le(a, i, n)` and `store_le(inout a, i, w, n);` follow the same rules as `load_be` and `store_be`,
  respectively, using little-endian byte order (byte `a[i+k]` has weight `256^k`). Both names are reserved.
- `eq_range(a, i, b, j, n)`: `a` and `b` of one type, `[i64]` or `[u8]` (else `type_mismatch`); `i, j, n: i64`; a
  `bool`, true iff `a[i+k] == b[j+k]` for every `k` in `0..n` (true for `n == 0`). Code only (`effect_in_contract` in a
  contract), name reserved. The verifier must prove, in this order, `0 <= i`, `0 <= j`, `0 <= n`, `i + n <= len(a)`,
  `j + n <= len(b)` (`verify` error `index_oob`). It then knows the result exactly: `r == forall k in 0..n: a[i+k] ==
  b[j+k]`, over the array values at the call (later writes do not change `r`). Both backends evaluate the five
  arguments once each, in source order. The Rust backend compares the two slices in one call; the LLVM backend
  compares bytes with `memcmp` (a literal `n > 0`, or a dynamic `n >= 4`) or an inline early-exit loop (a dynamic
  `n < 4`); a literal `n == 0` is `true` without a compare. An operand layout other than `[u8]` bytes or 64-bit
  elements is `codegen` error `unsupported_type` (today a `[u8]` operand is stored as bytes and the storage plan keeps
  an `[i64]` operand, refined or not, as 64-bit elements, so it is not reached).
- `copy_range(inout dst, i, src, j, n);` (R-5-1): a whole expression statement (like
  `store_be`; as an expression it is `type/inout_context`). `dst` is a mutable local array or an `inout` array
  parameter (a missing `inout` marker or a non-mutable place is `type/inout_argument`); `dst` and `src` have one type,
  `[i64]` or `[u8]` (else `type_mismatch`); `i, j, n: i64`. Code only (`effect_in_contract` in a contract), name
  reserved. It copies `src[j..j+n]` into `dst[i..i+n]` and leaves every other element and `len(dst)` unchanged; `n == 0`
  copies nothing. The verifier must prove, in this order, `0 <= i`, `0 <= j`, `0 <= n`, `i + n <= len(dst)`,
  `j + n <= len(src)` (`verify` error `index_oob`; only the first one that fails is reported). It then knows
  `len(dst') == len(dst)`, `forall k in 0..n: dst'[i+k] == src[j+k]` (`src` as it is at the call) and
  `forall p: (p < i || p >= i + n) ==> dst'[p] == dst[p]`. These facts are one array-theory update of `dst`
  (`dst' = fun p. if i <= p < i + n then src[j + p - i] else dst[p]`), not a quantified assumption, so they are exact
  (no vacuous or missing fact) and cost the callers no quantifier instantiation; a `[u8]` result stays in `0..=255`.
  Aliasing (LG5): `src` may be exactly `dst`, the same variable or an identical field path
  (`copy_range(inout a, 2, a, 0, 4)`). The copy then has memmove semantics: the source range is read before the
  destination range is written, so overlapping ranges copy the old contents (the Rust backend emits `copy_within`,
  the LLVM backend `llvm.memmove`). The five bounds obligations are unchanged, and `src` is still the array as it is
  at the call. The index arguments `i`, `j` and `n` naming `dst` (including `len(dst)`), and any other `src`
  expression mentioning `dst`, stay `type/inout_alias` (`copy_range(inout a, 0, a, len(a) - 2, 2)`); bind
  `let l: i64 = len(a);` first. Both backends evaluate the arguments once each,
  in source order dst, i, src, j, n. For a `src` that is not `dst`, the Rust backend emits one `copy_from_slice` of
  the two slices (unchecked slices when verified, a checked failure `pruv_oob` with `--no-verify`); the LLVM backend
  emits one `llvm.memcpy` of `n` times the element size (`llvm.memmove` when `src` is `dst`), and an operand layout
  other than `[u8]` bytes or 64-bit elements is `codegen` error `unsupported_type`, as for `eq_range`. Tests:
  `tests/spec/r5_1_copy_range_*.pruv`
  (accept: `_u8`, `_i64`, `_zero`, `_eq`, `_caller`; reject: `_i`, `_j`, `_n`, `_dst_end`, `_src_end`, `_order`,
  `_type`, `_nonarray`, `_noinout`, `_contract`, `_expr`, `_alias`, `_idx_alias`, `_fact_outside`, `_fact_nocopy`),
  `tests/runtime/l3_rt_copy_range*.pruv`, `tests/runtime/lg5_rt_copy_range_overlap.pruv`,
  `tests/must_reject/lg5_copy_range_overlap_oob.pruv`, `tests/must_reject/lg5_copy_range_self_idx_alias.pruv`.
- `copy_within(inout a, dst, src, n);` (R-5-2): an overlap-safe move inside one array (memmove). Same statement,
  place, type, contract and name rules as `copy_range` (`a` is `[i64]` or `[u8]`, `dst, src, n: i64`). It sets
  `a[dst..dst+n]` to the old `a[src..src+n]`, even when the two ranges overlap, and leaves every other element and
  `len(a)` unchanged. The verifier must prove, in this order, `0 <= dst`, `0 <= src`, `0 <= n`, `dst + n <= len(a)`,
  `src + n <= len(a)` (`verify` error `index_oob`, first failure only), and then knows the array-theory update
  `a' = fun p. if dst <= p < dst + n then a[src + p - dst] else a[p]` over the array as it is at the call. An index
  argument naming `a` (also `len(a)`) is `type/inout_alias`; bind `let l: i64 = len(a);` first. The Rust backend
  emits one `::core::ptr::copy` on the array's pointer when verified and `slice::copy_within` after the
  destination-then-source range checks (`pruv_oob`) with `--no-verify`; the LLVM backend emits one
  `llvm.memmove` of `n` times the element size (other layouts `codegen/unsupported_type`). Tests:
  [accept](tests/spec/r5_2_copy_within_accept.pruv), [reject end](tests/spec/r5_2_copy_within_src_end.pruv),
  [reject alias](tests/spec/r5_2_copy_within_idx_alias.pruv), `tests/runtime/l3_rt_copy_within*.pruv`.
- Evaluation order: code evaluates left to right in source order, the same in every compile mode; call arguments left to right, and in `a[i] = v` the index is evaluated before the value.
- Precedence (high to low): unary `- ! ~`; `* / %`; `+ -`; `<< >>`; `&`; `^`; `|`; comparisons;
  `&&`; `||`; `==>`. Binary operators group from the left. In expressions `>>` uses two adjacent `>`
  tokens, so whitespace cannot separate them. Nested type arguments can still close with adjacent `>` tokens.

## 6. Contract-only expressions
Allowed only in `requires`, `ensures`, `invariant`, `assert`:
- `result` (in ensures): the return value.
- `old(a)` (in ensures, invariant, assert): the entry contents of an `inout` parameter `a` (array or struct, also `old(x.f)`, `old(a[i])`, `old(len(a))`) of the enclosing function (section 6g).
- `forall i in lo..hi: e` / `exists i in lo..hi: e` (i ranges over lo <= i < hi).
- `a ==> b` (implication).
- `is_some(o)`, `some_val(o)`, `is_ok(r)`, `ok_val(r)`, `err_val(r)`.
- In contracts arithmetic is mathematical (unbounded); overflow is only checked in code. Contract `/` `%` are
  Euclidean (`-7 / 2 == -4`, `-7 % 2 == 1`, the remainder is never negative) and need only a divisor `!= 0`.
- Contract domain: every partial operation in a contract must be defined where it is evaluated, and the verifier
  proves it (`verify` error `contract_domain`): `a[i]` needs `0 <= i < len(a)`, `x / y` and `x % y` need `y != 0`,
  `some_val(o)` needs `is_some(o)`, `ok_val(r)` needs `is_ok(r)`, `err_val(r)` needs `!is_ok(r)`, `max`/`min` need
  `lo < hi`, and each argument of `band`/`bor`/`bxor` must be inside i64 (contract integers are unbounded and are
  never truncated: `band(9223372036854775807 + 1, 255)` is outside the domain); `wrapping_shl`/`ushr` need both
  arguments inside i64 and `k` in `0..64` (guard with `k >= 0 && k < 64 && ushr(x, k) >= 0`). `wrapping_add`/`wrapping_mul` need both arguments inside i64; `load_be(a, i, n)` and `load_le(a, i, n)` need `0 <= i && i + n <= len(a)`. The builtins have the same 64-bit
  two's-complement meaning in contracts as in code. Evaluation is left to right with short-circuit `&&` `||` `==>`; `forall`/`exists` go up from `lo` and stop
  at the first false/true k, so only the k they reach need a defined body; aggregates need it for every k in range.
  This holds in `requires` (each clause from the earlier ones only: write `requires 0 <= i && i < len(a)` before
  `requires a[i] > 0`), `ensures`, `invariant` (earlier clauses of the same kind may guard later ones), `assert` and
  `decreases`.
- User functions in contracts (L3 P06): a contract (`requires`, `ensures`, `invariant`, `assert`, `decreases`) may
  call a user function whose body is a fixed-size formula, such as an O(1) header check `valid(st)`. The checker
  replaces the call by the function's body, so the contract means exactly what the body computes:
  - The function is not generic, has no `inout` parameter, no `requires`, `decreases` or `cost`, takes only `i64`,
    `bool`, `[i64]`, `[u8]`, `[bool]` and returns `bool` or `i64`.
  - Its body has only immutable `i64`/`bool` lets (no refinement), `if` and `return`; only a `bool` function may
    branch, and every branch ends in `return`. Expressions are literals, names, operators, `a[i]`, and calls of
    `len`, the bit builtins, `load_be`, `load_le`, `utf8_valid` and other functions that follow these rules (no recursion).
  - Lets are substituted into their uses and early returns become `&&`/`||`, so the result is one contract
    expression placed at the call (its diagnostics point at the call). It is at most 4096 nodes.
  - Contract domain and arithmetic apply to the result as to any contract (the guards of the body's own `if`s guard
    its later reads); the function's code is verified separately with no assumptions, so its contract meaning and
    its run-time value agree.
  - Code calls know the body too (L3 P07): at a call of such a function in code, the verifier assumes
    `result == <expansion of the body at the call's arguments>`, as if the function had that `ensures`. So
    `if in_range(a, i) { a[i] }` needs no `ensures` on `in_range`, and a header predicate `valid(st)` checked in code
    bounds the reads after it. The fact uses the same expansion as contracts. An `i64` function whose body calls a
    bit builtin (`band`, `bor`, `bxor`, `wrapping_shl`, `ushr`, `wrapping_add`, `wrapping_mul`) gets no such fact,
    because it would bring bit-vector terms into every later query of the caller; give it an explicit `ensures`
    instead. `bool` functions always get it. A function that cannot be expanded keeps only its written `ensures`.
  - R-6-1 The body fact crosses modules: a `pub fn` with no `requires` in another module gives the same fact at a
    call in the caller's module. A callee with a `requires` (or any other rule above broken) gives none, so a fact
    that only the body proves fails at the call (`verify/assert` for an `assert`). An `opaque fn` (section 6o) gives
    none either. Tests: [accept](tests/spec/r6_1_body_facts/root.pruv),
    [reject](tests/spec/r6_1_body_facts_requires/root.pruv).
  - R-6-2 **Named predicates** (`pred fn`, L3 R03L): `pred fn p(x: T, ...) -> bool { return <expr>; }` names one
    contract expression. Its body is exactly one `return` of a `bool` expression checked as a contract, so it may use
    `forall`, `exists`, aggregates and field reads; a parameter may also be a non-generic struct, read through its
    fields. It has no effects, `requires`, `ensures`, `decreases`, `cost` or type parameters, and every rule above
    applies to its body (otherwise `type/pred_signature`). A pred is only a contract: each use in `requires`,
    `ensures`, `invariant`, `assert` or `decreases` is replaced by its body at the arguments, so the verifier proves
    an `ensures p(result)` from the code and checks a `requires p(d)` at every call, exactly as if the body were
    written there; nothing about a pred is assumed. `pub pred fn` exports it, and another module uses it by path
    (`crate::m::p(x)`) like a function. A call in code is `type/pred_in_code`. A pred has no code, so the
    back ends never see it. A function body with a quantifier or a struct parameter that is called in code gives no
    body fact (L3 P07) either. An argument that uses a name the body binds with `forall`/`exists` is
    `type/call_in_contract`. Tests: [accept](tests/spec/r6_2_pred.pruv),
    [unproved requires](tests/spec/r6_2_pred_unproved.pruv),
    [false ensures](tests/spec/r6_2_pred_false_ensures.pruv), [call in code](tests/spec/r6_2_pred_in_code.pruv),
    [effects and statements](tests/spec/r6_2_pred_effect.pruv),
    [effect call](tests/spec/r6_2_pred_effect_call.pruv), [nested binder](tests/spec/r6_2_pred_nested_binder.pruv),
    [pub pred](tests/spec/r6_2_pred_pub.pruv).
  - R-6-3 **Quantifier triggers** (L3 R03D, G3): `forall i in lo..hi trigger t1, ..., tn: e` (and `exists`) names the
    terms that let the verifier use the quantified fact: the verifier instantiates it only for an `i` where all of
    `t1..tn` appear as terms (one multi-pattern). Each `t` is an array read `a[x]` that mentions `i`. Triggers never
    change the meaning of the formula, only which instances the verifier tries, so they cannot make a false contract
    true. A trigger is a term, not a value: it has no domain obligation. A trigger on `count`/`sum`/`max`/`min`, a
    trigger that is not an array read, or one that does not mention `i` (also after calls in it are expanded) is
    `type/bad_trigger`. Tests:
    [accept](tests/spec/r6_3_trigger.pruv), [no binder](tests/spec/r6_3_trigger_no_binder.pruv),
    [not an array read](tests/spec/r6_3_trigger_not_index.pruv), [on count](tests/spec/r6_3_trigger_count.pruv),
    [false ensures still rejected](tests/spec/r6_3_trigger_still_checked.pruv),
    [binder lost after expansion](tests/spec/r6_3_trigger_expanded.pruv),
    [outer binder only in an inner trigger](tests/spec/r6_3_trigger_nested.pruv).
  - Anything else is `type` error `call_in_contract` "function F cannot be expanded in a contract: REASON". A call in
    a `cost` clause stays unsupported. A `pub` function's `requires`/`ensures`/`decreases` may expand only `pub`
    functions (`type/public_interface`, section 6f), because the body becomes part of the public contract.
- Aggregates (k ranges over lo <= k < hi, like `forall`):
  - `sum(k in lo..hi: e)` — sum of i64 `e`; 0 if the range is empty.
  - `count(k in lo..hi: c)` — number of k where bool `c` holds; 0 if empty.
  - `max(k in lo..hi: e)`, `min(k in lo..hi: e)` — largest / smallest `e`; undefined if empty (`contract_domain`),
    so guard with `len(a) >= 1` or `lo < hi`.
  - There is one form only: write `sum(k in 0..len(a): a[k])`, not `sum(a)`. Ranges are half-open so loop
    invariants can name the part done so far: `invariant s == sum(k in 0..i: a[k])`.
  - The verifier knows one step of each aggregate (e.g. `sum(lo..hi) == sum(lo..hi-1) + e[hi-1]`, and from the
    front `sum(lo..hi) == e[lo] + sum(lo+1..hi)`) at every place the aggregate appears. Loops that grow `i` by one
    and keep `acc == agg(k in 0..i: ...)` therefore verify without extra lemmas. Facts that need induction
    (e.g. `sum` splitting at an arbitrary point, `max` of `e + c`) are not known automatically, except the `count`
    split below.
  - R-6-4 **Count range split** (L3 LG19): the verifier knows `count(k in lo..hi: c) == count(k in lo..m: c) +
    count(k in m..hi: c)` whenever `lo <= m <= hi`, for counts with the same body `c`. It uses the fact for two
    counts that share an end point (`lo..m` and `m..hi`, or `lo..hi` and one of its parts), so a sum of two
    adjacent counts and the remainder of a count after its prefix both verify; it never invents a split point.
    Without `lo <= m <= hi` the fact is not used. Both verifiers try a query first without aggregate lemmas, then
    with them, and add the split only in a third attempt (LG20). Tests: [split](tests/spec/r6_4_count_split.pruv),
    [split point not in the range](tests/spec/r6_4_count_split_unbounded.pruv).
  - Example: `ensures result == count(k in 0..len(a): a[k] < x)` with
    `invariant c == count(k in 0..i: a[k] < x)`.

## 6a. Structs and enums
```
struct Point { x: i64, y: i64 }
enum Event { Empty, Hit(point: Point, accepted: bool), Failed(code: i64) }

let p: Point = Point { x: 3, y: 4 };      // every field, in declaration order
let e: Event = Event::Hit(p, true);       // arguments in field order; `Event::Empty` has no parentheses
let mut q: Point = p;                      // values are copied, never shared
q.x = 8;                                   // field assignment: `let mut` locals only, nested `f.origin.x = e` ok
match e {                                  // statement; every variant exactly once
  Event::Empty => { ... }
  Event::Hit(pt, ok) => { ... }            // one name per field, in order; names are immutable, arm-local
  Event::Failed(code) => { ... }
}
```
- Declared at top level, in any order. Type and variant names start upper-case, field names lower-case.
- Fields: `i64`, `bool`, `Option<i64>`, `Result<i64>`, or another struct/enum. No `()` fields, no `[Point]`, no
  `Option<Point>`. Non-generic structs/enums may also have `[i64]`/`[u8]`/`[bool]`/`str` fields and be recursive (section 6e). Empty structs are allowed; an enum needs at least one variant.
- Read struct fields with `v.field`. Enum payloads are read only with `match` in code.
- `==` / `!=` on structs and enums: contracts only, same type. Enums compare the variant and its fields.
- Contract-only: `is_variant(e, Event::Hit)` (bool) and `variant_field(e, Event::Hit, point)` (that field).
  `variant_field` is defined only when `e` is that variant; the verifier proves this wherever the contract is
  evaluated, following short-circuit order (`verify` error `variant_domain`; other partial operations give `contract_domain`, section 6). Guard it:
  `requires !is_variant(e, Event::Hit) || variant_field(e, Event::Hit, point).x > 0`.
- A loop that assigns any field of `p` forgets all of `p`; keep the other fields with invariants (`invariant p.y == 7`).
- Contract arithmetic inside struct literals is mathematical, like all contract arithmetic.
- Struct invariants (LG16): `struct S { a: i64, b: i64, invariant <bool expr>, invariant <bool expr> }`. The clauses come after the
  fields, name fields bare, may call contract-callable user functions, and may not use `old()` or `result`
  (`type/invariant_decl`; also for a non-bool clause, a generic struct with an invariant, and a parameter named `result`
  of type `S`). They are proof-only: generated code is unchanged and safe mode does not check them. In every function
  the verifier assumes the invariant for each parameter of type exactly `S` at entry and proves it at each handoff:
  every call argument of type `S` (callee precondition), every returned `S` and every `inout` `S` parameter at exit.
  After a call the caller assumes it for the result and for `inout` arguments. Between field writes the value may
  break it. A struct-typed field of `S` carries its own invariant (LG22): the effective invariant of `S` is its own
  clauses, then the effective invariants of its fields of a plain non-generic struct type at `p.f`, in field order
  (a struct already on the path is skipped), and a struct whose effective invariant is non-empty counts as having an
  invariant for every rule here. Nested clauses report the dotted path ("... for parameter p.log of F"). LG23 adds
  guarded positions: the fields of a generic struct instance (type arguments substituted, label `b.v`), the payload
  of an `Option` (under `is_some`, value `some_val`, label `o.some`), the Ok payload of a `Result<T, E>` with `E`
  other than `i64` (`Result<T, i64>` is normalized to the legacy `Result<T>` and is not walked) (under
  `is_ok`, value `ok_val`, label `r.ok`) and the fields of a variant of a non-generic enum (under `is_variant`, value
  `variant_field`, label `s.V.f`); a clause under guards g1..gn is `!g1 || ... || !gn || C`. A type already on the
  path is skipped. LG24 adds the Err payload of the same `Result<T, E>` (under `!is_ok`, value `err_val`, label
  `r.err`) and the fields of a variant of a generic enum instance (type arguments substituted, label `w.V.f`). At a
  call of a generic user function the rules apply to the instantiated signature: an argument whose instantiated
  parameter type has an effective invariant is checked ("... for parameter x of F", also through payloads such as
  `x.some`, partial operations of the clause proved first); a parameter named `result` adds no clause there. A
  result whose instantiated type has one is assumed after the call only when each type parameter of the callee
  occurs in its parameter types only directly, under `Option` or under a two-argument `Result<T, E>` (E not `i64`),
  none of those parameters is named `result`, and each type parameter of the result type occurs in a parameter;
  otherwise the uninstantiated result is assumed (G-LG24-2). The assumption rests on a generic body making a `T`
  value only from its checked `T` arguments; the body itself sees an abstract `T` and assumes nothing about it.
  LG27: a parameter or result of type exactly `Vec<T>` (top level, not nested in a field, payload or `HashMap`),
  where `T` has an effective invariant, gets each clause of `T` as `forall $inv_e in 0..len(p): C[p[$inv_e]]`
  (label `p[]`, `p[].f`): assumed at entry, proved at call arguments, for the result and for an inout `Vec` at exit,
  assumed after the call. A read-only `Vec` parameter cannot be written, so a loop keeps the entry fact; an inout
  `Vec` the body changes (`vec_at_mut`, `vec_clear`, ...) is proved at exit, and a loop that changes it, like a local
  `Vec` built in a loop, needs a user loop invariant stating the element clause (G-LG27-1). Elements of arrays (which cannot hold
  structs), of `Vec` values inside fields or payloads, and `HashMap` values are not assumed (G-LG27-2).
  A failure is `verify/struct_invariant` ("invariant of S (line L) for parameter P of F", "... for result",
  "... for inout P at exit"); a partial operation in a clause proved at a handoff (index, `/`, `some_val`, ...) is
  checked under the same code and message. A host entry may not take a parameter of such a struct
  (`type/entry_signature`). In legacy single-file mode every function is public, so every ordinary function (not a
  `pred`, not a foreign declaration) counts as a host entry for this rule: one with a parameter of such a struct (by
  value, `inout` or `take`) is `type/entry_signature` with the same message (LG17); returning it stays allowed. To
  keep such functions, write the program in explicit mode, where unmarked functions are private.

## 6b. Byte arrays, bool arrays and strings (G2)
Full contract: `docs/G2_SPEC.md`. There are no scalar `u8` types, no casts and no narrow arithmetic: reading `[u8]` or
`str` gives an `i64`, reading `[bool]` gives a `bool`, and all arithmetic stays i64.

**Arrays `[u8]` and `[bool]`.** Same positions and rules as `[i64]` (parameters read-only, `let mut` locals mutable,
return a parameter with `copy`). Refinements: `[u8 | L..=U; <=N]` with `0 <= L <= U <= 255`; `[bool; <=N]` (length only).
- Storing into `[u8]` (`b[i] = v`, and the fill value of `alloc(n, v)`, even when `n == 0`) must prove
  `0 <= v <= 255` (`verify` error `elem_range`; the check order of a store is index, then value range, then refinement).
  `b[i] + 1` is fine as an i64; storing a 256 is not.
- `alloc(n, v)` gets the narrow type **only** when it is directly the right side of a typed `let`
  (`let mut b: [u8] = alloc(n, 0);`), of a reassignment of such a variable, or directly after `return` in a function
  returning `[u8]`/`[bool]`. Anywhere else (a call argument, inside `copy(...)` or `len(...)`) it is `[i64]`; bind it to a
  typed local first. `copy(a)` keeps the type of `a`.
- `[bool]` stores and the fill value of `alloc` for `[bool]` must be `bool` (no 0/1 integers).
- Array types must match exactly. Between `[u8]` and `str`, the only conversion is `str_from_utf8`.
- Elements of `[u8]` parameters, call results and loop-changed arrays are known to be in `0..=255`. Count true elements
  with `count(k in 0..len(b): b[k])`; `sum` needs an i64 body (`sum(k in ..: b[k])` on `[bool]` is a type error).

**Strings `str`.** A `str` is always valid UTF-8 and its bytes never change (`let mut s: str` may only be reassigned).
- Literals `"..."` hold any Unicode text; escapes are only `\" \\ \n \r \t \0` (a real newline or other control
  character inside is an error). In code a literal creates a new string and needs `alloc`; in contracts it is free.
- `len(s)` is the number of **bytes**; `s[i]` is byte `i` as an i64 in `0..=255` (index must be proven in range).
- `copy(s)` (needs `alloc`) returns an independent copy. A `str` follows the array ownership rules: a `let` or
  assignment from another `str` variable, and returning a parameter, need `copy(s)`. Like an owned array, an owned
  `str` local can be handed over with `move t` (`text = move t`, `let x = move t`, `return move t`, a struct field
  `S { f: move t }`); `t` is dead after the move (a later use, also in an `assert` or loop clause, is
  `type/foreign_move`; a move in a loop body that does not restore `t` is `type/foreign_loop_move`), and moving a
  borrowed `str` parameter is `type/array_alias` (LG17). A `take` parameter of type `str` is not allowed.
- `is_char_boundary(s, i)` (code and contracts) is exactly: `i == 0 || i == len(s) || (0 < i && i < len(s) && (s[i] < 128
  || s[i] >= 192))`; it is false outside `0..=len(s)` and never needs a domain proof for `i`.
- `substr(s, lo, hi)` (needs `alloc`, code only) copies bytes `lo..hi`. The verifier proves `0 <= lo <= hi <= len(s)`
  (`string_slice_range`) and then `is_char_boundary(s, lo) && is_char_boundary(s, hi)` (`utf8_boundary`; also when
  `lo == hi`). It knows `len(result) == hi - lo` and `result[k] == s[lo + k]`.
- `utf8_valid(b: [u8]) -> bool` (code and contracts) is true iff the bytes in `b[0..len(b)]` are valid UTF-8.
  The verifier encodes it exactly (R-6b-3, LG26; before LG26 it was an uninterpreted predicate with only the LG9 ASCII
  fact): `utf8_valid(b)` is `W(b, 0, len(b))`, where `W(b, lo, n)` says that every byte `c = b[lo + i]`, `0 <= i < n`,
  is ASCII (`c < 128`), or a lead byte whose continuation bytes follow inside the window (`194..=223` + 1;
  `224` + `160..=191` + 1; `225..=236`, `238`, `239` + 2; `237` + `128..=159` + 1; `240` + `144..=191` + 2;
  `241..=243` + 3; `244` + `128..=143` + 2; a continuation byte is `128..=191`), or a continuation byte (`128..=191`)
  that completes a lead byte one, two or three bytes before it inside the window. This is Rust `str::from_utf8`
  validity (no overlong forms, no surrogates, nothing above U+10FFFF). ASCII bytes are valid, so a scanner that proves
  every byte of a span is below 128 can still pass it to `str_from_utf8` without a runtime check. Other argument types
  give `type_mismatch`. Tests: [ASCII proof](tests/runtime/lg9_rt_utf8_ascii.pruv),
  [bound 129](tests/must_reject/lg9_utf8_ascii_partial.pruv).
- `utf8_valid_span(b: [u8], lo: i64, hi: i64) -> bool` (R-6b-3, contracts only; in code or a `cost` clause `type/contract_only`) is
  `0 <= lo && lo <= hi && hi <= len(b) && W(b, lo, hi - lo)`: the bytes `b[lo..hi]` are valid UTF-8 (false outside
  the range, no domain proof). It is the loop invariant and contract form for a `[u8]` buffer filled by copying the
  bytes of `str` values: appending all bytes of a `str` to a valid prefix `0..k` gives a valid prefix `0..k + len(s)`,
  and a valid span ends at a char boundary, so `subview(buf, lo, hi)` of a proven span (or of a valid prefix cut at
  char boundaries) is passed to a `str` parameter (R-6g-10) with no run-time check. A `str` or other array argument is
  `type_mismatch`. Write the copy invariant over buffer indices, `forall t in k..k + j: buf[t] == s[t - k]`, so the
  solver can match each buffer byte of the goal to a byte of `s`. A `requires` checked at run time (such as under
  `--no-verify`) evaluates it with `str::from_utf8` on `b[lo..hi]` after the range test.
  Tests: [copy loop](tests/runtime/lg26_utf8_span.pruv),
  [partial character](tests/must_reject/lg26_partial_char_copy.pruv),
  [cut inside a character](tests/must_reject/lg26_prefix_boundary.pruv).
- Valid `str` values (R-6b-3). A `str` is valid UTF-8 at run time, and in a function that uses UTF-8 facts the verifier
  assumes `W(s, 0, len(s))` for its `str` values: `str` parameters at entry, reads of `str` variables, `str` results
  of user calls, `str` field reads (under the path condition, and for an enum variant field only when that variant is
  active; inside a quantifier, for all its indices), `str` views passed to `str` parameters, and the results of
  `substr`, `copy`, `str_from_utf8`, `string_to_str` (LG27) and string literals. Each value is assumed once (LG27): a
  later read of the same `str` value adds no new clause; after a loop havoc the new value is assumed again. A function uses UTF-8 facts when its body or its own
  contracts (`requires`, `ensures`, header `decreases` and `cost`, quantifier triggers included; not the contracts
  synthesized from struct invariants) call `utf8_valid`, `utf8_valid_span` or `str_from_utf8`, pass a `subview` of a
  `[u8]` to a `str` parameter, or call a user function whose `requires` or `ensures` mention `utf8_valid` or `utf8_valid_span`. Other
  functions get no such assumption, so their proofs are unchanged.
- `str_from_utf8(b: [u8]) -> str` (R-6b-1, needs `alloc`, code only) copies the bytes into a fresh string. The verifier
  proves `utf8_valid(b)` (`utf8_invalid`), then knows `len(result) == len(b)` and `result[k] == b[k]` for
  `0 <= k < len(b)`. `utf8_valid`, `utf8_valid_span` and `str_from_utf8` are reserved.
- Not available: `==`/`!=` on whole strings (compare `len` and bytes with `forall`), `print` of a string, `str` in
  `Option`/`Result`/struct fields, char/code-point indexing. Character counts are written with boundaries, e.g.
  `count(t in 0..len(s): is_char_boundary(s, t))` counts the characters of `s`.
- The verifier models a `str` as bytes `0..=255`. Outside functions that use UTF-8 facts (above) it does **not**
  assume a `str` is valid UTF-8 (e.g. that byte 0 is not a continuation byte unless you check it), and a
  counterexample string may be invalid UTF-8.

**Run time.** Proof mode removes the element-range, substring and UTF-8 conversion checks. `--no-verify` (safe mode) checks them and panics with exit 101:
`pruv: element out of range: v (u8 0..=255)`, `pruv: string slice out of bounds: lo..hi (len n)`,
`pruv: UTF-8 boundary error: lo..hi (len n)` (range is checked before boundaries), or
`pruv: invalid UTF-8 (len n)`. Raw `--rust-bindings` entries check
`str` UTF-8 and `[bool]` 0/1 bytes before the body (`pruv: invalid representation`) in both modes, also with
`--trust-callers`. Before those checks every raw array or `str` parameter must have a length in `0..=2^40` (bytes for
`str`): a negative length or one above 2^40 stops the program with exit 101 and `pruv: invalid representation: n (array
length)` (`(str length)` for `str`), on both the `pruv_abi_*` and the `__typed` symbols, in every mode (section 7,
"Length bound at host boundaries"). When an `inout` array is present the overlap check runs before this one and
also rejects a negative length of any array it compares (for example `dst_len = 0`, `src_len = -1`), so the overlap
`requires` message can come first; whichever check fails first decides the message, and either way the body never
runs. A null pointer with a positive length is the caller's duty.

## 6c. Generics (G3)
Full contract: `docs/G3_SPEC.md`. Examples: `examples/g3_*.pruv`.
```
struct Pair<A, B> { first: A, second: B }
enum Choice<A, B> { Left(value: A), Right(value: B) }

fn swap<A, B>(p: Pair<A, B>) -> Pair<B, A>
  ensures result.first == p.second
{ return Pair::<B, A> { first: p.second, second: p.first }; }

fn first<T: Element>(a: [T]) -> T
  requires len(a) > 0
  ensures result == a[0]
{ return a[0]; }

let q: Pair<bool, i64> = swap::<i64, bool>(p);
let c: Choice<bool, i64> = Choice::<bool, i64>::Left(true);
let o: Option<bool> = Some::<bool>(true);   // also None::<bool>, Ok::<bool>(x), Err::<bool>(code)
```
- **Type parameters** go on `fn`, `struct` and `enum`: `fn f<T, U: Element>(...)`. The only bound is `Element`; it is
  required wherever `[T]` is written. `Element` means `i64` or `bool`.
- **Type arguments** may be `i64`, `bool`, `Option<V>`, `Result<V>`, and structs/enums of values (also generic
  applications such as `Pair<i64, bool>`). Not allowed: arrays, `str`, `()`, `u8`. A `T: Element` argument must be `i64` or `bool`.
- **Explicit type arguments** are written at every generic call and constructor: `f::<T>(x)`, `Pair::<A, B> { .. }`,
  `Choice::<A, B>::Left(x)`, `Some::<T>(x)`, `None::<T>`, `Ok::<T>(x)`, `Err::<T>(code)`. There is no inference of
  type arguments. Unqualified `Some`, `None`, `Ok`, `Err` keep their old meaning (`Option<i64>`, `Result<i64>`).
- **On an abstract `T` you can** copy it, pass it, store it (locals, fields, payloads, `[T]` elements) and compare it
  with `==`/`!=` **in contracts only**. You cannot do arithmetic, ordering, run-time equality, field access, `match` or `print`
  on a `T`. (`match` on an `Option<T>` or `Choice<A, B>` is fine; the payload variable is a `T`.)
- `Result<T>` keeps an `i64` error code: `Err::<T>(code)`.
- **Typed errors (LG10 E1).** `Result<T, E>` carries an error value of type `E`. `Result<T>` means exactly
  `Result<T, i64>`: the two spellings are the same type with the same code, proof and ABI. `E` may be `i64`,
  `bool`, a value struct or enum (also a generic application that is a value), or an owned `String` (LG11 E2, below).
  Any other `E` (`str`, arrays, collections, a record holding a buffer, `Option`, `Result`, `u8`, `()`, a type
  parameter) is
  `type/result_error_type`. Constructors name `E`: `Ok::<T, E>(v)`, `Err::<T, E>(e)` with `e: E`; `Ok::<T>`,
  `Err::<T>` and unqualified `Ok`/`Err` keep `E = i64` (no inference of `E`). In `match`, `Err(e)` binds `e: E`, and
  the contract selector `err_val(r)` has type `E`. `Result<T, A>` and `Result<T, B>` are different types
  (`type/type_mismatch`). The proof models the error payload with its own sort and well-formedness, and the
  recursive size counts it. Both backends support every value `E` (LG12). In LLVM output a `Result<T, E>` is
  `{ i1, T, E }` and its ABI words are the tag, the words of `T`, then the words of `E` (words of the inactive side
  are 0); for `E = i64` this is the old `Result<T>` layout, and `Result<i64, i64>` keeps its old two ABI words (the
  tag and the active payload).
  ```
  struct ParseErr { pos: i64, code: i64 }
  fn parse_digit(c: i64) -> Result<i64, ParseErr>
    ensures !is_ok(result) ==> err_val(result).code == c
  {
    if c >= 48 && c <= 57 { return Ok::<i64, ParseErr>(c - 48); }
    return Err::<i64, ParseErr>(ParseErr { pos: 0, code: c });
  }
  ```
- **The `?` operator (LG11).** `e?` takes a `Result<T, E>` and yields its Ok value, or returns its error from the
  enclosing function. It may stand anywhere inside the value of a `let`, of an assignment to a local, of a `return`,
  inside an expression statement or inside an `if` condition (LG13), for example `return Ok(f(x)? + 1);`,
  `g(f(x)?);`, `if f(x)? { .. }` or `let y: i64 = a + (f(x)? + f(y)?);`. It is `type/try_position` in any other
  statement (a `while` condition, a field or index assignment, an assignment through a reference, a contract),
  inside a quantifier (also inside one in the right operand of `&&` or `||`), in the right operand of `==>` (a
  contract-only operator, so it is not lowered), in a `return` of a `pred fn` (checked before
  `type/pred_signature`, LG14), and when its operand passes an `inout` argument while the statement reads a local
  outside the operand. When an assignment `x = ..` passes `x` itself `inout` in a statement that has a `?`, it is
  `type/inout_context` (LG14). **Short-circuit operands (LG15).** A `?` only in the right operand `R` of `L && R` or
  `L || R` runs only when `R` runs: the operator is first lowered to `let mut t: bool = D; if C { t = R; }` with a
  fresh local `t` replacing the operator in the statement, where `D, C` is `false, L` for `&&` and `true, !L` for
  `||`; the `?` in `R` is then lowered inside the `if` by the rules here. So
  `if x > 3 && f(x)? { .. }` means exactly the hand-written `let mut t: bool = false; if x > 3 { t = f(x)?; } if t {
  .. }`, with the same proof and the same code on both backends. This is `type/try_position` when the statement, as left
  after its earlier operands are bound and its earlier `?` are lowered, passes any `inout` argument, when the operator
  contains a quantifier or a `subview`, when it moves a name the statement mentions outside it or mentions a name the
  statement moves outside it, or when it mentions a binding whose declared type is not a value type and that the
  statement, counted before any earlier operand is bound to a temporary, mentions again (the operator now runs before
  the rest of the statement, and a borrow and an implicit move of that binding would be split between the `if`
  condition and the `?`); a `?` in the left operand is lowered as usual. **Earlier operands (LG14).** When something other than a local, a literal other than a
  string, a field access, `!` or a comparison or logical operator is evaluated before the `?` in the same statement (a
  call, constructor, move, index, arithmetic, unary `-` or string literal), each such earlier operand on the way from
  the statement to the `?` is first bound to a fresh immutable local, left to right: `return Ok(g(x) + f(x)?);` means
  `let t: i64 = g(x); return Ok(t + f(x)?);`, so it still runs (and its preconditions and index bounds are still
  checked) before the `?`. A short-circuit operand is bound whole, so `(false && b()) == x?` keeps its skipping. This
  is `type/try_position` when a temporary would change the meaning: when the earlier operand contains a quantifier or
  a `subview`; when it moves a name that the statement mentions again (also kept unlowered, as that is a use after
  move); when it or a `?` operand mentions a binding of a type that is not a value type which a call holding it
  mentions again outside it, other than as a direct argument of a borrowed parameter of a user function (nested
  calls share one borrow track, so a borrow taken out of the call would hide its `type/foreign_borrow` with a move,
  explicit or implicit through `Err(x)` or a constructor argument; a borrow in an earlier operand of a different call
  keeps its order before the later move and is bound, LG18); when its type is `()`; when the statement passes an
  `inout` argument and the earlier operand or any `?` operand of the statement mentions that argument's root local
  (or an inout argument is not a local or a field of one), the earlier operand passes `inout` itself, or it or any `?`
  operand calls `print` or a function with an effect other than `alloc` (the inout call's alias and effect checks
  would no longer see it, LG18; a statement with `inout` whose earlier operands and `?` operands do not touch the
  inout roots and have no effect is bound as usual); or when its type is not a value type, except (LG18) a `Vec`,
  `HashMap` or `String` that is a call result (not `subview`) or a plain `move x`, passed directly as an argument of a
  user (non-foreign) function to a `take` parameter of exactly that type: the call then gets `move t`, so the
  temporary owns exactly the value the argument held and is consumed by the same call. A temporary, of a value type
  or such an owned call result, is bound only when every argument of its call (for a non-call operand, every direct
  sub-expression) is plain: a `move`, an integer or bool literal, a place rooted at a local (`x`, `x.f`, `x[i]`), a
  unary, binary or index operator over plain operands, or (LG20) any other expression of a value type whose own
  sub-expressions are plain, such as `len(s)` or `twice(len(s))`; otherwise it is `type/try_position`. A
  non-plain argument, such as `mk(2)` in `lend(mk(2), 0) + need(x)?` or the string literal in `mk(len("a"))`, would
  be built inside the temporary's `let` and dropped there, before the `?` operand runs instead of at the end of the
  statement (LG19, LG20). The rules on a `?` operand (the borrow split, the inout root and the effect rule) hold
  also when nothing is evaluated before the `?`, so `eat(read(s)?, move s)` and `Ok(use3(get(y)?, inout y))` are
  `type/try_position` (LG21). Any other operand of a
  non-value type (a borrow of a live local, a field, an index, a reference, a type holding one, an owned value passed
  to a borrowed parameter, whose temporary would be dropped at the end of the block instead of the statement, one
  passed to a parameter of another type such as `Vec<T>` to `[T]`, an argument of a builtin or a constructor) stays
  `type/try_position`. The operand must be
  a `Result` (`type/try_operand`), the function must return a `Result<U, E'>` (`type/try_return`), and the errors
  must be the same type, `E == E'` (`type/try_error_type`; there is no error conversion). Operands are evaluated left
  to right and the `?` sites of a statement are taken in that order: the statement and the rest of its block mean
  exactly `match e { Ok(v) => { <the statement with e? replaced by v>; <rest> } Err(err) => { return Err::<U, E>(err);
  } }` with fresh names, applied again to the next `?` of the statement inside the Ok arm (a bare `e?;` drops the
  value). So the proof checks the postconditions on the early-return path as for the hand-written `match`, and both
  backends compile the same `match`. The LLVM backend supports `?` for every `E` it lays out (LG12).
  ```
  fn quarter(x: i64) -> Result<i64>
    requires x >= 0
  {
    let h: i64 = half(x)?;
    let q: i64 = half(h)?;
    return Ok(q);
  }
  ```
- **Owned `String` errors (LG11 E2).** `E` may be an owned `String`. A `Result<T, String>` is owned like a `String`:
  `Err::<T, String>(s)` moves `s`, the `Err(e)` arm of a `match` moves the error into `e` (the matched local is used
  up), `?` moves it into the early return, and use after a move is `type/foreign_move`. In contracts `err_val(r)` is a
  `String` (`len(err_val(result))`). The LLVM backend (LG12) lays out `Result<T, String>` as `{ i1, T, ptr }` with
  the String handle null on Ok and after a move, and drops a live handle on every path; it supports every value
  `T` (`i64`, `bool`, a value struct or enum, an `Option` of a value; LG13) for a local, parameter, result or
  temporary. In a runtime-checked `requires` and in a `cost` clause that is kept (`--no-verify`) it reads `is_ok`,
  `ok_val` (for `T` = `i64`, `bool`, LG15, or a struct, enum, `Option` or `Result` whose layout is not extended, read
  into its contract form, LG18) and `err_val` (a `String`, for example `len(err_val(r))`) of a parameter from the
  real storage, with the same result as the Rust backend (LG15; a recursive `T` or one holding an array cannot be a
  `Result` payload, `type/type_argument`); `ok_val` of a legacy-normalized payload (`Option<i64>`, `Result<i64>`) is
  read too (LG23); an operand that is not a parameter is `codegen/unsupported_type`, whose message names the clause
  (`runtime requires` or `kept cost clause`). On both backends a `Result<T, String>` inside a record, enum or
  another generic type is `type/foreign_owned_type` (as the error type of another `Result` it is
  `type/result_error_type`), and one in a host entry signature (no C ABI) is `type/entry_signature`. Records that
  hold a buffer (E3) and `T = String` on the Ok side stay rejected.
  ```
  fn need_pos(x: i64) -> Result<i64, String> ! alloc
    ensures !is_ok(result) ==> len(err_val(result)) == 12
  {
    if x > 0 { return Ok::<i64, String>(x); }
    return Err::<i64, String>(string_from("not positive"));
  }
  ```
- `alloc`, `copy`, indexing and stores work on `[T]` (`T: Element`) as on `[i64]`; the verifier knows that
  `copy(a)` has `len(a)` elements equal to `a`'s. Explicitly typed constructors may appear anywhere an expression may
  (`o = Some::<T>(v);`).
- **Contracts** may use abstract equality, guarded payload access (`is_some(o) ==> result == some_val(o)`) and
  `count` over abstract equality. `sum`/`max`/`min` need an `i64` body.
- Generic recursion only with a stable type-parameter vector: inside a recursive cycle every function has the same
  type parameters (same order and bounds) and every call in the cycle forwards them unchanged (section 6e).
  Recursive or buffer-field types cannot be type arguments.
- **Proof:** a generic definition is proved once, for an abstract `T`. A counterexample prints abstract values as `opaque#N`.
- **Code generation** specializes per concrete instance (`first::<i64>`, `first::<bool>` are separate functions).
  Generic functions are **not exported** (no `--rust-bindings` entry, no raw ABI symbol): expose instances through
  non-generic wrappers (`fn head_int(a: [i64]) -> i64 { return first::<i64>(a); }`, `examples/g3_wrapper_instances.pruv`).
- **Specialization limits:** at most 4096 function instances, 4096 type applications, type nesting depth 128 and 65536
  type nodes; beyond that `codegen/specialization_limit` (no partial output).
- **New error codes:** `generic_syntax` (malformed `<..>`/`::<..>`), `type_parameter` (unknown, duplicate or misused
  parameter), `type_arity` (wrong number of type arguments), `type_argument` (inadmissible type argument),
  `generic_operation` (operation not allowed on an abstract `T`), `generic_bound` (`[T]` without `T: Element`, or a
  non-`Element` argument for an `Element` parameter), `generic_main` (`main` may not be generic), and
  `specialization_limit` (above).

## 6d. Calling Rust and standard collections (G6)
Full contract: `docs/G6_SPEC.md` (§10–§13 are the implemented decisions). Examples: `examples/g6_*.pruv`.
```
trusted rust fn rust_min(a: i64, b: i64) -> i64
  ensures result <= a && result <= b
  ensures result == a || result == b
  = "::std::cmp::min";

fn push_two(take v: Vec<i64>, x: i64) -> Vec<i64> ! alloc
  requires len(v) < 1000
  ensures len(result) == len(v) + 2 && result[len(v)] == x
{
  let mut w: Vec<i64> = vec_push::<i64>(move v, x);
  w = vec_push::<i64>(move w, x);
  return move w;
}
```
- **Trust.** A `trusted rust fn` has no body; it names a Rust function by absolute path (`::std::...` or a crate given
  with `--rust-extern NAME=PATH`). The prover checks that callers satisfy its `requires` and then **assumes** its
  `ensures`; the Rust code itself is not proved. A wrong contract can make wrong code "verify", so the Rust backend lists the contracts in
  `<out>.foreign.json` (the LLVM backend does not write this file yet). Signatures use `i64`, `bool`, `Option`/`Result` of those, read-only `[i64]`/`[u8]`/`[bool]`/`str`
  parameters, and `i64`/`bool`/`()`/`Option`/`Result` results; at least one `ensures` (`ensures true` is allowed).
- R-6d-1 **Owned results.** A trusted function may return `String`, `Vec<i64>` or `Vec<String>` with
  `! alloc`. The result moves into Pruv without copying. Its `ensures` may use `len(result)`; `result[i]`
  is allowed for `String` and `Vec<i64>`. A result longer than 2^40 is a run-time representation failure
  (section 7, length bound). A trusted `Vec<String>` result requires the Rust backend and the same
  length limit applies to each returned String.
  Other result types outside the trusted signature, such as `[i64]`, are `type/foreign_signature`.
  The generated call is wrapped in `catch_unwind`. Tests:
  [accept](tests/spec/r6d_1_owned_results.pruv), [reject](tests/spec/r6d_1_array_result.pruv),
  [accept Vec<String>](tests/spec/r6d_1_vec_string_result.pruv),
  [reject element indexing](tests/spec/r6d_1_vec_string_index.pruv),
  [reject use after move](tests/spec/r6d_1_vec_string_moved.pruv).
- R-6d-2 **Proved vector reads.** In a verified build with `--foreign-checks requires` or `off`, the Rust backend
  emits `vec_get::<T>(v, i)` as `(unsafe { *(<v>).get_unchecked((<i>) as usize) })` after proving its bounds
  precondition, provided the program declares no user `trusted rust fn` (whose unchecked `ensures` a bound proof
  could rest on). A field operand reads the field directly, without a clone. With `--no-verify`,
  `--foreign-checks all` or a user `trusted rust fn`, Rust uses the checked catalog helper. The inlined read is
  still listed in `.foreign.json` and `--trust-report` as a `vec_get` use. This rule does not change LLVM lowering.
  Under the same conditions `vec_push::<T>(vec_new::<T>(), x)` is emitted as `::std::vec![x]` (one element at
  exact capacity, no length check, since an empty vector meets `len(v) < 2^40`); both uses stay listed.
- **Collections.** `Vec<T>`, `HashMap<K, V>` (`K` is `i64`, `bool` or `String` under R-6e-5) and
  `String` hold values. Collections cannot be stored in `Option`, `Result` or arrays, except an owned `String` as the
  error of a `Result<T, String>` (LG11 E2). R-6e-1, R-6e-3 and R-6e-6
  describe collection fields and the permitted nesting. Operations (`vec_*` and `map_*` take explicit type arguments, `string_*` and `len` take none): `vec_new`, `vec_push`,
  `vec_get`, `map_new`, `map_insert`, `map_get` (returns `Option<V>`), `string_new`, `string_push_str`, `string_byte`,
  `string_to_str` (copies into a `str`), `string_from` (copies a `str` into a new `String`), `vec_clear`, `string_eq`, and `len`.
  Allocating operations need `! alloc`. In contracts only: `v[i]`, `s[i]`,
  `map_lookup::<K, V>(m, k)` and `map_updated::<K, V>(after, before, k, v)` (the whole new map: k set, size, others kept).
  A map whose values grow in a loop (counters, histograms) needs either a saturating update (`if c < LIMIT { c + 1 }`)
  (preferred) or an invariant bounding every value (quantifying over all keys is expensive and may time out); an
  invariant about only the queried key cannot bound the others, so `count + 1` is then not provably free of overflow.
- **Ownership.** A collection has one owner. An unmarked collection parameter is borrowed for the call (read only, cannot
  be moved, returned or aliased). `take` parameters and locals own their collection. Pass ownership with `move x`
  (into a `take` argument, a `let`, an assignment, a `return` or a field initializer under section 6e); after
  that `x` is dead until reassigned. A binding
  moved on only some paths cannot be used after the join; a loop must leave every outer owner alive at the end of each
  iteration. Collections are freed automatically. `ensures` may mention a consumed parameter: it means the value at entry.
- **Run-time boundary checks** (`--foreign-checks`): `requires` (default) checks foreign and catalog preconditions at
  run time, subject to R-6d-2; `all` also checks their postconditions (snapshots; slower, repeated appends become quadratic; on the LLVM backend
  the bytes copied by `string_to_str` are not re-compared); `off` skips
  both (only with verification). Failures exit 101 with `pruv: foreign requires violated:`, `pruv: foreign ensures
  violated:`, `pruv: foreign contract undefined:` or `pruv: foreign panic:`.
- **Build.** The Rust backend supports all of the above. The LLVM backend supports collections of `i64`/`bool` and trusted
  functions over `i64`/`bool`/arrays/`str` returning `i64`/`bool`/`()`, built by compiling the generated bindings
  (which then contain the Rust shim) with rustc together with the object; the Rust compiler's `--emit exe|obj` rejects such
  programs with `codegen/foreign_target`, and a `--no-verify` LLVM build cannot check a `requires` that uses `v[i]`, `s[i]`,
  `map_lookup` or `map_updated` (`codegen/unsupported_type`). Collection-bearing functions are internal except
  for the Rust entry results specified in section 6f.
- **New error codes:** `foreign_syntax`, `foreign_path`, `foreign_signature`, `foreign_owned_type`, `foreign_move`,
  `foreign_borrow`, `foreign_loop_move`, `foreign_contract`, `foreign_export`, `foreign_target`.

## 6e. Recursion, recursive types and buffer fields (G9)
Full contract: `docs/G9_SPEC.md` (§10–§14 are the implemented decisions). Examples: `examples/g9_*.pruv`.
```
enum Tree { Empty, Node(value: i64, left: Tree, right: Tree) }
struct Packet { bytes: [u8], label: str }

fn tree_count(t: Tree) -> i64
  requires size(t) <= 1000
  ensures result >= 0 && result <= size(t)
  decreases size(t)
{
  match t {
    Tree::Empty => { return 0; }
    Tree::Node(v, l, r) => { return 1 + tree_count(l) + tree_count(r); }
  }
}
```
- **Recursion and `decreases`.** Functions may call themselves or each other, in any order. Every function in a
  recursive cycle needs exactly one `decreases e` after its `requires`/`ensures`. `e` is a mathematical integer built
  from parameters, constants and contract operations. The verifier proves `e >= 0` at entry and, at every call inside
  the cycle, that the callee's `e` on the actual arguments is `>= 0` and smaller than the caller's entry value
  (`verify` errors `recursive_nonneg`, `recursive_decrease`). Only then is the callee's `ensures` assumed, so it works
  as an induction hypothesis. One scalar measure; for mutual phases use e.g. `2 * n + 1` versus `2 * n`.
- **Recursive types.** Non-generic structs/enums may refer to each other directly through fields, e.g. a list as
  `struct Node { value: i64, next: Link }` with `enum Link { End, More(node: Node) }`. Every type must have a finite
  value (`struct Bad { next: Bad }` is rejected). The compiler adds the boxing; you never write `Box`. Recursion must
  be a direct field, not hidden in `Option`, `Result`, arrays, generic arguments or collections.
- **`size(x)`** (contracts only, non-generic struct/enum values): the number of struct/enum constructors in the value,
  counting only active payloads; scalars, arrays and strings count 0 (use `len` for those). Every direct child is
  smaller than its parent, so `decreases size(t)` works for tree walks. Bound it (`requires size(t) <= 1000`) when
  you add up per-node values.
- **Buffer fields.** `[i64]`, `[u8]`, `[bool]` and `str` may be fields of non-generic structs and payloads. Initialize
  or replace them with a fresh buffer (`alloc`, a literal string) or `copy(view)`; return a field buffer with `copy`.
  `p.a[i] = v` writes one element of an array field in place, including through a nested path `p.s.a[i] = v`.
  The root `p` must be a `let mut` local or an `inout` struct parameter and the field type `[i64]`, `[u8]` or
  `[bool]` (`str` fields are `type/immutable_string`). The obligations and codes are those of `x[i] = v`
  (`verify/index_oob`, `verify/elem_range` for `[u8]`); the length and the other fields are unchanged. With a
  proved index both backends write without a runtime check; `--no-verify` keeps the bounds and byte guards.
- R-6e-1 **Owned collection fields.** Non-generic structs and enum payloads may have `Vec<i64>`, `Vec<bool>`
  and `String` fields. Other collection fields are `type/foreign_owned_type` except as provided in
  R-6e-3, R-6e-5 and R-6e-6. Generic structs and enums cannot have collection fields.
  These aggregates are extended types and follow the buffer-field rules for copying and type positions.
  An owned collection field initializer or replacement (`p.f = e` for a `let mut` local `p`) requires a call
  result or `move x` of an owned collection local or `take` parameter. The move consumes `x`. A plain binding,
  borrowed parameter or field read in the owning position gives the same diagnostic as using that expression
  to initialize a collection local.
  A read `r.f` on a struct parameter, local or match binding is a borrowed view. It may be passed to an
  unmarked collection parameter, including the catalog operations `len`, `vec_get`, `string_byte` and
  `string_to_str`. It is never moved implicitly. Contracts support `len(r.f)` and `r.f[i]` using the same
  buffer model as for collection locals. The verifier models field construction, replacement and reads with
  that model, including `vec_get` preconditions and postconditions and loop invariants on `len(r.f)`.
  Rust uses `::std::vec::Vec<i64>`, `::std::vec::Vec<bool>` and `::std::string::String` for these fields.
  The aggregate derives `Clone` without `Copy`, and struct parameters remain borrowed (`&Rec` for `Rec`).
  Initializers that move a binding or use a call result store it directly without cloning.
  LLVM rejects a struct or enum with such a field as `codegen/unsupported_type`. The diagnostic names the
  aggregate and says owned collection fields need the Rust backend.
- R-6e-2 **Moving a field into its replacement.** This rule permits `move p.f` as a direct `take` argument
  of a call whose result is assigned back to the same field, as in
  `p.f = vec_push::<i64>(move p.f, 5);`. The receiver `p` must be a `let mut` local, a struct `inout` parameter (R-6g-2) or
  the `Some` binder of R-6e-4, and `f` an owned collection field. Apart from the assignment target and the move, `p` may not occur anywhere in the
  statement. R-6e-16 also permits moving a field out and restoring it in the next statement.
  The callee takes the old field value and the assignment stores its result.
  Rust passes it with `::std::mem::take(&mut p.f)` without cloning. An active
  match borrow of `p` still makes the assignment `type/recursive_borrow`.
- R-6e-3 **Collections of owned aggregates.** An owned aggregate is a non-generic, non-recursive struct whose
  fields are scalars, plain value types, R-6e-1 owned fields or `[u8]` buffer fields. A `[u8]` field keeps the
  §6e buffer rules (a fresh buffer or `copy(view)` to build it, reads by index and `len`, no element store) and is
  `Vec<u8>` in Rust, so session bytes stay bytes and are not widened to `Vec<i64>`. Other buffer fields (`[i64]`,
  `[bool]`, `str`) do not make an owned aggregate; they keep their `Vec<i64>`, `Vec<bool>` and `String` spellings.
  A `vec_get::<S>(v, i).f` read of a collection or `[u8]` field borrows it (Rust `&v[i].f`).
  `Vec<S>` and `HashMap<i64, S>` with an owned
  aggregate `S` are allowed wherever a collection type is allowed, including catalog type arguments. A state
  struct is a non-generic struct that may also have fields of these two types. Nesting stops there: an owned
  aggregate may not hold such fields and a state struct is not a payload (`type/foreign_owned_type`).
  A zero-sized struct (no fields, or only zero-sized struct fields) is not a `Vec` element or `HashMap`
  value either (`type/foreign_owned_type`), because the verifier bounds collection lengths by memory.
  `vec_push::<S>` and `map_insert::<i64, S>` move the element in without cloning. The element must be a
  constructor, a call result or `move y`; anything else is `type/foreign_move`. When `S` has owned fields,
  `vec_get::<S>(v, i)` is allowed only as the receiver of a field access (`vec_get::<S>(v, i).f`, Rust
  `v[i as usize].f`, or `get_unchecked` under R-6d-2), and `map_get::<i64, S>(m, k)` only as a direct `match`
  scrutinee whose `Some(r)` binds a borrowed view (Rust `match m.get(&k)`). Other uses of those reads are
  `type/foreign_owned_type`. A plain `S` keeps the copying reads.
- R-6e-4 **In-place map element update.** `map_get_mut::<K, V>(inout P, k)` is allowed only as the direct
  scrutinee of a `match` statement; elsewhere it is `type/foreign_contract`. `P` must be a `let mut` local,
  an `inout` parameter or a field path of either, of type `HashMap<K, V>` (`type/inout_argument`). `k` is
  evaluated once and may not mention the root of `P` (`type/inout_alias`). In the `Some(r)` arm `r` is an
  inout view of the element. The arm may read and assign its fields, use R-6e-2 on `r.f` and pass `inout r`.
  It may not return, move or store `r`. For a record element of a map or vector, `r = e` and `*r = e`
  replace the whole record and emit `*r = e;` in Rust; the right-hand side must be a constructor,
  call result or move of an owned binding (`type/foreign_move`) and may not mention `r` or the root
  of `P` (`type/recursive_borrow`). The root of `P` may not be mentioned in the arm
  (`type/recursive_borrow`), except through a field path disjoint from `P` (R-6e-8). The arm may not contain `return`, `break` or `continue` (`type/foreign_contract`).
  The `None` arm is unrestricted. Rust emits `match P.get_mut(&k)` with one lookup. The verifier gives `r`
  the stored element when `k` is present and writes the final `r` back at arm exit. The `None` arm leaves `P`
  unchanged. When the R-6d-2 condition holds (verified build, foreign checks not `all`, no trusted Rust
  function), R-6e-2's `P.f = vec_push::<T>(move P.f, x)` emits `P.f.push(x);` without `mem::take`.
- R-6e-5 **Text-keyed maps.** `HashMap<String, V>` supports `i64`, `bool` and owned aggregate values
  wherever the corresponding `HashMap<i64, V>` is allowed. `map_get`, `map_get_mut`, `map_remove`, `map_take`
  and contract-only `map_lookup` borrow a key of type `str` or a read of a `String` binding or field.
  Rust passes the `&str` directly or uses `.as_str()` for a `String` read. A borrowed literal allocates
  no buffer. Read-only iteration is specified by R-6e-13. `map_insert` takes an
  owned `String` from a call or `move`; a `str` value there is an argument type mismatch and a borrowed
  `String` read is `type/foreign_move`.
  A Rust state entry under R-6f-2 with the new collection fields may also borrow `String` parameters
  under the same length conditions as `str`.
  `[u8]` keys are `type/foreign_owned_type` or an argument type mismatch. The verifier uses an integer map index `strkey(n, a)`, where `n`
  is the byte length, `a` is the byte array and `strkey` is one uninterpreted function without axioms.
- R-6e-6 **One level of nested collections in aggregate records.** An owned aggregate may also have
  `Vec<String>`, `HashMap<String, bool>`, `HashMap<i64, bool>`, `HashMap<String, i64>` or
  `HashMap<i64, i64>` fields. An aggregate used as a collection element or value may also have a
  `HashMap<String, R>` field, where `R` is an owned aggregate with only scalar, plain-value or
  `String` fields. The i64-valued map fields likewise require the record to be used as a
  collection element or value in the program. Such an aggregate can be an element of
  `Vec<S>` or a value of `HashMap<i64, S>` or `HashMap<String, S>` in the positions of R-6e-3.
  A non-generic *holder record* may have direct fields of such a nested-collection record type
  besides scalar, plain-value, R-6e-1 owned and `[u8]` fields. A holder record is itself an owned
  aggregate with nested collections: it may be an element or value in the positions above, a
  `HashMap<String, H>` value of a root state field, a local, a `take` parameter or a result. It is
  constructed without a copy from a call result or `move x` per field (R-6e-12); a plain binding as a
  constructor field deep-copies it (Rust `Clone::clone`). It may move whole fields in and out
  under R-6e-16, reads `e.b.f` as a borrowed view, assigns and pushes to `e.b.f` in place, including in a
  `map_get_mut` arm, and passes `inout e.b` to a user function (R-6g-3). Rust emits a plain nested struct
  with no `Box` or flattening. A copy of a holder value without `move` deep-copies it, as for every
  aggregate. A `HashMap` whose values are holder records needs `i64` or `String` keys. An element read
  `vec_get::<H>(v, i).b.f` reads the field in place without moving the element. One call cannot both
  borrow a binding and move it inside another argument, including inside a nested constructor or call
  (`type/foreign_borrow`).
  Deeper collection nesting, a holder record as a direct field, and using a nested-collection or holder
  record as an `Option` or `Result` payload are `type/foreign_owned_type`, with the direct
  scrutinee exception for `map_take` under R-6e-14. In a mutable `Some` arm,
  R-6e-2 permits replacing nested fields with `vec_push` or `map_insert`, and R-6e-7 permits
  `map_remove` on a nested map. R-6e-14 permits taking its value with `map_take`.
  Under R-6d-2 the replacements emit `.push(x)` and `.insert(k, v)`.
  `len` reads either collection, and `map_get` and `map_get_mut` access nested map values.
  For a nested `HashMap<String, R>` field `r.sub`, the insertion form is
  `r.sub = map_insert::<String, R>(move r.sub, key, value)`, and the removal form is
  `map_remove::<String, R>(inout r.sub, k)`. In a mutable outer arm,
  `map_get::<String, R>(r.sub, k)` as a match scrutinee binds a read-only view in the
  `Some` arm. `map_get_mut::<String, R>(inout r.sub, k)` binds a mutable view under
  R-6e-8 and emits `match r.sub.get_mut(k)` in Rust. The views end with their arms.
  A mutable scalar binder is read with `*v` or `v` and assigned with `*v = x` or `v = x`.
  The verifier stores each nested collection in its own datatype field and preserves
  sibling fields and other collection elements on write-back. A root `pub` function may borrow a
  record with the new collection fields through the Rust ABI. Like R-6f-2 resident state, its collection
  lengths (at most 2^40, section 7 item 6) are not checked per call: the record must come from this
  program's entries (section 7 caller obligations). Tests:
  [accept String keys](tests/spec/r6e_6_scalar_map_field.pruv),
  [accept i64 keys](tests/spec/r6e_6_i64_map_field.pruv),
  [accept nested record values](tests/spec/r6e_6_record_map_field.pruv),
  [reject String values](tests/spec/r6e_6_string_map_field.pruv),
  [reject deeper nesting](tests/spec/r6e_6_deep_map_field.pruv),
  [accept holder records](tests/spec/r6e_6_holder_record.pruv),
  [reject holder in holder](tests/spec/r6e_6_record_field.pruv),
  [reject holder payload](tests/spec/r6e_6_holder_option.pruv),
  [reject use after move](tests/spec/r6e_6_holder_use_after_move.pruv),
  [accept element field read](tests/spec/r6e_6_holder_vec_get_field.pruv),
  [reject non-text map key](tests/spec/r6e_6_holder_map_key.pruv),
  [reject borrow and move in one call](tests/spec/r6e_6_holder_borrow_and_move.pruv),
  [reject a move inside a nested call](tests/spec/r6e_6_holder_borrow_through_call.pruv).
- R-6e-7 **Remove and positional update.** `map_remove::<K, V>(inout P, k) -> bool` removes the
  entry and returns whether it was present. It accepts `i64` or `String` keys, uses the place and
  alias rules of R-6e-4 and the statement rules of R-6g-1, and requires no `alloc` effect. Rust emits
  `.remove(&k).is_some()` for `i64` and uses the borrowed key of R-6e-5 for `String`. The verifier clears
  the presence bit and subtracts one from the length only when the key was present.
  `vec_at_mut::<S>(inout P, i)` on `Vec<S>` for an owned aggregate uses the direct match and
  arm rules of R-6e-4. Non-entry functions may take `inout` map and vector parameters whose value or
  element type is an owned aggregate struct; R-6e-10 extends this to every supported vector element type.
  Other collection types stay `type/inout_type`. Rust emits `match P.get_mut(<usize as ::std::convert::TryFrom<i64>>::try_from(i).unwrap_or(usize::MAX))`, so no target truncates `i` (the qualified `TryFrom` path compiles in every Rust edition). The verifier selects `Some` when
  `0 <= i < len(P)` and writes the final binder back to that index with unchanged length.
  These operations and the new collection types require the Rust backend.
- R-6e-8 **Disjoint field borrows.** Inside the `Some` arm of `map_get_mut` or `vec_at_mut` on the field
  path `P = x.f1...fn`, the arm may use any other field path `x.g1...gm` of the same root that is
  disjoint from `P`: neither path is a prefix of the other. It may read and assign it, pass it as an
  `inout` place, and start a nested `map_get_mut` or `vec_at_mut` arm on it, nested to any depth over
  pairwise disjoint paths. A use of `x` as a whole, of `P` itself, of a prefix of `P` or of a path
  through `P` stays `type/recursive_borrow`. R-6g-3 allows `f(inout x.f, inout x.g)` for disjoint
  fields. Rust emits nested
  `match x.f.get_mut(..)` arms over sibling fields with no clone (Rust's disjoint field borrows).
  The verifier writes the final `r` back into the arm-exit value of `x` along `P` only, so writes to
  disjoint paths in the arm are kept and `P` is unchanged by them.
  A `match` on an extended field path `Q` of a mutable local, an `inout` parameter or an element binding holds `Q`
  the same way while its arms run: assigning, moving or passing `inout` a path that overlaps `Q` (or
  the whole root) is `type/recursive_borrow`, and disjoint sibling paths stay usable.
- R-6e-9 **Borrowing a String vector element.** `vec_get::<String>(v, i)` borrows a `str` from a
  `Vec<String>` local, parameter or field. It is allowed only as a direct argument to a user
  function's `str` parameter, as the text key of `map_get`, `map_get_mut`, `map_remove` or `map_take`, or as
  an operand of `string_eq` or `len`. Other uses are `type/foreign_owned_type`.
  The verifier proves `0 <= i < len(v)`. Rust borrows the element with checked indexing, or uses
  `get_unchecked` under R-6d-2, followed by `.as_str()`. Tests:
  [accept](tests/spec/r6d_1_vec_string_result.pruv), [reject](tests/spec/r6e_9_element_let.pruv).
- R-6e-10 **Clearing a vector.** `vec_clear::<T>(inout P) -> ()` is a whole statement. The vector place
  may be an owned mutable local, an inout parameter or a mutable field, including a nested vector
  field in an arm. It follows the place and alias rules of R-6e-4 and the statement rules of R-6g-1.
  Rust emits `P.clear();`, which keeps capacity and needs no `alloc`. The verifier sets the length
  to zero and preserves sibling fields and other elements of outer collections. It requires the
  Rust backend. Tests:
  [accept](tests/spec/r6e_10_vec_clear.pruv), [reject](tests/spec/r6e_10_clear_borrowed.pruv).
- R-6e-11 **Text equality.** `string_eq(a, b) -> bool` is a pure operation in executable code.
  Each operand may be a borrowed `String`, a `str`, a `[u8]` or an R-6e-9 element.
  The verifier compares lengths and every byte in the valid range. Rust compares borrowed byte slices.
  A field or element operand borrows its root place for the call; moving that root in the same call is
  `type/foreign_borrow`.
  Tests: [accept](tests/spec/r6e_11_string_eq.pruv), [reject](tests/spec/r6e_11_eq_non_text.pruv),
  [reject](tests/spec/r6e_11_eq_moved_elem.pruv).
- R-6e-12 **Moving an array.** `move x` transfers an owned array local or a `take` array
  parameter into a struct field, enum payload, local initializer, assignment, return, or
  `take` argument. The same holds for an owned local or `take` parameter of a non-generic struct
  type (an R-6e-1 aggregate or an R-6e-6 record) moved into a struct field or enum payload, as in
  `return St { s: move s };`; Rust stores it directly without cloning. On LLVM (LG9, G-LG8-5) a `take`
  parameter of an R-6e-6 record (`{ptr, i64, i64}` buffer fields) is a pointer to storage the callee owns: the
  caller passes a fresh spilled copy of the moved value, zeroes the moved source slot, and the callee drops the
  record at exit unless it moved it out (a borrowed record parameter stays a read-only view of the caller's slot).
  Tests: [take record](tests/runtime/lg9_rt_take_record.pruv), [record move](tests/runtime/lg9_rt_record_move.pruv). A struct with an array field
  other than `[u8]` (for example `S { xs: [i64] }`) is not an owned aggregate, so `move` of it is
  `type/foreign_move`; a struct of scalar and value fields only is a value aggregate and may be moved.
  Tests: [accept](tests/spec/r6e_12_take_aggregate.pruv),
  [reject](tests/spec/r6e_12_take_aggregate_after_move.pruv),
  [reject plain record](tests/spec/r6e_12_move_plain_record.pruv). Indexed elements cannot be moved; array fields move only under R-6e-16. Moving a plain or
  `inout` array parameter is `type/array_alias`; declare it `take` or make a copy.
  A `take x: [T]` parameter requires an explicit `move` argument.
  The source is dead until reassigned. A read after a definite or possible move is
  `type/foreign_move`, and a loop must restore a moved binding on every continuing path
  (`type/foreign_loop_move`). Returning an owned local array as `return x` remains legal.
  Rust uses `Vec<T>` for a `take` array parameter and emits the moved value directly without copying.
  The verifier charges zero units for the move itself. LLVM moves local arrays by emptying
  the source slot. Both backends support internal `take` array parameters. LLVM rejects them on host
  entries with `codegen/unsupported_type`.
  Tests: [field and payload](tests/spec/r6e_12_field_move.pruv),
  [take parameter](tests/spec/r6e_12_take_param.pruv), [LLVM](tests/spec/r6e_12_llvm_move.pruv),
  [use after move](tests/spec/r6e_12_use_after_move.pruv), [borrowed parameter](tests/spec/r6e_12_borrowed_param.pruv),
  [inout parameter](tests/spec/r6e_12_inout_param.pruv), [bare field initializer](tests/spec/r6e_12_bare_var.pruv),
  [move in dead code](tests/spec/r6e_12_dead_move.pruv), [take parameter in a contract](tests/spec/r6e_12_take_contract_index.pruv),
  [move in a loop condition](tests/spec/r6e_12_loop_cond_move.pruv).
- R-6e-12a **Moving an array out of a match binder (G-DB16-1).** `move b` of an array-typed binder `b` of a
  user-enum arm is allowed when the scrutinee is a plain name (not a field path or call) of a non-generic enum
  that is an owned local (not a `take`, plain or `inout` parameter, and not another arm's binder), and no arm of that
  `match` names the scrutinee. A `match` where some arm moves a binder consumes its scrutinee on entry to every arm:
  the scrutinee is moved inside each arm and after the `match`, and inside an arm its array binders are owned
  bindings (a use after their own move is `type/foreign_move`). A later read of the scrutinee is `type/foreign_move` ("use after move" / "binding may have been moved"), and in a loop the scrutinee must be
  live again (`type/foreign_loop_move`). Every other `move` of a binder stays `type/array_alias`. Rust matches the
  scrutinee by value, also under cost metering, so `b` is an owned `Vec<T>` moved as is. LLVM takes the binder's
  `{ptr, len, cap}` and stores the empty array into that field of the scrutinee's slot, so its drop frees nothing twice.
  Tests: [runtime](tests/runtime/lg8_rt_match_move.pruv),
  [use after](tests/must_reject/lg8_match_move_use_after.pruv),
  [borrowed scrutinee](tests/must_reject/lg8_match_move_borrowed.pruv).
  LG9 (G-DB15-2): under the same conditions, `return x` of a `str` binder `x` of that arm is allowed without
  `copy(x)` and means exactly `return copy(x)`: it needs `alloc`, and the verifier and both backends treat it as that
  copy (the checker rewrites the value into the `copy(x)` call, so both backends and the cost bound count the
  copy). Otherwise returning a binder stays `type/array_alias`.
  Tests: [runtime](tests/runtime/lg9_rt_match_str_return.pruv),
  [parameter scrutinee](tests/must_reject/lg9_match_str_return_param.pruv),
  [named scrutinee](tests/must_reject/lg9_match_str_return_named.pruv).
- R-6e-13 **Read-only text-keyed map iteration.** The statement
  `for (k, v) in P invariant <bool>* { body }` iterates over a binding or field path `P` of type
  `HashMap<String, V>`, with `V` equal to `i64`, `bool` or an owned aggregate (including R-6e-6
  records). Here `for` is contextual at the start of a statement and must be followed by `(`.
  A non-place expression is `type/foreign_contract` and an unsupported place type is
  `type/foreign_owned_type`. The loop may have zero or more invariants and has no `decreases` clause.
  Invariants are checked in the enclosing scope. In the body, fresh names `k` and `v` bind a borrowed
  `String` view and the current value, respectively. Scalar values are copied and aggregate values
  are borrowed as in a `map_get` arm. The key can be read as an R-6e-5 key or passed to `string_eq`,
  `len` and `str` parameters. The binders cannot be moved, stored, returned or passed `inout`.
  Assigning a binder is `type/immutable`.
  The body holds `P` read-only. Assigning, moving or mutably borrowing an overlapping path,
  including the whole root, is `type/recursive_borrow`. Disjoint sibling paths remain usable
  under R-6e-8. A `return` inside the body or a map loop inside a function with a `cost` clause is
  `type/foreign_contract`.
  Iteration order is unspecified. The verifier assumes no order, no distinctness and no iteration
  count. Programs must not depend on them. It proves invariants on entry, forgets the locals and
  places assigned by the body as for `while` while preserving `P`, and assumes the invariants.
  It checks the body and invariant preservation with an arbitrary well-formed key whose
  `strkey(n, a)` is present in `P` and with `v` equal to that entry's value. After the loop it
  again forgets the same assignments, preserves `P`, and assumes the invariants. Each iteration
  adds one unit as for `while`.
  Rust emits `for (k, &v) in P.iter()` for scalar values and `for (k, v) in P.iter()` for aggregates.
  The key is a `&String` and Rust uses `k.as_str()` where a `&str` is required.
  LLVM rejects map iteration with `codegen/unsupported_type`.
- R-6e-14 **Taking a map entry.** `map_take::<K, V>(inout P, k) -> Option<V>` removes the
  entry at `k` and supplies its former value in `Some(v)`, or supplies `None` if the key was absent.
  `K` is `i64` or `String`, and `V` is `i64`, `bool` or an owned aggregate struct, including
  R-6e-6 records with nested collections. Unsupported types are `type/foreign_owned_type`.
  The mutable place `P` follows R-6e-4 and R-6e-7, including a nested map field of a mutable
  collection binder. The key is evaluated once and borrows text under R-6e-5 and R-6e-9.
  Mentioning the root of `P` in the key is `type/inout_alias`. Taking from a place that
  overlaps an active collection borrow or map iteration is `type/recursive_borrow`.
  The call is allowed only as the direct scrutinee of a `match` statement (`type/foreign_contract`
  elsewhere), so its result cannot be discarded. The call needs no `alloc` effect.
  `Some(v)` binds an owned mutable local. It can be assigned or passed `inout`; an aggregate
  binder can be moved, and a scalar binder is copied (an explicit `move` of it is `type/foreign_move`).
  A read view of a different map field does not overlap `P`; a read view of `P` itself does.
  Both arms can use the map again and can return, break or continue where the surrounding
  context allows. The verifier binds the old value, clears the key's presence, and decreases the
  map length by one exactly when the key was present. Changes to the binder stay local to the arm.
  Rust emits a single `match P.remove(key)` with `Some(mut v)`, borrowing the key as for
  `map_remove`. LLVM rejects the call with `codegen/unsupported_type`.
  Tests cover [record](tests/spec/r6e_14_take_record.pruv), [scalars](tests/spec/r6e_14_take_scalar.pruv),
  [mutable binder](tests/spec/r6e_14_take_mut_binder.pruv), [nested map](tests/spec/r6e_14_take_nested.pruv),
  [mutable view](tests/spec/r6e_14_take_in_get_mut.pruv), [read view](tests/spec/r6e_14_take_in_get_arm.pruv),
  [iteration](tests/spec/r6e_14_take_in_iter.pruv), [let binding](tests/spec/r6e_14_take_let.pruv),
  [key type](tests/spec/r6e_14_take_key.pruv), [LLVM](tests/spec/r6e_14_take_llvm.pruv),
  [other field](tests/spec/r6e_14_take_other_field.pruv), [scalar read view](tests/spec/r6e_14_take_in_scalar_get.pruv)
  and [binder name reuse](tests/spec/r6e_14_take_proc_binder.pruv).
- R-6e-15 **Owned text from a borrow.** `string_from(t) -> String` needs `! alloc` and takes
  the same `str` argument as the text parameter of `string_push_str`. The verifier checks
  `len(t) <= 2^40` and establishes that the fresh result has length `len(t)` and matches every byte
  of `t`. Rust uses `t.to_string()` for the sole copy. LLVM calls the same helper through the
  collection bridge and returns a fresh owned handle.
  Tests cover [copying text](tests/spec/r6e_15_string_from.pruv),
  [missing effect](tests/spec/r6e_15_from_no_alloc.pruv), [non-text argument](tests/spec/r6e_15_from_non_text.pruv),
  [false postcondition](tests/spec/r6e_15_from_wrong_ensures.pruv), and [LLVM lowering](tests/spec/r6e_15_from_llvm.pruv).
- R-6e-16 **Moving a field out.** For a receiver `p` mutable under R-6e-2, `p.g = move p.f;`
  moves an owned `String`, `Vec<i64>` or `Vec<bool>` field, or a buffer field `[i64]`, `[u8]` or `[bool]`,
  to a disjoint field path of the same
  root and type. `let t: T = move p.f;` moves the field into an ordinary owned local of its type.
  Both forms support nested struct field paths. The immediately next statement in the same block
  must assign to exactly the moved path `p.f`. Its right-hand side may be an owning expression
  allowed for field replacement and may not mention `p`. It may also be a move from a disjoint
  same-typed field of `p`, in which case the following statement must restore that field in turn.
  For example, `let t: String = move p.f; p.f = move p.g; p.g = move t;` swaps two fields.
  Consuming form (G-R09-4, extended by LG9 G-LG8-1/G-LG8-3): `let t: T = move d.f;` of any field type this rule
  moves (`String`, `Vec<i64>`, `Vec<bool>`, `[i64]`, `[u8]`, `[bool]`), at any nested struct field path, may stand
  without a restore when `d` is a `let mut` local record or a `take` record parameter (which the callee owns). The
  same holds for `f(move d.f)` as a `let` initializer, a call statement or a `return` value when the field move is
  the direct argument of a `take` parameter of `f`, it is the only field move of the call, and no other argument
  names `d` (otherwise the field move keeps its non-consuming errors, e.g. `type/array_alias` for
  `let n = f(move d.a, d.n);`, while `return f(move d.a, d.n);` gives `type/foreign_move` from the return check). The consuming form on a match binder (also a `map_take` binder) and on a plain or `inout` root stays
  `type/foreign_move`. It moves `d` wholly: a later use of `d` is
  `type/foreign_move` "use after move" ("binding may have been moved" on some
  paths, `type/foreign_loop_move` for a loop that does not restore it); assigning `d` again revives it. A restore in the
  next statement keeps the form above. `p.g = move p.f;` and moves from `inout` and plain roots keep
  the errors below. Rust emits `::std::mem::take(&mut d.f)` (the root is declared `mut`, also for a `take`
  parameter). LLVM lowers buffer fields at any path as above; `String`/`Vec` fields need the Rust backend.
  Tests: [take field](tests/runtime/lg9_rt_take_field_move.pruv), [field kinds](tests/runtime/lg9_rt_field_move_kinds.pruv),
  [take argument](tests/runtime/lg9_rt_field_move_arg.pruv), [use after](tests/must_reject/lg9_move_field_arg_use_after.pruv),
  [other argument](tests/must_reject/lg9_move_field_arg_other_mention.pruv),
  [borrowed root](tests/must_reject/lg9_move_field_arg_borrowed.pruv).
  Outside R-6e-2 and these forms, field moves are `type/foreign_move`. A missing or invalid
  restore, or a move between overlapping paths, has the same code and the message
  "a field moved out must be reassigned by the next statement". Moving a path held by an active
  match, collection borrow or map iteration is `type/recursive_borrow` under the overlap rules
  of R-6e-8 and R-6e-13.
  Rust emits `::std::mem::take(&mut p.f)` without cloning. The verifier reads the old field into
  the destination at zero cost and retains its symbolic source value until the restore.
  Owned collection fields require the Rust backend. Buffer fields also lower to LLVM: the move copies the
  field's `{ptr, len, cap}` into the destination and stores the empty array into the field without freeing,
  and the restore is an ordinary field replacement. Any other array field move is `type/array_alias`.
  Tests cover [field transfer](tests/spec/r6e_16_move_field.pruv),
  [swapping](tests/spec/r6e_16_swap.pruv), [mutable collection views](tests/spec/r6e_16_in_get_mut.pruv),
  [missing restoration](tests/spec/r6e_16_not_restored.pruv) and [held paths](tests/spec/r6e_16_held_borrow.pruv).
- R-6e-17 **Borrowing a vector as an array argument.** A read of a `Vec<i64>` or `Vec<bool>` place
  may be passed to an unmarked parameter of type `[i64]` or `[bool]`, respectively, of a Pruv
  function. The place may be a local, parameter, binder or field path. This conversion is unavailable
  for `take` or `inout` parameters, trusted Rust calls and positions other than call arguments.
  Rust passes `place.as_slice()` without allocating or copying. The verifier uses the vector's existing
  length and contents. A call that borrows a field of a vector-bearing binding cannot also move
  that binding (`type/foreign_move`). This conversion requires the Rust backend even in contracts (`codegen/unsupported_type`).
- R-6e-18 **Appending a slice to a vector.** `vec_extend_from_slice::<T>(inout P, s: [T]) -> ()` for `T`
  `i64` or `bool` is a whole statement and needs `! alloc`. The place `P` follows R-6e-10; other
  element types are `type/foreign_owned_type`. Rust emits `P.extend_from_slice(&(s)[..]);` with no
  per-element loop. Unless the catalog runs unchecked (verified, foreign checks not `all`, no foreign
  functions), Rust first checks `len(P) + len(s) <= 2^40` and otherwise fails as a foreign `requires`
  violation. The verifier proves the new length is at most 2^40 and sets it to
  `len(old P) + len(s)`, keeps the old prefix and makes the suffix equal to `s`, preserving sibling
  fields. LLVM rejects it as `codegen/unsupported_type`. Tests:
  [accept](tests/spec/r6e_18_vec_extend.pruv), [reject element type](tests/spec/r6e_18_extend_string.pruv),
  [reject borrowed vector](tests/spec/r6e_18_extend_borrowed.pruv),
  [run-time length check](tests/spec/r6e_18_extend_guard.pruv).
- R-6e-19 **Reserving vector capacity.** `vec_reserve::<T>(inout P, n) -> ()` is a whole statement that
  needs `! alloc`; the place `P` and `T` follow R-6e-10. The verifier requires `0 <= n <= 2^40`
  (`verify/call_requires`) and keeps `len(P)`, every element and sibling fields unchanged; capacity is not
  modelled, so the proof knows only the length invariant. Rust emits `P.reserve((n) as usize);` (with the same
  run-time `requires` check as R-6e-18 when the catalog runs checked), so a writer can then `vec_push` or
  `vec_extend_from_slice` without filling the tail first. Both backends support reserving a local
  `Vec<i64>` or `Vec<bool>`. Tests:
  [accept](tests/spec/r6e_19_vec_reserve.pruv), [reject](tests/spec/r6e_19_reserve_unbounded.pruv).
- **Values, not pointers.** Recursive and buffer-bearing values are independent: copying one deep-copies everything,
  and changing a copy never changes the original. Parameters, field reads and `match` bindings are read-only views
  and do not allocate (the one exception: R-6e-12a moves an array binder out of a consumed scrutinee). Putting an existing value into an owning place (`let`, assignment, constructor field,
  `return`) copies it and needs `! alloc`; a fresh constructor or call result moves without a copy. Any constructor
  of a recursive value needs `! alloc`. While a `match` arm views part of a mutable local, that local cannot be
  changed (`type/recursive_borrow`). Whole-value `==`/`!=` on values that contain arrays, strings or owned collections is rejected
  (`type/eq_type`); compare tags, lengths and elements instead.
- **Limits.** Recursive and buffer-field types cannot be generic arguments, `Option`/`Result` payloads or G6
  collection elements, except the owned aggregates of R-6e-3. Functions whose signatures use these types are internal, except
  the Rust entries in R-6e-5, R-6e-6 and R-6f-2. In explicit mode other host entry signatures
  with such types are `type/entry_signature`. A property such as "x is somewhere in the tree"
  must be stated with the existing contract operations, and not every recursive property is
  expressible. Proofs about deeply nested or very wide types can be slow. Native stack depth is not checked: very
  deep recursion, copying or freeing can overflow the stack (a resource failure, never a successful result).
  `--no-verify` does not check `decreases` and may recurse forever. Run-time `requires` that use `size`, `==` or
  field reads of recursive values count one unit of the 2^30 contract budget per node visited.
- **New error codes:** `decreases_syntax`, `recursive_decreases` (missing measure), `recursive_polymorphism`,
  `recursive_generic_type`, `recursive_borrow`, `size_type`, `recursive_nonneg`, `recursive_decrease`; reused:
  `recursive_type` (type without a finite value), `field_type`, `type_argument`, `eq_type`, `missing_effect`.

## 6f. Modules and visibility (G4)
Full contract: `docs/G4_SPEC.md` (§10–§13 are the implemented decisions). Example: `examples/g4_backend/`.
```
// root.pruv                               // arithmetic.pruv
mod arithmetic;                            pub fn increment(x: i64) -> i64
pub fn bounded(x: i64) -> i64                requires 0 <= x && x <= 10
  requires 0 <= x && x <= 10                 ensures result == x + 1
  ensures result == x + 1                  { return x + 1; }
{ return crate::arithmetic::increment(x); }
```
- **Files.** The command-line file is the root. `mod name;` (top level only) loads `name.pruv` next to the root;
  inside `a.pruv`, `mod b;` loads `a/b.pruv`. Only declared files are loaded, each exactly once. No inline modules,
  `pub mod`, path attributes or `mod.pruv`.
- **Explicit mode.** A root that contains `mod` or a top-level `pub` switches the whole program to explicit
  visibility. Then unmarked `fn`/`struct`/`enum` items are private to their file. A root with neither keeps the old
  single-file rules and output unchanged.
- **Paths.** Other modules' items are reached only by full paths from `crate`: `crate::math::clamp(x)`,
  `crate::model::Event::Hit`, `crate::model::Pair<i64, bool>`, `crate::math::first::<i64>(a)`. Plain names look only
  in the same file, then builtins. There is no `use`, `self`, `super` or relative path.
- **Visibility.** `pub` makes an item usable from every module. Fields and variants of a visible type are always
  visible. A public signature or contract may mention only public types and items (checked through field types too);
  a function a public contract calls directly must be `pub` (section 6, user functions in contracts);
  private helpers are fine inside bodies. `pub trusted rust fn` is rejected: wrap it in a public Pruv function.
- **Function attributes.** Write at most one of `#[cold]`, `#[inline]` or `#[inline(always)]` directly before a
  non-`pub` top-level `fn` item (generic fns allowed, any module). The attribute affects generated Rust (R-7-3);
  checking and verification are unchanged. Other attributes and arguments are `parse/unknown_attribute`.
  A recognized attribute before `pub fn`, `opaque fn`, `trusted rust fn`, a type, `mod`, another attribute or the
  end of the file, and any `#[` inside a body, is `parse/attribute_position`.
- **Host entries.** In explicit mode only root `pub` non-generic functions are host entries. A parameter of
  collection, recursive or buffer-field type, or an `inout` parameter (section 6g, Host limits), is
  `type/entry_signature`, subject to R-6e-5, R-6e-6, R-6f-2, R-6f-4 and R-6f-6. The same restriction applies to results, subject to R-6f-1 and R-6f-3. Ordinary entries can be
  exported through `--rust-bindings` and the LLVM
  `pruv_abi_<name>` interface. Child `pub` functions are for Pruv callers only. An optional root `main`
  must be `pub fn main() -> ()`. A child `main` is an ordinary function.
- R-6f-1 **Vector entry results.** In explicit mode a root `pub` non-generic function may return `Vec<i64>` or
  `Vec<bool>`. The generated Rust module exposes it like any other host entry (`pub fn`, or `pub(crate) fn` under
  manifest `raw_entries`, section 7 Raw entries) returning `::std::vec::Vec<i64>` or `::std::vec::Vec<bool>`.
  A vector-result entry has no C ABI. The LLVM backend and
  `--rust-bindings` reject it with `codegen/unsupported_type` naming the entry. Other collection and
  extended-type results remain `type/entry_signature`. Legacy mode is unchanged.
- R-6f-2 **Typed resident state entries.** In explicit mode the first parameter of a root `pub` entry may be
  `inout st: S` for a non-generic struct `S`, including a state struct (R-6e-3). Only the Rust backend emits it,
  as `pub fn e(st: &mut S, ...)`, or `pub(crate) fn` under `raw_entries` "crate" or "unsafe" where the host keeps
  a `PruvStore<S>`. `raw_entries` "c", `--rust-bindings` and the LLVM backend reject it with
  `codegen/unsupported_type` naming the entry. `requires` on `st` are checked at entry and skipped under
  `--trust-callers`, like other entry `requires`.
- R-6f-3 **Struct entry results.** In explicit mode a root `pub` non-generic function may return a
  non-generic struct `R` that has at least one collection field, whose fields are plain values (no
  `[T]` or `str` buffer anywhere),
  `Vec<i64>`, `Vec<bool>`, `Vec<String>` or `Vec<E>` for an owned aggregate `E` (R-6e-3) without R-6e-6 nested
  collection fields or `[u8]` buffer fields. The Rust entry returns `R` by value (`pub fn e(..) -> R`, fields `pub`), so the host
  owns the moved-out result without a copy. A `Vec<String>` field is exposed as
  `::std::vec::Vec<::std::string::String>`. Its length and the length of each string must be at most
  2^40, as in R-6d-1. The entry trusts those length bounds for the returned field.
  A `HashMap` field, a nested-collection element and deeper nesting remain
  `type/entry_signature`. Like R-6f-1 it has no C ABI: the LLVM backend rejects `R`
  with `codegen/unsupported_type`. Legacy mode is unchanged.
  The entry may also return a non-generic enum `E` whose payload fields are plain values, `Vec<i64>`,
  `Vec<bool>` or such a struct `R` without a `Vec<String>` field, with at least one of the owned kinds
  (`Vec<String>` payloads stay `type/foreign_owned_type` at the enum declaration). The Rust entry returns `E` by value
  (`pub fn e(..) -> E`, a `pub enum`), so a variant's collections move to the host without a copy. Other
  payloads stay `type/entry_signature` and the LLVM backend rejects `E` with `codegen/unsupported_type`.
- R-6f-4 **Owned String entry parameters.** A Rust host entry, with or without a state
  parameter, may take an owned `String` using `take key: String`; the wrapper passes ownership
  to the body without a copy, and LLVM and the C ABI reject the signature with
  `codegen/unsupported_type`.
- R-6f-6 **Owned aggregate entry parameters.** In explicit mode a Rust host entry may take a parameter
  `x: S` or `take x: S` of an owned aggregate `S` (R-6e-3), including one with `Vec<i64>`, `Vec<bool>`,
  `String` or `[u8]` fields. A borrowed `x: S` is `x: &S` in Rust and a `take x: S` is `x: S`, moved into the
  body without a copy; a `[u8]` field is `Vec<u8>`. As for R-6f-2 state, field lengths (at most 2^40, section 7
  item 6) are trusted, not checked per call, for both the borrowed and the `take` form; the host caller must keep them. `requires` on `x` are checked at entry unless `--trust-callers`.
  A `take` parameter follows R-6e-12 in the body (`type/foreign_move` after a move). A struct that is not an
  owned aggregate (an `[i64]`, `[bool]` or `str` field, a generic struct) is not covered by this rule; it keeps
  the plain-value and R-6e-6 entry rules and is otherwise `type/entry_signature`.
  `raw_entries` "c" and the LLVM backend reject it with `codegen/unsupported_type`.
- **Proof.** Every function is verified, including private and unused ones; a caller sees the callee's contract and,
  for a function that can be expanded (section 6, L3 P07, R-6-1), the fact `result == <body>` at its call, from any
  module. It sees no other part of the body. Recursion and recursive types may span modules. Host entries check `requires` at run
  time unless `--trust-callers`; `--no-verify` checks `requires` on every call.
- **Diagnostics** in explicit mode carry a `file` field (relative to the root, `/` separators). New codes include
  `parse/module_syntax`, `parse/visibility_syntax`, `parse/module_file`, `type/private_item`, `type/public_interface`,
  `type/entry_signature`, `type/entry_main` (see `docs/G4_SPEC.md` §8 for the full table).
- **Library directories (G4b-0).** Full rule: `docs/G4B_LIB_SPEC.md`. `--lib DIR` (repeatable, order kept) only changes
  where the file of a `mod NAME;` written in the root is looked up (`NAME` other than `std`):
  `ROOT_DIR/NAME.pruv` plus `DIR/NAME.pruv` for each `--lib`. Exactly one hit is used; none is `parse/module_file`
  (its hint adds `pass --lib DIR`); more than one is `parse/lib_module_conflict`. Children of a library module resolve
  inside that library (`DIR/NAME/x.pruv`); libraries are never consulted below the root, and `std` is never taken from
  one. Library files show as `@lib/NAME.pruv` / `@lib/NAME/x.pruv` in diagnostics (never the real `DIR`). A library
  module is mounted at `crate::NAME` like a local one and names its siblings by `crate::OTHER`, so the root must
  declare them too. The alias check applies; a legacy root ignores `--lib`. `DIR` must be an existing directory.

## 6g. In-place array parameters: `inout` (G5)
Full contract: `docs/G5_SPEC.md` (§2–§8 and §15 for deferred limits). R-6g-1 below replaces its §2.1
statement-boundary rule for calls; the rest of that contract still applies. Examples: `examples/g5_*.pruv`, `tests/runtime/g5_r*`.
LG5 tests: `tests/runtime/lg5_rt_inout_reassign.pruv`, `tests/must_reject/lg5_inout_reassign_len.pruv`,
`tests/must_reject/lg5_inout_reassign_entry/` (and the tests named in R-6g-2 and R-6g-4).
```
fn set_at(inout a: [i64], i: i64, v: i64) -> ()
  requires 0 <= i && i < len(a)
  ensures a[i] == v
  ensures forall k in 0..len(a): k != i ==> a[k] == old(a)[k]
{ a[i] = v; }

let mut w: [i64] = copy(a);
set_at(inout w, 0, 7);
```
- **Syntax and types.** `inout name: T` on a parameter, `T` one of `[i64]`, `[u8]`, `[bool]` or `[T]` with `T: Element`
  (section 6c), `i64` or `bool` under R-6g-4, a non-generic struct under R-6g-2, a map under R-6e-7 and R-6e-14, or a vector under R-6e-10; any other type is `type/inout_type`. The mode is not part of the type. The caller must write `inout`
  at the call (`type/inout_argument` if missing or extra). The marked argument must name a mutable place of
  exactly the parameter's type (see R-6g-3 for field paths). Immutable locals, ordinary parameters,
  read-only `match` binders, elements and calls are rejected. `inout` is contextual, not a keyword;
  malformed or combined modes (e.g. with `take`) are `parse/inout_syntax`.
- **What the callee may do.** Read, index, `len`, `copy`, and assign elements (`a[i] = e`, with the usual bounds,
  `u8` range and refinement proofs). Never `a = ...` (`type/immutable`), resize, move, store or return the view
  itself (`type/array_alias`; return `copy(a)` instead). (LG5) It may reassign an `inout` array parameter as a
  whole (`input = repack(input, hi)`, `a = alloc(len(old(a)) + 2, 7)`), the right side obeying the usual rules for
  assignment to a typed array (section 6c, including the narrow element type of `alloc`). Such a parameter is
  resizable: the caller learns its new length only from `ensures` (`len(a) == ...`, `len(old(a))`); without a length
  clause the length is unknown after the call. A host entry (R-6f, explicit mode) with such a parameter is
  `type/entry_signature` ("host entry F cannot reassign inout array parameter A"), because the host owns the buffer
  and its length. The Rust backend keeps `&mut Vec<T>` for such a parameter; the LLVM backend passes a pointer to the
  caller's `{ptr, len, cap}` slot and frees the old buffer on reassignment. A callee that never reassigns the whole
  parameter is unchanged (`&mut [T]`, pointer and length). Reassigning needs the `alloc` effect when the right side
  allocates; element assignment still needs none. It may pass `a` unmarked to a read-only parameter or
  forward it as `inout a`.
- **Exclusivity.** A whole marked binding may occur nowhere else in the same call's arguments, including inside nested calls
  or `len(a)`: `f(inout a, a)`, `f(inout a, len(a))` and `f(inout a, inout a)` are `type/inout_alias`. Compute
  `let n: i64 = len(a);` first. Read-only arguments may alias each other. Holds in every mode, also under
  `--no-verify` and `--trust-callers`. Mutable access while a `match` arm views the same root is `type/recursive_borrow`.
- **`old`.** `old(a)` names the entry contents of an `inout` array parameter `a` of the enclosing function; legal only in
  that function's `ensures`, `invariant` and `assert` (`type/old_context`, `type/old_argument` otherwise: locals,
  ordinary parameters and nested `old` are rejected). For an `inout` parameter `x` of a non-generic struct
  (LG22) `old(x)` is its entry value and `old(x.f.g)` means `old(x).f.g`. The operand may also be a place chain of
  `.field` and `[index]` steps from such a parameter, or `len` of one that is an array (LG23): `old(a[i])` means
  `old(a)[i]`, `old(len(c.a))` means `len(old(c).a)`. An index inside the operand is read at the current point, so it
  may not mention an `inout` parameter or `old` (`type/old_argument`; write `old(a)[old(b)[0]]` explicitly). Other
  operands (`old(a[0] + 1)`, a generic struct parameter, which cannot be `inout`) stay `type/old_argument`. It always means function entry. In `requires`
  a parameter is its entry value; in `ensures` an `inout` parameter is its final value. `len(a) == len(old(a))` is
  always known for a fixed-length `inout` parameter (one the callee never reassigns as a whole); for a resizable one
  (LG5) only `ensures` relate `len(a)` and `len(old(a))`. `old` is a proof object: no copy, no run-time check. Arrays still cannot be compared with `==`.
- **Frame.** After `f(inout a, ...)` the caller knows only what `f`'s `ensures` says about `a`: fresh contents, and the
  same length when the parameter is fixed-length (a resizable parameter's length comes only from `ensures`). An unconstrained element is arbitrary, even if the body never writes it, so state the frame
  (`k != i ==> a[k] == old(a)[k]`). Other variables keep their values. The `requires` are proved on the pre-call
  arrays and `old` refers to them. `ensures` of an `inout` function are always applied at the call.
- R-6g-1 **Inout calls in expressions.** A user function call with an `inout` argument may occur at any
  depth in a code expression if all of the following conditions hold. The statement is one simple statement.
  For `if`, `while` or `match` it consists only of the condition or scrutinee.
  1. A place passed `inout` has no overlapping mention elsewhere in the statement, including the assignment target, other
     operands or arguments, and other `inout` calls. Overlapping mentions in the same call's arguments are
     `type/inout_alias`. Elsewhere the error is `type/inout_context` and says that the binding is passed
     inout and also mentioned elsewhere in this statement.
  2. The call is evaluated exactly once when the statement is reached. It cannot be in a `while` condition,
     the right operand of `&&` or `||`, or any other part that may be skipped or repeated. Violations are
     `type/inout_context` and say that the call may be skipped or repeated.
  3. The statement is not an element or field assignment. Violations are `type/inout_context`.
  4. No call with an effect other than `alloc` runs before the inout call in the statement. Violations are
     `type/inout_context` and say that a call with effects runs before it.

  Rust and LLVM evaluate the call in place. Expressions evaluate left to right, with arguments evaluated
  before the call they belong to. The verifier may model inout calls in fresh temporaries before the statement,
  in their original evaluation order. The existing checks on whole-expression calls still apply. The builtins
  `copy_range`, `store_be` and `store_le` remain whole expression statements, and the rules for `par` are unchanged.
- R-6g-2 **Struct inout parameters.** `inout x: S` is allowed for a non-generic struct `S`, emitted as
  `x: &mut S`. The callee may assign fields of `x` (also nested), use R-6e-2 on `x.f`, pass `x.f` to
  `map_get_mut` and forward `inout x`. Whole reassignment `x = e` is `type/immutable`. A generic struct is
  `type/inout_type`. The caller passes `inout y` for a `let mut` local or `inout` parameter `y`, with the alias
  rules for arrays. `old(x)` and `old(x.f)` give the entry value (LG22). After the call the caller
  knows only the callee's `ensures` about `y`, with one frame rule (LG5): the caller keeps `len` of every array
  field of `y` (through nested struct fields) that `f` cannot change, that is, `f` contains no field assignment,
  move, or resizing `inout` pass of that field or of an enclosing field (transitively through the functions `f`
  calls). All other contents of `y` come only from `ensures`. A callee that does change the field leaves its
  length unknown (`index_oob` at a later index relying on the old length). A field assignment `p.f = alloc(n, v)`
  takes its element type from the field type, as a typed `let` does (`p.a = alloc(4, 200)` for a `[u8]` field,
  `p.b = alloc(2, true)` for a `[bool]` field; `alloc(3, 300)` for a `[u8]` field is `verify/elem_range`).
  Tests: `tests/runtime/lg5_rt_struct_len_frame.pruv`, `tests/runtime/lg5_rt_field_alloc_hint.pruv`,
  `tests/must_reject/lg5_struct_len_changed.pruv`, `tests/must_reject/lg5_field_alloc_u8_range.pruv`.
- R-6g-3 **Inout field paths.** A user call may pass `inout x.f1...fn` with at least one field.
  The root must be a `let mut` local, an `inout` parameter, or the mutable binder of a
  `map_get_mut` or `vec_at_mut` `Some` arm. The field type must exactly match the parameter type.
  An invalid root or mismatched type is `type/inout_argument`.
  The path must be disjoint from every path on that root held by an enclosing mutable collection arm
  or field-path `match` (R-6e-8). An overlap is `type/recursive_borrow`.
  Paths overlap when either is a prefix of the other, with the bare root as the empty path.
  Overlap with any other argument mention, including another marked argument, is `type/inout_alias`.
  Rule 1 of R-6g-1 applies per path, so disjoint sibling reads and inout arguments are allowed.
  The verifier uses the current field value as the pre-call argument and writes a fresh value constrained
  by the callee's `ensures` back along that path only. Where `old` is supported it refers to the pre-call
  value. Sibling fields and other elements of enclosing collections retain their values.
  Rust passes `&mut x.f1...fn` directly, including when `x` is already a mutable reference.
- R-6g-4 **Scalar inout parameters.** Non-entry Pruv functions may declare `inout n: i64`
  or `inout n: bool`, read `n` and assign it with `n = e`. The caller marks an exactly
  matching mutable local, inout parameter, field path or mutable scalar collection binder
  with `inout`; the exclusivity rules of R-6g-1 and R-6g-3 apply. In `ensures` the name
  denotes the final value. The caller learns only what those clauses guarantee about it.
  `old(n)` is `type/old_argument` and a scalar inout root entry is `type/entry_signature`.
  Rust uses `n: &mut T`, reads `(*n)`, writes `*n = e;` and passes a mutable borrow.
  The LLVM backend passes a pointer to the scalar (a load on read, a store on write), so both backends support
  scalar `inout`. A scalar `u8` does not exist. Tests: `tests/runtime/lg5_rt_scalar_inout.pruv`,
  `tests/must_reject/lg5_scalar_inout_ensures.pruv`.
- **Generics.** `fn swap<T: Element>(inout a: [T], ...)` works like any G3 function; `swap::<i64>(inout w, i, j)`
  (`examples/g5_generic_swap.pruv`). It is specialized per element type, also across modules.
- **Loops.** A loop that passes an array as `inout` treats it as changed: an `inout` parameter or a local keeps its
  length unless the loop can also reassign it as a whole (LG5) or the callee's parameter is resizable. State what the loop preserves in `invariant`s
  (`old` may be used there); `--infer` may miss such invariants but never keeps a false one.
- **Refinements.** An annotation on an `inout` parameter is an entry `requires` only, not an output guarantee; state
  that in `ensures`. A refined local passed `inout` must satisfy its refinement again right after the call
  (`verify/refinement_assign`), proved from `ensures` (a length-only refinement follows from the preserved length).
- **Modes and back ends.** Verified and safe (`--no-verify`) modes both work. Safe mode checks the `requires` of an
  `inout` function on every call to it, including under `--trust-callers`, which only skips checks at host entries;
  other internal calls follow the existing safe-mode rules. Rust emits
  `&mut [T]`; LLVM passes the existing pointer and length with `noalias nocapture` and no `readonly`, and never frees
  it. Neither allocates or copies for the parameter. Programs without `inout` compile exactly as before.
- **Host limits.** In explicit mode a root `pub` non-generic function may take `inout` array parameters
  (`[i64]`, `[u8]`, `[bool]`) on both back ends: Rust emits `&mut [T]`, the LLVM ABI symbol takes the pointer and
  length, and `--rust-bindings` forwards `b: &mut [T]` (section 7; `tests/runtime/g5_r19_resident_entry`). The host
  supplies the buffer and keeps its length. A scalar `inout` entry parameter is `type/entry_signature`; an `inout`
  struct is allowed only as the R-6f-2 typed state entry. In legacy mode an `inout` function is automatically
  internal (private, child-`pub` and generic functions may always use `inout`); a
  `trusted rust fn` with `inout` is `type/foreign_signature`. Give the host an ordinary function that owns a
  `copy` of its input, calls the `inout` helpers on it and returns it.
- **Not supported.** Elements as `inout` places,
  `str`, collection types outside R-6e-7, or generic struct `inout` parameters (non-generic structs are R-6g-2), `old` of anything but a place chain (or its `len`) from an array or struct `inout` parameter, mutable host entries other than `inout` arrays and R-6f-2.
- **New error codes:** `inout_syntax`, `inout_type`, `inout_argument`, `inout_alias`, `inout_context`, `old_context`,
  `old_argument`; reused: `immutable`, `array_alias`, `recursive_borrow`, `entry_signature`, `foreign_signature`,
  `reserved`, `refinement_assign`.
- **Field places (LG6).** A scalar field path (`f(inout c.n)`, `f(inout d.c.n)`) and a resizable array field
  (`f(inout d.a)` where `f` reassigns `a`) work on both back ends. After the call the caller knows the field's
  length only from the callee's `ensures`. Tests: `tests/runtime/lg6_rt_field_inout.pruv`,
  `tests/must_reject/lg6_field_inout_reassign_len.pruv`.

## 6g2. Borrowed slice arguments: `subview` (LG6)

- R-6g-5 **Form.** `subview(b, lo, hi)` passes the elements `b[lo]` to `b[hi - 1]` to a function without a copy.
  `b` is an array (`[i64]`, `[u8]`, `[bool]` or a generic `[T]`): a local, a parameter or a field path rooted at
  one. Any other source (a call result such as `copy(b)`, an index) is `type/view_context`.
  `lo` and `hi` are `i64`. There is no lifetime syntax and no slice type. The callee declares an ordinary
  non-`inout` array parameter and cannot tell whether it got a whole array or a view.
- R-6g-6 **Where it may appear.** Only as a direct argument of a call to a Pruv function, in a non-`inout`
  parameter whose array type matches `b`. Anywhere else is `type/view_context`: a `let` or assignment, a `return`,
  a struct literal, a contract, an argument of a builtin or foreign function, an `inout` position, or anywhere
  inside the three operands of another `subview` (also through a nested call); the error is at the inner `subview`. `subview` is a builtin name (`type/reserved`); the shorter `view` was not taken because existing programs define functions named `view`.
  Exception (LG8): `copy(subview(b, lo, hi))` is allowed, with the `subview` as the only direct argument of the builtin `copy`; it yields an owned array of `hi - lo` elements with the same obligations as above (`0 <= lo <= hi <= len(b)`, `verify/index_oob`). Every other builtin (for example `len(subview(...))`) still rejects.
- R-6g-7 **Proof.** At the call the verifier proves `0 <= lo`, `lo <= hi` and `hi <= len(b)`
  (`verify/index_oob`). The callee's parameter `v` then has exactly `len(v) == hi - lo` and
  `v[i] == b[lo + i]` for every `i` in range, so the callee's `requires` are proved from `b`.
- R-6g-8 **No escape, no write.** The rules for array parameters already keep a view inside the call. The callee
  cannot write it (`type/immutable`), return it or store it in a field (`type/array_alias`), or push it into
  a `Vec` (`type/foreign_owned_type`) without `copy`. The view exists only during the call.
  `subview(b, ...)` counts as a use of `b` for `type/inout_alias`, so neither `b` nor a struct holding it
  can be passed `inout` in the same call. A `par` branch argument cannot be a view
  (`type/parallel_argument`). Nothing can write `b` while the view lives.
- R-6g-9 **Code.** Rust passes `&b[lo..hi]` (unchecked when verified). LLVM passes the array pointer advanced by
  `lo` elements and the length `hi - lo`. Neither copies or allocates. The callee is compiled as before, so
  index checks it proved stay removed. In safe mode (`--no-verify`) the caller checks the range at run time
  and panics with the array range error.
- Tests: `tests/runtime/lg6_rt_view*.pruv`, `tests/must_reject/lg6_view_*.pruv`. New error code: `view_context`.
- R-6g-10 **String views (LG25).** `subview(x, lo, hi)` may also be the direct argument of a non-`inout` `str`
  parameter. Then `x` is a `str` or a `[u8]` (a local, a parameter or a field path rooted at one), and the callee
  gets a borrowed `str` of the bytes `x[lo..hi]`: no copy, no allocation, no `! alloc`. R-6g-6 and R-6g-8 apply
  unchanged; a `str` source for an array parameter stays `type/type_mismatch` (section 6b: the only `[u8]` to `str`
  conversion that makes a value is `str_from_utf8`).
  - `str` source: the verifier proves `0 <= lo <= hi <= len(x)` (`string_slice_range`), then
    `is_char_boundary(x, lo) && is_char_boundary(x, hi)` (`utf8_boundary`), as for `substr`.
  - `[u8]` source: the verifier proves `0 <= lo <= hi <= len(x)` (`index_oob`), then that the span is valid UTF-8
    (`utf8_invalid`): `utf8_valid` of the span (its ASCII fact), or `utf8_valid(x)` with both `lo` and `hi`
    char boundaries of `x` (the byte formula of `is_char_boundary`: the end points, or a byte `< 128` or `>= 192`).
    A valid UTF-8 buffer cut where no continuation byte starts gives valid pieces, so this is sound.
  - The callee's parameter `t` has `len(t) == hi - lo` and `t[k] == x[lo + k]`.
  - Code: Rust passes `x.get_unchecked(lo..hi)` for a `str` source and `str::from_utf8_unchecked` of the byte
    span for a `[u8]` source when verified, so the proven path has no run-time UTF-8 or boundary check. LLVM passes
    the pointer advanced by `lo` and the length `hi - lo`. In safe mode (`--no-verify`) the caller checks the range,
    then the boundaries (`str` source, the `substr` messages) or the bytes (`[u8]` source, `pruv: invalid UTF-8
    (len n)` with `n = hi - lo`), and panics.
  - Tests: [views](tests/runtime/lg25_str_view.pruv), `tests/must_reject/lg25_*_view_*.pruv`.

## 6h. Files (G10)

`fs` is the file-system effect, independent of `io` (stdout) and `alloc`. Direct and transitive calls
must declare their effects. `File` is an owned type: local, borrowed parameter, `take` parameter or
result; never nested in aggregates/collections or exposed in a host-entry signature. Host entries take paths
as `str`; paths are relative to the process working directory.

| Operation | Effects | Meaning / contract |
|---|---|---|
| `file_open_read(path: str) -> File` | fs | readable iff ok; not writable; size 0 |
| `file_create(path: str) -> File` | fs | truncate/create; writable iff ok; not readable; size 0 |
| `file_read_all(take f: File) -> File` | fs, alloc | requires readable; read remaining bytes into the handle; readable iff ok; not writable; failure clears buffer |
| `file_write_str(take f: File, text: str) -> File` | fs | requires writable; write UTF-8 bytes; writable iff ok; not readable; size 0 |
| `file_write_bytes(take f: File, data: [u8]) -> File` | fs | same as write_str, for bytes |
| `file_close(take f: File) -> bool` | fs | consume/close; true only if ok and final flush succeeds |
| `file_ok/readable/writable(f: File) -> bool` | none | status/mode observers, usable in code and contracts |
| `file_size(f: File) -> i64` | none | buffered byte count, >= 0 |
| `file_byte(f: File, i: i64) -> i64` | none | requires 0 <= i < size; returns 0..255 |

Pass ownership with `move f`; a moved/closed binding cannot be read until reassigned. Branch joins and loops
follow §6d. Borrowing never transfers ownership. Every live owner is dropped on scope exit, including early
return; drop closes the handle and ignores errors. Explicit close reports flush success; it does not sync
to disk. OS failures become `file_ok == false`, not panics. Verification models each operation's result only
through its catalog contract: writing and reopening a path does not prove its contents.

```
fn sum_bytes(f: File) -> i64
  requires file_size(f) <= 1000000
  ensures result >= 0
{
  let mut s: i64 = 0;
  let mut i: i64 = 0;
  while i < file_size(f)
    invariant 0 <= i && i <= file_size(f) && 0 <= s && s <= 255 * i
    decreases file_size(f) - i
  {
    s = s + file_byte(f, i);
    i = i + 1;
  }
  return s;
}
pub fn checksum(path: str) -> i64 ! fs, alloc
  ensures result >= -1
{
  let f: File = file_open_read(path);
  if !file_readable(f) { return -1; }
  let g: File = file_read_all(move f);
  if !file_ok(g) || file_size(g) > 1000000 { return -1; }
  return sum_bytes(g);
}
```

Rust uses `pruv_File`; LLVM uses an opaque pointer and the same Rust operations in the §6d shim emitted with
`--rust-bindings`, including observers and drop. Link with Rust; `--emit exe|obj` rejects File programs with
`codegen/foreign_target`. `--foreign-checks` follows §6d: with `--no-verify`, reading a write handle exits 101
with `pruv: foreign requires violated:`. Programs without File retain their generated output.

## 6i. Parallel calls: par (G7)

A `par` statement runs two or more semicolon-terminated unit calls and joins them
before the next statement. `par` remains an ordinary identifier elsewhere.

```pruv
fn set(inout a: [i64], x: i64) -> ()
  requires len(a) > 0
  ensures a[0] == x
{ a[0] = x; }

fn pair(inout a: [i64], inout b: [i64]) -> ()
  requires len(a) > 0 && len(b) > 0
  ensures a[0] == 7 && b[0] == 9
{
  par { set(inout a, 7); set(inout b, 9); }
}
```

Each branch must call an ordinary Pruv function returning `()`. Arguments are
binding names, integer/bool literals, or `inout name`; bind calculations first.
Boundary types are `i64`, `bool`, primitive arrays (`[i64]`, `[u8]`, `[bool]`),
and supported concrete `Element` specializations. Arrays are borrowed, never
moved or copied. A binding passed `inout` by one branch may not be read or written by another
branch; read-only inputs may repeat. These rules apply even with
`--no-verify --trust-callers`.

The entire reachable branch call graph may use only the `alloc` effect, with no
trusted Rust functions, catalog operations, or nested `par`. Ordinary pure
builtins and sequential helpers are allowed. The caller may perform I/O outside
the group; branch allocation effects propagate to it. Scheduling adds no source
effect. Arguments are captured once in branch and parameter order before workers
start. All started workers join before normal continuation; successful results
match sequential calls in source order. Execution order and overlap are unspecified.

Verification checks every callee normally and proves all branch preconditions
against the same group-entry state. At join it assumes their postconditions,
including unit calls without inout; written arrays retain their lengths and only
contracted contents. `old` in a branch contract refers to its group-entry values.
Safe mode retains bounds, arithmetic and branch precondition checks even under
`--trust-callers`; postconditions are not checked dynamically.

A caught worker panic joins started workers and exits 101. Spawn failure joins
started workers, prints `pruv: parallel spawn failed`, and exits 101. Failures do
not roll back mutations. Process-fatal failures and LLVM runtime check exits may
terminate immediately; no Rust panic unwinds across the callback ABI.

The Rust backend uses scoped threads. LLVM keeps branch bodies in LLVM and emits
private callbacks with stack contexts; its generated `--rust-bindings` file
contains the synchronous Rust thread bridge. Emit IR and bindings, compile the
IR with the usual Pruv runtime, and link with a Rust host. Native `--emit exe` and
`--emit obj` reject `par` with `codegen/parallel_target`.

## 6j. Cost bounds: cost and spent (Stage 5)

A function may state an upper bound on the work it performs. The verifier proves
that every execution stays within it.

```pruv
fn sum(a: [i64]) -> i64
  requires len(a) <= 1_000_000
  cost len(a)
{
  let mut s: i64 = 0;
  let mut i: i64 = 0;
  while i < len(a)
    invariant 0 <= i && i <= len(a)
    invariant spent == i
    decreases len(a) - i
  { s = s + a[i]; i = i + 1; }
  return s;
}
```

The header clause `cost <i64 expr>` comes last (`requires* ensures* decreases?
cost?`), at most once. It is a contract expression over the parameters, evaluated
in the entry state, and must be defined there; `result`, `old` and `spent` are not
allowed in it. `spent` is the number of units this call has spent so far. It may
appear only in `invariant` and `assert` of a function with `cost`. `cost` and
`spent` are keywords.

Units: each `while` iteration adds 1 before the body; a call `f(args)` adds
`1 + C_f` evaluated on the actual arguments (the callee's stated bound, not its
real spending); `alloc(n, v)` adds `1 + n`; `copy(a)` adds `1 + len(a)`; `truncate(a, m)` adds `1 + m`;
`resize(a, n, v)` adds `1 + n`; `par`
adds every branch's call units once at group entry from the captured arguments.
Everything else costs 0. Every return must satisfy `spent <= C`, where `C` is the
bound at entry. A function without `cost` is unchanged.

A function with `cost` may call only functions with `cost`
(`effect/cost_callee`), and may not use foreign, catalog or file operations
(`effect/cost_unsupported`). Loops forget `spent` like any assigned variable, so
a loop needs a `spent` invariant. A failed return check is `verify/cost_bound`
with a counterexample; an undefined bound is `verify/contract_domain`. Generic
functions may have `cost`, and a `pub` function's bound is part of its interface.

Verified mode erases `cost` and `spent`: Rust source, LLVM IR and bindings are
byte-identical to the program without them. Safe mode (`--no-verify`) keeps a
checked counter per cost function on both backends. It evaluates `C` at entry
with the run-time contract evaluator (after the function's own `requires` checks
unless `--trust-callers`; outside its domain `pruv: requires undefined`). A
return with `spent > C` or a negative `C`, a call whose evaluated callee bound
is negative, or counter overflow panics `pruv: cost exceeded` (exit 101). `--trust-callers` does not disable these
checks; `assert` stays erased.

## 6k. The standard module: std (G8)

Full contract: `docs/G8_SPEC.md`. Examples: `examples/g8_std_clamp.pruv`, `examples/g8_std_index.pruv`,
`examples/g8_std_modules/`.

The compiler ships one module of twelve verified functions written in ordinary Pruv. It is source, not a
precompiled library: the bundled file is parsed, checked and proved together with your program.

```pruv
mod std;

pub fn bounded(x: i64) -> i64
  ensures 0 <= result && result <= 100
{
  return crate::std::clamp_i64(x, 0, 100);
}
```

- **One import, at the root.** `mod std;` in the root file loads the bundled module. It is the only way in: no `use`,
  no bare `std::`, no implicit import. `mod std;` in a child is `parse/std_import_location`. Like any `mod`, it
  switches the program to explicit visibility (section 6f), so a root `main` must be `pub`.
- **Qualified calls.** Every module, root or child, calls `crate::std::<name>(...)`. A function of your own named
  `min_i64` stays your own; plain names never fall back to std. Std functions may not appear in contracts: write the
  property itself (`forall k in 1..len(a): a[k - 1] <= a[k]`), not `crate::std::is_sorted_i64(a)`.
- **Contracts.** The twelve functions and their exact contracts are listed in `docs/G8_SPEC.md` section 4:
  `min_i64`, `max_i64`, `clamp_i64` (requires `lo <= hi`), `abs_i64` (requires `x > i64::MIN`), `checked_add_i64`
  and `checked_sub_i64` (return `None` exactly when the mathematical result leaves i64), `sum_i64`, `contains_i64`,
  `index_of_i64` (the first matching index, `None` when absent), `is_sorted_i64` (nondecreasing, empty and singleton
  arrays are sorted), `array_max_i64` (requires `len(a) > 0`) and `reverse_i64 ! alloc` (a fresh array; the input is
  unchanged). A call proves the callee's `requires` on your arguments and then knows only its `ensures`, exactly as
  for your own functions.
- **Sum bounds.** `sum_i64` requires `len(a) <= 1_000_000` and every element in `-1_000_000_000..=1_000_000_000`.
  This keeps every prefix of the left-to-right sum within 10^15. It rejects some inputs whose sum would fit; bounding
  only the final sum would not be sound. For other inputs, write your own loop or use `checked_add_i64`.
- **Proof on every compile.** A verified compilation that imports std proves all twelve bundled functions, used or
  not, before generating anything. There is no cache, certificate or trusted copy. A std proof that fails or returns
  unknown fails the whole compilation with its ordinary code (`verify/ensures`, `verify/overflow`, ...) and the file
  label `@std/std.pruv`. Without `mod std;`, nothing is loaded and output is byte-identical to before.
- **Errors.** Your calls report ordinary diagnostics: `verify/call_requires` for an unproved precondition (with a
  counterexample), unknown-function resolution for a misspelled name, unknown-module resolution when `mod std;` is
  missing. `parse/std_source` means the compiler's own bundled resource is missing or unreadable: reinstall the
  compiler; do not create a `std.pruv` of your own.
- **Safe mode.** `--no-verify` proves nothing (neither your code nor std) but keeps parsing and checking, checks
  index and overflow at run time, and checks every std `requires` on every call (`pruv: requires violated:
  crate::std::<name>`). `--trust-callers` only skips the root entry's `requires`; the checks inside std calls stay.
- **Host wrappers.** Std functions are Pruv-only APIs. They are never exported as `pruv_abi_*` symbols or Rust
  bindings, and both backends give them internal linkage. To call one from Rust or C, write a root `pub` wrapper
  with a plain-value signature:
  ```pruv
  mod std;
  pub fn safe_add(a: i64, b: i64) -> Option<i64> { return crate::std::checked_add_i64(a, b); }
  ```
  A wrapper may return the `reverse_i64` result; ownership passes to the host as for any owned array result.
- **Migrating a module named std.** `std` is now reserved for the bundled module. If your project has a `std.pruv`
  next to the root and declares `mod std;`, compilation fails with `parse/std_module_conflict` (even if the file is
  identical to the bundle, and also for a directory or dangling link of that name). Rename the file and the
  `crate::std::...` paths that used it, e.g. to `mystd.pruv` and `crate::mystd::...`. Other module names and local
  identifiers named `std` are unchanged.
- **Packaging.** The Python compiler reads `stdlib/std.pruv` next to its `pruvc/` package directory, never from the
  working directory or an environment path, so copy or install both directories together. The Rust compiler embeds
  the same file at build time; its binary needs no file next to it.

## 6l. Child processes: proc (G10)

Full contract: `docs/G10_PROC_SPEC.md`. Tests: `tests/g10_proc/`, `tests/runtime/g10p_r*`.

`proc` is the effect for running child programs, independent of `io`, `fs` and `alloc`. `Process` is an owned
type with the `File` placement rules, and it is **must-consume**: every handle, including the one a failed spawn
returns, must end in `proc_wait` or `proc_kill`; scope exit cannot drop it. `ProcessArgs` is an ordered list of
UTF-8 arguments with the same placement rules, affine with auto-drop; an argument is never split or parsed.

| Operation | Effects | Meaning / contract |
|---|---|---|
| `proc_args_new() -> ProcessArgs`, `proc_args_push(take a, arg: str) -> ProcessArgs` | alloc | build the list; `proc_args_len` counts it; push requires `proc_args_len(a) < 2^40` and adds one (R-6l-2) |
| `proc_spawn(program: str, args: ProcessArgs, cwd: str) -> Process` | proc, alloc | no requires; ok and stdin open, or not ok with error 1 or 2. `cwd == ""` inherits |
| `proc_spawn_env(program: str, args: ProcessArgs, cwd: str, env_names: ProcessArgs, env_values: ProcessArgs) -> Process` | proc, alloc | same contract as `proc_spawn`; the child inherits the parent environment, then the pairs are set in order (last wins, no removal); unequal counts, empty name, `=` or NUL in a name, NUL in a value: error 1 |
| `proc_write_str/write_bytes(take p, ...) -> Process` | proc, alloc | requires ok and stdin open; writes everything |
| `proc_close_stdin(take p) -> Process` | proc, alloc | requires ok and stdin open; ensures stdin closed |
| `proc_read(take p, stream: i64, max: i64) -> Process` | proc, alloc | requires ok, stream 1 or 2, `1 <= max <= 2^40` (R-6l-1); next chunk of at most `max` bytes; EOF iff the chunk is empty |
| `proc_read_all(take p) -> Process` | proc, alloc | requires ok and stdin closed; the rest of both streams (bytes not delivered by earlier reads) |
| `proc_bytes(p, stream: i64) -> [u8]` | alloc | independent copy of a public buffer |
| `proc_wait(take p) -> Result<i64>`, `proc_kill(take p) -> Result<i64>` | proc, alloc | no requires; reap (kill first) and consume; `Ok(code)` or `Err(error)` |
| `proc_ok/error/stdin_open/eof/size/byte(p, ...)` | none | observers, usable in code and contracts |

`proc_error` is `0..6`: 0 none, 1 bad input (empty program, NUL, a Windows `.bat`/`.cmd` file, which is never
run), 2 spawn failed, 3 read, 4 write, 5 reap, 6 kill request. The first error is kept; a failed update closes
stdin and empties both buffers, so only `wait`/`kill` remain allowed. `Ok(7)` is exit code 7, not a failure; a
Unix signal exit is the negative signal number, a Windows code its unsigned 32-bit value. Both output pipes are
drained internally whichever stream is read, so a full stderr cannot hang a stdout reader. Observers never
query the OS: the same value always answers the same, and new output becomes visible only at the next effect
operation. Protocol deadlocks, timeouts and process trees are not handled.

- R-6l-1 A `proc_read` whose `max` is not proved to be in `1..=2^40` is `verify/call_requires`. Tests:
  [accept](tests/spec/r6l_1_read_max_ok.pruv), [reject](tests/spec/r6l_1_read_max_zero.pruv).
- R-6l-2 `proc_args_push(take a, arg)` requires `proc_args_len(a) < 2^40` and ensures the count is one more; a
  push whose count bound is not proved is `verify/call_requires`. Tests:
  [accept](tests/spec/r6l_2_args_push_ok.pruv), [reject](tests/spec/r6l_2_args_push_unbounded.pruv).

```
pub fn version(program: str, cwd: str) -> Result<i64> ! proc, alloc
{
  let mut args: ProcessArgs = proc_args_new();
  args = proc_args_push(move args, "--version");
  let mut p: Process = proc_spawn(program, args, cwd);
  if !proc_ok(p) { return proc_wait(move p); }
  p = proc_close_stdin(move p);
  if !proc_ok(p) { return proc_kill(move p); }
  p = proc_read_all(move p);
  if !proc_ok(p) { return proc_kill(move p); }
  let out: [u8] = proc_bytes(p, 1);
  return proc_wait(move p);
}
```

Rust uses `pruv_Process`/`pruv_ProcessArgs` and reader threads; LLVM uses opaque pointers and the same Rust
runtime in the §6d shim emitted with `--rust-bindings`. `--emit exe|obj` rejects proc programs with
`codegen/foreign_target`. Functions with `Process` parameters are internal only, so their contracts are proved,
never checked at run time. Run on macOS and Linux (CI) and on Windows (`tests/g10_proc/windows.py`, manual).

## 6m. TCP sockets: net (G10)

Full contract: `docs/G10_NET_SPEC.md`. Tests: `tests/g10_net/`, `tests/runtime/g10n_r*`.

`net` is the effect for blocking TCP, independent of `io`, `fs`, `proc` and `alloc`. `Socket` (a connection) and
`Listener` (a bound server port) are owned types with the `File` placement rules, and both are **must-consume**
like `Process`: every handle, including a failed one, must end in `net_close` or `net_listener_close`.

| Operation | Effects | Meaning / contract |
|---|---|---|
| `net_listen(host: str, port: i64) -> Listener` | net, alloc | requires `0 <= port <= 65535` (0 = OS-chosen); ok and `net_listener_port` the bound port, or error 1 or 2 and port 0 |
| `net_accept(l: Listener) -> Socket` | net, alloc | borrows `l`; requires `net_listener_ok(l)`; ok and write-open, or error 5. Blocks |
| `net_connect(host: str, port: i64) -> Socket` | net, alloc | requires `1 <= port <= 65535`; ok and write-open, or error 1 or 2 |
| `net_write_str/write_bytes(take s, ...) -> Socket` | net, alloc | requires ok and write-open; writes everything |
| `net_shutdown_write(take s) -> Socket` | net, alloc | requires ok and write-open; half-close, reading continues |
| `net_set_timeout(take s, ms: i64) -> Socket` | net, alloc | requires ok, `0 <= ms <= 86400000` (0 = none); read and write timeout |
| `net_read(take s, max: i64) -> Socket` | net, alloc | requires ok, `1 <= max <= 2^40` (R-6m-1); next chunk of at most `max` bytes; EOF iff the chunk is empty |
| `net_read_all(take s) -> Socket` | net, alloc | requires ok; the bytes up to EOF not delivered by earlier reads |
| `net_bytes(s) -> [u8]` | alloc | independent copy of the public buffer |
| `net_close(take s)`, `net_listener_close(take l) -> Result<i64>` | net, alloc | no requires; release; `Ok(0)` or `Err(first error)` |
| `net_ok/error/write_open/eof/size/byte(s, ...)`, `net_listener_ok/error/port(l)` | none | observers, usable in code and contracts |

`net_error` is `0..6`: 0 none, 1 bad input (empty host, NUL), 2 connect or listen failed, 3 read, 4 write,
shutdown or set_timeout, 5 accept, 6 a timeout set by `net_set_timeout` expired. The first error is kept; a
failed update closes the write side and empties the buffer, so only `net_close` remains allowed. Observers never
query the OS: data becomes visible only at the next effect operation. `net_read_all` does not close the write
side; a peer that waits for our EOF needs `net_shutdown_write` first. TLS, UDP, HTTP, DNS, async and
multiplexing are not part of this slice.

- R-6m-1 A `net_read` whose `max` is not proved to be in `1..=2^40` is `verify/call_requires`. Tests:
  [accept](tests/spec/r6m_1_read_max_ok.pruv), [reject](tests/spec/r6m_1_read_max_zero.pruv).

```
pub fn ask(host: str, port: i64, q: [u8]) -> Result<i64> ! net, alloc
  requires 1 <= port && port <= 65535
{
  let mut s: Socket = net_connect(host, port);
  if !net_ok(s) { return net_close(move s); }
  s = net_write_bytes(move s, q);
  if !net_ok(s) { return net_close(move s); }
  s = net_shutdown_write(move s);
  if !net_ok(s) { return net_close(move s); }
  s = net_read_all(move s);
  let answer: [u8] = net_bytes(s);
  return net_close(move s);
}
```

Rust uses `pruv_Socket`/`pruv_Listener` over `std::net`; LLVM uses opaque pointers and the same Rust runtime in
the §6d shim emitted with `--rust-bindings`. `--emit exe|obj` rejects net programs with
`codegen/foreign_target`. Functions with `Socket` or `Listener` parameters are internal only, so their contracts
are proved, never checked at run time. Run on macOS and Linux (CI) and on Windows (`tests/g10_net/windows.py`,
manual), over the loopback.

## 6n. Readiness loops: async (G10)

`Loop` is an owned, fixed-capacity readiness multiplexer for sockets, listeners, child processes and one-shot
timers. Declare `! async` for loop operations; `alloc` is also required. Socket operations additionally need
`net`, process operations need `proc`, and `ev_remove` / `ev_close` require both. `async` is forbidden in `par`.

| Operations | Meaning |
|---|---|
| `ev_new(cap)` | Create 1..4096 slots; no worker starts until a resource is added |
| `ev_add_socket`, `ev_add_listener`, `ev_add_process` | Consume the loop and resource, returning a loop; `ev_last` is the lowest free slot |
| `ev_add_timer(l, ms)` | Add a one-shot timer, 0..86400000 ms |
| `ev_wait(l, timeout_ms)` | One event; -1 waits indefinitely, 0 polls, otherwise at most 86400000 ms |
| `ev_read(l, slot, max)`, `ev_proc_read(l, slot, stream, max)` | Nonblocking read into the shared public buffer; stream is 1 (stdout) or 2 (stderr); max is 1..2^40 |
| `ev_read_into(take l: Loop, slot: i64, inout dst: [u8], inout n: i64) -> Loop` | Nonblocking read from a socket slot into the caller's buffer |
| `ev_proc_read_into(take l: Loop, slot: i64, stream: i64, inout dst: [u8], inout n: i64) -> Loop` | Nonblocking read from a process slot into the caller's buffer; stream is 1 (stdout) or 2 (stderr) |
| `ev_write`, `ev_proc_write` | Write all bytes to a write-open slot; may block |
| `ev_shutdown_write`, `ev_proc_close_stdin` | End writes while keeping reads available |
| `ev_accept(l, slot)` | Nonblocking accept; `ev_last == -1` if nothing is pending |
| `ev_proc_wait(l, slot)` | Wait and reap (kill a failed slot), free the slot, publish `ev_exit` |
| `ev_proc_wait_for(l, slot, timeout_ms)` | Close stdin and wait at most 0..86400000 ms; if the child exited and its outputs ended, act as `ev_proc_wait` (kind becomes 0), else keep the slot (kind 3, stdin closed, `ev_exit` unchanged) |
| `ev_proc_write_for(l, slot, data, timeout_ms)` | Write with a deadline; on a deadline or a closed pipe stdin is half-closed (`ev_write_open` false) and the slot stays usable, not failed |
| `ev_remove(l, slot)` | Cancel, join workers and free a non-free slot |
| `ev_close(l)` | Consume the loop and end all slots; `Ok(number ended)` or `Err(code)` |
| `ev_ok`, `ev_error`, `ev_cap`, `ev_count`, `ev_last`, `ev_exit` | Observe the last published state |
| `ev_kind`, `ev_eof`, `ev_write_open` | Observe an in-range slot: kind 0 free, 1 socket, 2 listener, 3 process, 4 timer |
| `ev_event_slot`, `ev_event_kind` | Last event: 0 timeout, 1 data, 2 accept, 3 end, 4 timer, 5 slot error |
| `ev_size`, `ev_byte`, `ev_bytes` | Buffer size, in-range byte, and independent owned byte copy |

All updates take `move l` and return the next loop. Except `ev_new` and `ev_close`, operations require `ev_ok(l)`.
Check the slot kind, range, capacity and write-open state before the relevant operation. An add consumes its resource
even on failure. `Loop` cannot be copied, stored in a container or exposed in a host entry; every normal path must
consume it with `ev_close` or transfer it to another owner. Observers read snapshots, never the OS. A byte copy survives
later updates and close. Reads publish EOF only after queued bytes are drained; size zero alone means nothing yet.
The `*_read_into` operations copy up to `len(dst)` queued bytes into `dst[0..n)`, leaving the rest of
`dst`, its length, and the shared public buffer unchanged on a valid call. They set `n` to the number
copied and guarantee `0 <= n <= len(dst)`. An empty queue, an empty destination, or a failed slot
produces zero bytes. On a drained stream they publish EOF or a pending read error as the corresponding
`ev_read` or `ev_proc_read` does. Check `ev_eof` for EOF, which for a process requires both streams
to have been observed drained at end of stream. A return count of zero alone does not indicate EOF.
Wait scans round-robin, reports one event, and frees fired timers. End and error events are reported once.

Error codes: 1 internal (including invalid input), 2 setup (worker spawn, descriptor clone, timeout reset), 3 read, 4 write, 5 write timeout, 6 accept, 7 process I/O, 8 reap.
Slot failures leave the loop usable and require removing that slot; spawn failure makes the loop failed, so close it.
Close preserves the first published failure, even after its slot is removed. Runtime contract violations exit 101.
Writes may block (accepted sockets have no write timeout; process writes can use `ev_proc_write_for`); this is readiness multiplexing, without `async fn` / `await`.
LLVM uses the Rust shim; native `--emit exe|obj` requires the Rust host build and reports `codegen/foreign_target`.

```pruv
fn timer() -> i64 ! async, net, proc, alloc {
  let mut l: Loop = ev_new(1);
  l = ev_add_timer(move l, 50);
  l = ev_wait(move l, -1);
  let r: Result<i64> = ev_close(move l);
  match r { Ok(n) => { return n; } Err(e) => { return -e; } }
}
```

## 6o. Opaque predicates: opaque fn, reveal, opaque_total (L3 P13o)
```
pub opaque fn good(a: [i64], i: i64) -> bool
{ return 0 <= i && i < len(a) && a[i] > 0; }

pub fn read(a: [i64], i: i64) -> i64
  requires good(a, i)
  ensures result > 0
{
  reveal good;
  return a[i];
}
```
- R-6o-1 **Declaration.** `opaque fn` (or `pub opaque fn`) declares a predicate that follows the rules for user
  functions in contracts (section 6, L3 P06) and returns `bool`. Any other result type is `type/opaque_signature`.
- R-6o-2 **Callers see a symbol.** In every proof query an application `F(args)` is an uninterpreted `bool` symbol:
  facts that only carry it along (from `requires` to `ensures`, between equal arguments) need nothing more, and a
  code call of `F` gets no body fact (R-6-1). A proof that needs the body fails with the code of the failed
  obligation (such as `verify/index_oob`) and a hint "opaque F is not revealed here; add reveal F; ...". At run
  time (contract checks under `--no-verify`, host-entry `requires`) the body is evaluated as usual.
- R-6o-3 **`reveal F;`** is a statement in a function body; `F` is resolved like a call path. From that point on the
  current proof path, applications of `F` equal its body. It does not reveal opaque functions that `F` calls. A
  target that is not an opaque function is `type/reveal_target`.
- R-6o-4 **`opaque_total`.** Each opaque body is proved defined (no out-of-range index, overflow or division fault)
  for every argument, with no `requires` and no range assumption on its parameters, so a symbol never stands for an
  undefined value. A body that is not total is `verify/opaque_total`; guard reads with `&&` as above.

Tests: [accept](tests/spec/r6o_1_opaque_bool.pruv) (all four rules);
reject [R-6o-1](tests/spec/r6o_1_opaque_i64.pruv), [R-6o-2](tests/spec/r6o_2_no_reveal.pruv),
[R-6o-3](tests/spec/r6o_3_reveal_target.pruv), [R-6o-4](tests/spec/r6o_4_total.pruv).

## 7. What the compiler proves (all must succeed)
1. Every `a[i]` (read or write): `0 <= i < len(a)`.
2. Every `+ - * unary-` in code: result within i64 range.
3. `/ %`: divisor > 0, dividend >= 0. `alloc(n, v)`: 0 <= n <= 2^40.
4. Callee `requires` at each call; `ensures` at each return; `assert`s.
5. Loop invariants hold on entry and are preserved; `decreases` is >= 0 at each iteration start and strictly decreases.
   For a recursive call, the callee's `decreases` (evaluated on the arguments) is >= 0 and strictly below the caller's
   value at entry (section 6e).
6. Array parameters have `0 <= len(a) <= 2^40` and elements in i64 range (assumed; guaranteed for Pruv callers because every alloc is proven <= 2^40;
   for host callers every boundary checks the length at run time, see "Length bound at host boundaries" below).
Variables not assigned in a loop keep their values across it; assigned ones are only known through invariants.
A variable counts as assigned if the loop body assigns it, stores into its elements, or passes it as an `inout`
argument (section 6g); an `inout` array keeps only its length across the loop.
R-7-6 A `verify/ensures` diagnostic for an explicitly written `ensures` clause points to the `return` keyword on a path
that executes a `return`; on a path that reaches the end of a `()` body without one, it points to the function's `fn`
keyword. Tests:
[accept](tests/spec/r7_6_ensures_fallthrough_ok.pruv), [reject at `fn`](tests/spec/r7_6_ensures_fallthrough.pruv),
[reject at the return](tests/spec/r7_6_ensures_return.pruv).

### Run-time `requires` and modes
A public function's `requires` are checked at run time, left to right with short-circuit evaluation; a violation
panics with `pruv: requires violated`. If evaluation reaches something outside the contract domain (index out of
range, division by zero, `max`/`min` over an empty range, `some_val(None)`, `variant_field` of another variant, ...), it panics with
`pruv: requires undefined`. So write domain conditions first, e.g. `0 <= i && i < len(a) && a[i] > 0`.
A verified program never reaches these (the verifier proves the contract domain), except two run-time resource limits
that also panic with `pruv: requires undefined`: contract arithmetic is evaluated in i128 (an intermediate value or
running sum beyond i128 stops the check), and one public call's `requires` checks share a budget of 2^30 quantifier and
aggregate iterations (`pruv: requires undefined: contract iteration budget exceeded`). Skipped short-circuit operands
and empty ranges cost nothing. These limits never make a false `requires` pass.
Number-range quantifiers and aggregates iterate once per element of the range, so their cost is the range length,
with one exception: the sortedness precondition `forall i in 0..len(a): forall j in i..len(a): a[i] <= a[j]` (or with
inner start `i + 1`; any bound names, same array `a`, exactly this shape) is checked as the equivalent
`forall k in 1..len(a): a[k-1] <= a[k]`, in O(len(a)). The verifier still uses the written form.

A run-time `requires` that passes an argument outside i64 to `band`/`bor`/`bxor` panics with
`pruv: requires undefined: argument outside i64 in band` (`bor`, `bxor` likewise). `--no-verify` gives the same
`band`/`bor`/`bxor` results as the verified build.
With `--no-verify`, a shift count outside `0..64` stops the program before shifting (exit 101, stderr
`pruv: arithmetic error: shift count outside 0..64`); a run-time `requires` that passes such a count or an argument
outside i64 to `wrapping_shl`/`ushr` panics with `pruv: requires undefined: shift count outside 0..64 in ushr`
(`wrapping_shl` likewise).
The operators `<<` and `>>` use the same safe-mode count check and runtime message. Safe-mode `<<` also
checks overflow after shifting (`pruv: arithmetic error: overflow in <<`, exit 101). A runtime `requires` with
bitwise operators reports `pruv: requires undefined: argument outside i64 in &` (using the actual operator),
`pruv: requires undefined: shift count outside 0..64 in <<` (or `>>`), or
`pruv: requires undefined: overflow in <<`, according to which domain check fails.

`--no-verify` emits safe mode: no proof exists, so indexing and arithmetic are checked (panic on out-of-range index
or i64 overflow), and `/` `%` use the verifier's domain (divisor > 0 and dividend >= 0, otherwise panic).
`--trust-callers` makes the public entry skip the `requires` checks (for benchmarking; callers must be trusted).
It never skips the length bound below.

### Length bound at host boundaries
The verifier assumes every array (and `Map`) parameter, every buffer field and every `str` has a length (byte length
for `str`) in `0..=2^40` (item 6 above). Every place where a host buffer first enters proved code checks that bound
before the representation checks, the `requires` and the body, in both compilers and in every mode, including
`--trust-callers` and `--no-verify`. The exceptions are manifest `raw_entries = "crate"` on the Rust backend (below)
and collection fields of a record the entry does not own (R-6f-2 resident state, an R-6e-6 borrowed record): those
lengths are not walked per call, so the caller keeps them at most 2^40 from the last entry call on (section 7 caller
obligations):
- Generated Rust (every output form): every public function that takes an array or `str` (including an `inout` array)
  starts with one check per such parameter, `pruv: invalid representation: n (array length)` / `(str length)`, exit
  101 when not caught. A public function without `requires` that has such a parameter is emitted like one with
  `requires`: a private body `f__pruv_body` and the public `f` that runs the checks and calls it. Verified Pruv callers
  call the body (the caller proved the bound), so the check runs once per host call and never on an internal call;
  under `--no-verify` internal calls go through `f` as they do for `requires`. A `raw_entries = "unsafe"` or `"c"`
  entry checks it in its `pub(crate)` item, so the `pruv_raw` forwarders get the same check. Under manifest
  `raw_entries = "crate"` (exactly that value) the Rust backend's `pub(crate)` entries do not check it: only code in
  the same crate can call them, and that host adapter takes over the obligation below. The check runs before
  the entry writes anything, so the no-unwind store rule below still holds (an entry panics only in its length or
  `requires` checks).
- Proven entries (`--proven-entries`, Rust backend only; rejected with `--backend llvm`): the generated file also
  holds `PruvArr<T>` (a `#[repr(transparent)]` wrapper of `[T]`) and, when a `str` entry needs it, `PruvStr` (of
  `str`), both with a private field, and a module `pruv_proven` with one twin per checked public entry (same name,
  each array parameter as `&PruvArr<T>` / `&mut PruvArr<T>`, each `str` as `&PruvStr`). The only way to make one is
  `PruvArr::new` / `new_mut` / `PruvStr::new`, which perform the same check with the same message and exit 101, so
  the bound is proved by the type: a twin skips the per-call check and runs the same representation checks,
  `requires` and body as its checked entry. A host that wraps resident state once pays the check once, not per
  call. The checked entries stay unchanged. An entry with an owned length-checked parameter gets no twin. A user
  item whose generated Rust name is `PruvArr` or `PruvStr` is `codegen/proven_name_clash` (a private or
  specialized item gets a mangled name, such as `pruv_m_t_1_7_PruvArr`, and does not clash); the other generated names (`pruv_proven`,
  `pruv_proven_types`, `pruv_arr_slow`, `pruv_str_slow`) already carry the reserved `pruv_` prefix
  (`parse/reserved_name`). The guarantee covers every host style, a `#[path] mod` and a crate-root `include!` (see
  the next item).
- Private bodies (every Rust-backend output): the whole generated program sits in a private module, unindented,
  `mod pruv_impl {\n<forwarders><program>}\n#[allow(unused_imports)]\npub use self::pruv_impl::*;\n`. The glob
  re-export reaches the `pub` items (checked entries, `pruv_proven`, `PruvArr`/`PruvStr`) and, inside the host crate,
  the `pub(crate)` raw entries; private items, including the unchecked `f__pruv_body`, cannot be named from the host,
  neither unqualified after an `include!` (E0425) nor as `pruv_impl::f__pruv_body` (E0603, private function) nor
  through a `#[path] mod` (E0425 / E0603, private module). A host `macro_rules!` defined before the generated file is
  in textual scope inside it, so a host `panic!` that returns could let a failed `requires` run the body; the module
  therefore defines a forwarder for each macro name the generator calls unqualified (`assert`, `eprintln`,
  `format_args`, `matches`, `panic`, `println`, `unreachable`, `vec`), each `#[allow(unused_macros)] macro_rules! m
  { ($($t:tt)*) => { core::m!($($t)*) } }` (`std::` for `eprintln`, `println`, `vec`), which shadows any host
  definition. The path goes through the extern prelude, so a `no_std` host still compiles and edition 2015 works
  (a leading `::` there names the crate root). Trusted `rust` blocks that use other macros are the
  author's trust, as before. Because the module name is fixed, a crate root `include!`s at most one generated file
  (two would define `pruv_impl` twice, E0428); attach several as `#[path] mod`s. Host items in the same scope keep
  away from the `pruv_` prefix, as before. The wrap is applied last, after the R-7-3 inline policy, so it changes no
  inlining decision; a module boundary and a macro forwarder are not a codegen cost (budget M4: same `__text` size,
  overlapping instruction and RSS distributions, TIDY 4.13). The LLVM backend is unaffected: its bodies are
  `define internal`, the exported `pruv_abi_*` symbols call a checked entry or, where the caller is trusted or no
  check is needed, the internal body, and the `--rust-bindings` file names no body.
- By-value results (`--proven-entries`): a twin whose last parameter is `inout` of `[i64]` (the result buffer) also
  gets a by-value form `pruv_proven::val::f::<K>(..)`: the twin's parameters without the last one, returning
  `(r, [i64; K])`, where `r` is the entry's result and the array is the result buffer after the call. `K` is the
  host's chosen buffer length (the generated const parameter is named `PRUV__K`, a reserved name, so it cannot
  clash with a parameter named `K`). It runs the twin on a fresh zeroed `[i64; K]`, so it is exactly the twin
  called with a new length-`K` all-zero buffer: the same `requires`, status and words. A small `K` gives what the
  twin gives for that buffer: if it violates a declared `requires`, a checked build panics, and under
  `--trust-callers` the host must meet the twin's contract for that zero buffer, as for the twin itself; an entry
  that declares no length precondition just runs (for example it may return a status for a buffer too short). `K > 2^40` is a compile-time error (a
  `const` assertion, not a run-time check), so the length bound holds by construction. An entry whose last
  parameter is not `inout [i64]` (another element type, not `inout`, or not last) gets no `val` form. The form is
  `#[inline(always)]` and generic. This rule fixes the meaning only; where the words live after optimization
  (registers or a stack array) and how many stores the zeroing costs are up to the Rust compiler, so a host picks
  the form per call site by measurement (Appendix P18c of docs/L3_RESTRUCTURE_SPEC.md).
  Unused forms are never instantiated (no code). `val` is a module, so it does not clash with an entry named `val`.
- Fixed-prefix internal results (`--proven-entries`, Rust backend, both compilers): only in a verified build (not
  `--no-verify`) of a program without G10 runtime or `par` groups, a non-entry, non-foreign function (a generic
  function only through its concrete specializations) that cannot reach a foreign function, a G6 catalog operation (whose boundary helper panics on a failed check or an exhausted check budget) or `print` through calls and whose last parameter is `inout o: [i64]` and whose every use of `o` is a literal index
  (`o[c]` read or write), a comparison of `len(o)` with an integer literal, or `inout o` passed as the last argument
  to another such function, has a prefix length `K` (at least 1, at most 16) computed by the compiler: past every
  literal index, past every literal `len(o)` is compared against, and at least each callee's `K`. Recursive
  functions and any other use of `o` get no `K`. Such a function is emitted as a by-value core that takes and
  returns `[i64; K]` (a single `i64` when `K` is 1; with the logical length `min(len(o), K)`) plus an always-inlined adapter with the old
  signature that copies the first `min(len(o), K)` words in and back. The body sees a buffer of that length instead of
  `len(o)`; for the accepted uses both lengths give the same branches, reads and writes, and words past `K` are never
  touched, so the caller's buffer and status are the same as without the flag (the adapter copies back only when the
  core returns, which is why a function that could unwind after a write never gets the form). No new contract or proof
  obligation is involved; the shape is a syntactic property the code generator checks itself.
- LLVM ABI symbols (`pruv_abi_*` and `__typed`): section 6b, "Run time"; a negative length is rejected by the same
  unsigned compare.
- C adapter (`pruv_c`): an array or `str` length, a `pruv_handle_new` length and the `new_len` of `pruv_hr_*` are
  "oversized" above `min(2^40, isize::MAX / element size)`. `pruv_handle_new` returns null for such a length; a call
  or rebuild with one is status 2 and nothing runs, unless an earlier check fails first (a poisoned handle is status
  1 before any length is looked at). A failed rebuild keeps the old state. Every length of a call (or
  of a batch item) is checked before a UTF-8 or `[bool]` scan reads any of its buffers. Each slice inside a batch item
  is checked the same way when that item runs; the batch count `n` itself has no Pruv bound (it only has to be
  nonnegative and its tables valid).
Caller obligation under `raw_entries = "crate"` (Rust backend): the host adapter in the crate passes only arrays
(including `inout` arrays) whose element count, and `str` values whose byte length, is in `0..=2^40`. Resident
state can be checked once when it is created or replaced; every per-call input and output buffer must meet the bound
too. `--trust-callers` does not waive this obligation. Every other boundary (default and legacy `pub` entries, the
`"unsafe"` forwarders' target, `"c"`, C batch items, the LLVM ABI symbols) keeps its check. Breaking this
obligation is undefined behavior, not a panic: a verified body runs with its checks erased (`get_unchecked`,
unchecked arithmetic) and the no-unwind store has no poison flag, so nothing detects it. The adapter bounds every
length where it is fixed, before the buffer is passed: sizes from configuration or input, sizes an entry returns or
implies (a growth request can exceed 2^40), and every slice its own `pub` functions forward from outside the crate.
An adapter that cannot do this must not use `"crate"`; `"unsafe"`, `"c"` and default entries check on every call.
The generated `pruv_raw` module of a `"crate"` program repeats this obligation in its header comment.
Caller obligations that no check can cover remain the host's: a buffer passed as a Rust slice / `&str` is a valid Rust
value; a raw (pointer, length) pair with length > 0 names that many initialized elements in one live allocation that
nobody else touches during the call; resident state passed to an entry, and a record borrowed under R-6e-6, came from this program's entries, and the host did not grow its collection fields past 2^40 since the last entry call. Apart from
the `raw_entries = "crate"` adapter obligation above, no trusted path (`--trust-callers`, `raw_entries = "unsafe"` /
`"c"`, the `pruv_raw` forwarders, the C adapter) waives the length bound.

### Raw entries (manifest `raw_entries`)
A program package may set `raw_entries` in its manifest (docs/G4B_SPEC.md §3 rule 7). Missing: output is unchanged and
host entries are `pub fn`. `"crate"`: every host entry (and the safe wrapper of LLVM bindings) is `pub(crate) fn`, and
the output ends with `pub mod pruv_raw` holding `PruvStore<T>`: `with(f)` refuses with `PruvPoisoned` once poisoned,
otherwise sets the poison flag, runs `f` on the state and clears the flag only when `f` returns normally, so a caught
panic leaves the store poisoned and its state is never observed or reused; dropping a poisoned store is safe. A host
adapter in the same crate keeps resident state in a `PruvStore` and calls entries only inside `with`. `"unsafe"`: the
same, plus one documented `pub unsafe fn` forwarder per entry in `pruv_raw` (`# Safety`: the caller upholds the
`requires`, passes only state produced by this program and unchanged since, and never reuses state a panic interrupted).
No mode has a public safe entry taking an arbitrary array. `requires` are checked once, in the `pub(crate)` entry.
The crate is the trust boundary: code in the crate is the adapter and may call entries directly. The adapter keeps two
rules the types cannot enforce: `T` owns the state (no reference or shared handle to it outside the store), and the
closure given to `with` never catches a panic (`catch_unwind` inside it would clear the flag).
No-unwind store (L3 P13a.10): on the Rust backend, a verified build (not `--no-verify`) of a program with no foreign
code (G6), no `print` (`println!` panics when stdout fails), no G10 file / process / socket / event runtime and no
`par` group emits `PruvStore` without the poison flag:
same API, `with` always runs `f` and returns `Ok`, `is_poisoned` is always false. In such a program every check inside
a body is erased (callers proved `requires`, no cost counters) and every remaining operation is total on the proved
domain, so an entry panics only in its own entry `requires` check, before it writes; the state in the store is always
the result of complete entry calls (the length checks above also run before any write). Allocation failure and stack overflow abort, they do not unwind. `--no-verify`,
the LLVM backend and programs outside that rule keep the poisoning store. The store no longer catches a host panic
inside `with`, so the adapter rule becomes: the closure changes `T` only through the entries (it already had to, since
the entries' state must come from the entries).
`--trust-callers` does not require `raw_entries`; without it a trust-callers entry is safe only if every caller is
trusted, and `raw_entries` enforces that by visibility.
`"c"`: the Rust backend output is the `"crate"` output (it never contains `extern "C"`), except that the entries
keep their length checks (the host length bound above). LLVM bindings get the
`"crate"` output plus `pub mod pruv_c`, the C adapter. `PruvHandle` is opaque to C (one pointer) and owns the resident
state (`i64` words) in a `PruvStore`; C never sees the state buffer. Every global C symbol of `pruv_c` carries the
manifest package name as a prefix, `<pkg>__` (`pkg` = the manifest `name`): `<pkg>__pruv_handle_new`,
`<pkg>__pruv_handle_free`, `<pkg>__pruv_handle_poisoned`, `<pkg>__pruv_handle_probe_panic`, `<pkg>__pruv_h_NAME`,
`<pkg>__pruv_hr_NAME`, `<pkg>__pruv_hb_NAME`; Rust callers use the same names (`sol::pruv_c::<pkg>__pruv_...`). The
names below are written without the prefix. `pruv_handle_new(len)` returns a handle whose state
is `len` zero words (null for a negative or oversized length); call the program's init entry next.
`pruv_handle_free(h)` frees a handle, poisoned or not (null is ignored); `pruv_handle_poisoned(h)` returns 1/0 (2 for
null). Every entry whose first parameter is `[i64]` (`in` or `inout`), whose other parameters are `i64`, `bool`, `str`
or arrays of `i64`/`u8`/`bool`, and whose result is `i64`, `bool` or unit gets
`pruv_h_NAME(h, args..., ret) -> i32`: the handle's state is the first argument, an array is a (pointer, length) pair,
`bool` is an `i64` 0/1, and an `i64`/`bool` result goes to `*ret` on success. An entry `(prev: [i64], inout next:
[i64]) -> i64` also gets `pruv_hr_NAME(h, new_len, ret)`: it runs the entry from the state into a fresh zeroed buffer
of `new_len` words and makes that buffer the state only when the entry returns 0. An entry whose state parameter is
`inout [i64]` and whose result is `i64` also gets the batch entry `pruv_hb_NAME(h, n, <per user parameter aK>, ret,
done) -> i32` (`n: i64`, `ret: *mut i64`, `done: *mut i64`). Each user parameter becomes a table of `n` items: an
`i64`/`bool` scalar is `aK: *const i64` (n values); an array is `aK: *const <pointer type of the single entry's
parameter>` (n pointers: `*const *const i64`, `*const *mut i64`, `*const *const u8`, `*const *mut u8`) plus
`aK_len: *const i64` (n lengths). `ret` is a table of `n` results. Items 0..n run in order inside ONE store access
(one poison set/clear, one `catch_unwind`), each as the single entry would, and the batch stops after the first item
whose result is nonzero (its result is stored; later `ret` slots and buffers are untouched and later items do not
run). `*done` is the number of items that ran, including the stopping one (written on status 0 and 2). Statuses are
those of the single entry: 0 ok (all items ran, or the batch stopped on a nonzero result: compare `*done` with `n`),
1 poisoned (checked before any argument), 2 bad arguments, 3 panicked. Status 2 is answered up front, before any item
runs, for a negative `n`, a null or misaligned table with `n > 0`, or overlap of the `ret` table, `done` and the
handle object; an up-front refusal writes `*done = 0` (no item ran) if `done` is itself usable, i.e. non-null,
8-aligned, its end address does not overflow, and its 8 bytes are clear of the handle object and of the `ret` table's
byte range (`ret` up to `ret + 8 * max(n, 0)`, even when that table is itself the bad argument); otherwise `done` is not
written. `*done` is never written for a null handle (status 2), a poisoned handle (status 1) or a panic (status 3). It is
answered per item `i` (with `*done = i`, items 0..i already ran and stay visible) for the
single-entry argument errors of that item (a `bool` other than 0/1, a bad span, overlap with that item's other
buffers, the `ret` table, `done` or the handle object). `n = 0` is status 0 with `*done = 0`. Every call runs inside
`catch_unwind` and `PruvStore::with` and returns a status: 0 ok, 1 poisoned (checked right after the null handle,
before any other argument; nothing ran), 2 bad arguments (null handle or `ret`; negative, oversized, null or
misaligned array; a range whose end address overflows; a `bool` byte other than 0/1; invalid UTF-8; overlapping
ranges where one is written, including `ret` and the handle object; refused, nothing ran), 3 a Rust panic unwound to
the adapter (inside the store it also poisoned the handle). No unwind crosses into C. Checked failures inside
LLVM-compiled code (range, arithmetic, `requires`) and G6 host-callback failures exit the process with 101 and never
return a status. C parameter names are positional (`a0`, `a1`/`a1_len`, ...), so user parameter names cannot collide
with the adapter's own. The `pruv_c` module docs state the caller contract (each `# Safety` section points to it):
`h` came from `pruv_handle_new` and is live, one handle is never used concurrently or reentrantly, and each
(pointer, length) with length > 0 names that many initialized elements in one live allocation that nobody else
touches during the call. Each function has a fixed parameter count; there is no registry, so a double free or a call
after free is undefined behaviour the adapter cannot detect. Symbol names are global; the package prefix keeps the
`pruv_c` symbols of `"c"` programs with different package names apart (the same package name twice still clashes).
LLVM symbol prefix: with a package name (the manifest `name`, `pkg`) every program-specific non-internal global symbol of
the LLVM build also carries `<pkg>__`: the `.ll` exports `<pkg>__pruv_abi_<entry>` and `<pkg>__pruv_abi_<entry>__typed`
(the Rust bindings call them by those names), and the G6 bridge between the `.ll` and the Rust module uses
`<pkg>__pruv_g6_v1_<helper>` (including `drop__<C>`, `len__<C>`, `string_bytes`, `string_take`, `<f>__str`) and
`<pkg>__pruv_g6_bridge_v1`, defined `#[no_mangle]` in the Rust module and declared in the `.ll`. A program without a
package name (no manifest) keeps the unprefixed names (`pruv_abi_*`, `pruv_g6_v1_*`, `pruv_g6_bridge_v1`); it can be the
only such program in a link, and `raw_entries = "c"` always needs a package name. Internal (`internal`/`private`) LLVM
symbols are never prefixed. The shared runtime (`pruv_alloc`, `pruv_panic_*`, `pruv_check_*`, ...) and the allocator
hooks the runtime calls (`pruv_host_*`) are program-independent and unprefixed: link the runtime object once, and let only
one of the linked Rust modules define the `pruv_host_*` hooks (and `pruv_panic_cost` / `pruv_g7_v1_run`, which programs
using `cost` or `par` also define unprefixed). Two programs with different package names then link into one binary.

### Inline policy (generated Rust)
R-7-3 Each generated Rust fn that the code generator emits with an `#[inline]` hint (such as the `F__pruv_body`
functions) gets its final hint from one pass over the finished Rust text; fns emitted without
the hint keep none. The Python and Rust compilers use the same pass, so their output stays byte-identical:
- A fn is recursive if it can reach itself through calls (directly or through other fns). Recursive fns keep
  `#[inline]`.
- The size of a fn is its number of body lines (1 for a one-line fn). Its inclusive size `incl` is its size plus, for
  each call to another fn that gets `#[inline(always)]`, that fn's `incl`. Calls to fns that keep `#[inline]` add
  nothing.
- Call sites are counted textually over the whole output: a fn name followed by `(`, not after `.` and not in its own
  `fn name(` header.
- A non-recursive fn gets `#[inline(always)]` when `incl <= 8`, when it has exactly one call site, or when it has at
  least two call sites and `incl * (call sites - 1) <= 1500` (the extra copies inlining adds). A fn with zero call
  sites in the output (any visibility; in practice a host entry such as `pub fn m11_load`, called only from the
  harness, whose calls the pass does not see) is therefore promoted only by `incl <= 8`. Every other fn keeps `#[inline]` and is left to rustc's heuristics. Why: host entries called from
  harness closures would otherwise be copied into every closure; measured on M11 in `docs/L3_RESTRUCTURE_SPEC.md`
  Appendix P33: `__text` -16%, rustc CPU 4.05-4.08 -> 2.84-2.85 s (about -30%); some cells 3-6% slower than base, recorded as debt D110 and D113.

Measured on the request-budget milestones (`bench/l3/budget/M8_NOTES.md`, "Inline policy"). This policy kept the M8
gains of `#[inline(always)]` on every fn and improved M7, M9, M4 and M11 too. Always-on-every-fn grew rustc time 3-4x
and binaries by up to 38%. Size thresholds and single-call-site rules alone did not reach the M8 gains.

R-7-7 The LLVM backend applies the same rule to its finished IR text: every `define internal ... @F__pruv_body(...) {`
header is a candidate, its size is its number of body lines up to the first line `}`, call sites are the textual
`@F__pruv_body(` tokens outside its own header, and recursion, inclusive size and the `INLINE_SMALL` /
`INLINE_BUDGET` thresholds are as in R-7-3. A promoted body gets `alwaysinline` before ` {`; the rest keep no
attribute and clang's heuristics. Runtime helpers that already carry `alwaysinline` and ABI wrappers are not
candidates; source attributes (section 6f) are not read. Why: clang -O2 kept calls to small Pruv bodies that the
Rust backend's `#[inline(always)]` removes (D-R10-1). The rule does not promote `put_cp` of
`bench/l3/supervisor/btail.pruv`: its inclusive size is 353 lines and it has 6 call sites, 353 * 5 = 1765 > 1500. Tests: [accept](tests/spec/r7_7_llvm_inline.pruv),
[reject](tests/spec/r7_7_llvm_inline_recursive.pruv).

Tests: [accept](tests/spec/r7_3_inline.pruv) (a small fn called once gets `#[inline(always)]`),
[reject](tests/spec/r7_3_inline_recursive.pruv) (a recursive fn keeps `#[inline]`),
[accept](tests/spec/r7_3_inline_entry.pruv) (a large uncalled `pub fn` keeps `#[inline]`, a small one gets
`#[inline(always)]`),
[reject](tests/spec/r7_3_inline_entry_called.pruv) (calling the same large `pub fn` from `main` goes to its `__pruv_body`, so the wrapper keeps `#[inline]` and the body, with two sites, gets always).

A source function attribute (section 6f) takes precedence over the automatic R-7-3 rule. The Rust body and checked
wrapper use the explicit `#[inline]` or `#[inline(always)]` directive, or `#[cold]` followed by `#[inline(never)]`
for a cold fn. This also applies to `F__pruv_chk` and `F__pruv_par_chk` where emitted, and `--proven-entries`
twins retain the directive. Variants without an inline hint (`F__pruv_cost`, `F__pruv_small`) keep none.
The pass never evaluates these fns for promotion. Each call to an explicit `#[inline(always)]` fn adds its
inclusive size just like a call to an automatically promoted fn. Calls to explicit `#[inline]` and `#[cold]`
fns add nothing. A recursive fn without a directive is still never evaluated and adds nothing. A recursive fn with
an explicit `#[inline(always)]` is evaluated, but its inclusive size counts only its calls to non-recursive fns:
calls from it to recursive fns are dropped, so the size stays finite. Both LLVM backends ignore these attributes.

Tests: [accept](tests/spec/r7_3_inline_explicit.pruv) (single-call `#[inline]` and large `#[inline(always)]` fns
retain their directives), [accept](tests/spec/r7_3_cold.pruv) (body and wrapper get `#[cold]` + `#[inline(never)]`,
never `#[inline(always)]`), [reject](tests/spec/r7_3_cold_unknown.pruv) (`#[hot]`),
[reject](tests/spec/r7_3_cold_pub.pruv) (`#[cold] pub fn`), [reject](tests/spec/r7_3_cold_struct.pruv)
(`#[cold] struct`), [reject](tests/spec/r7_3_cold_args.pruv) (`#[cold()]`),
[reject](tests/spec/r7_3_cold_twice.pruv) (`#[cold]` twice), [reject](tests/spec/r7_3_cold_body.pruv) (on a
statement), [reject](tests/spec/r7_3_inline_pub.pruv) (`#[inline(always)] pub fn`),
[reject](tests/spec/r7_3_inline_struct.pruv) (`#[inline] struct`), [reject](tests/spec/r7_3_inline_body.pruv) (on a
statement), [reject](tests/spec/r7_3_inline_unknown.pruv) (`#[inline(x)]`),
[reject](tests/spec/r7_3_inline_never.pruv) (`#[inline(never)]`), [reject](tests/spec/r7_3_inline_twice.pruv)
(`#[inline] #[inline]`), [reject](tests/spec/r7_3_inline_cold.pruv) (`#[cold] #[inline(always)]`). Cold keeps rare
fallback paths out of inlined hot code (M7 measurements in `docs/L3_RESTRUCTURE_SPEC.md` Appendix P13k).

### Stack arrays (generated code)
R-7-1 A local array `let [mut] a: [E] = alloc(N, v);` (`E` is `i64`, refined or not, `u8` or `bool`) with a literal
length `N` lives on the stack (`[i64; N]` in Rust, `alloca` in LLVM) when every use of `a` is `a[i]`, `a[i] = e`,
`len(a)` or a direct `a` / `inout a` argument of a user (not foreign, not `take`) array parameter, the array takes at
most 16384 bytes, and the function's stack arrays so far (in source order) total at most 65536 bytes. A function in
a recursive cycle has none. A larger or escaping array stays on the heap; this is not an error. The type syntax `[i64; N]` itself is not
Pruv (`parse/refinement_syntax`). Tests: [accept](tests/spec/r7_1_stack_array.pruv),
[reject](tests/spec/r7_1_stack_array_big.pruv) (2049 `i64`s stay on the heap). Full rule:
`tests/runtime/p3_s1_stack.pruv`.

### Short `[i64]` and `[u8]` results in a caller buffer (generated Rust)
R-7-2 A function whose top-level `ensures` includes `len(result) <= K` with a literal `K` in `1..=64`, that calls no
foreign function, and that returns one `let mut o: [T] = alloc(...)` (`T` is `i64` or `u8`, the result type) as its only final `return`, is also
emitted as `F__pruv_small(pruv_sb: &mut [T; K], ...) -> usize`. A non-recursive caller that binds the result with an
immutable `let` passes a stack buffer instead of allocating (at most 4096 bytes of such buffers per caller). Nothing
changes under `--trust-callers` or when any condition fails; this is not an error. Full rule:
`docs/SMALL_RESULT_SPEC.md`. Tests: [accept](tests/spec/r7_2_small_result.pruv), [accept `[u8]`](tests/spec/r7_2_small_result_u8.pruv),
[reject](tests/spec/r7_2_small_result_k65.pruv) (K = 65 gets no small form).

### Proof cache
R-7-4 `--proof-cache DIR` (off by default) stores each proved (unsat) solver query under `DIR/<2 hex>/<key>`, where
the key is a hash of the query and the compiler (the Python and Rust compilers never share entries), and answers a
later identical query from the file without calling the solver. Only unsat answers are stored; a failed or unknown
query is always asked again. `--trust-report FILE` reports `"proof_cache": {"hits", "misses", "stores"}`, and the
proof trace marks a reused query `"cached": true`. The std revalidation (section 6k) never uses the cache. Tests:
[accept](tests/spec/r7_4_cache_hit.pruv) (a second identical compile hits),
[reject](tests/spec/r7_4_cache_miss.pruv) (a different query after priming misses). Full rule:
`tests/g4b/check_cache.py`.

Module proof records: `--module-cache DIR` (off by default) records, per module, that all of the module's functions verified
with no error: file `DIR/<2 hex>/<key>` containing `pruv-module-proof 1\nproved\n`. The key hashes the compiler
(source files for `pruvc.py`; for `pruvc-rs` the executable on Linux and a build-time hash of its inputs elsewhere; if that hash cannot be made safely the flag records and
reuses nothing), the Z3 version, `--infer`, the G9 state, and the
content and name resolution of every module the module's proof can reach. A later compile of any root that loads
the module with the same key skips proving it; any change to those inputs gives a new key and a full proof. A
module with an error is never recorded. With the flag, `--trust-report FILE` adds `"flags": {"module_cache": true}`
and `"module_cache": {"hits", "misses", "stores"}`. Full rule: docs/G4B_SPEC.md section 7.8. Test:
`tests/g4b/check_module_cache.py`.

### Native output (`--emit`)
R-7-5 `--emit exe|obj|ll|rs` is a Rust-driver option: `pruvc-rs` builds a native object or executable itself.
`pruvc.py` accepts the option only to report a diagnostic found before code generation; otherwise it stops with a
command-line error ("unrecognized arguments: --emit ..."). Tests: [accept](tests/spec/r7_5_emit_obj.pruv),
[reject](tests/spec/r7_5_emit_exe_entry.pruv).

## 7a. Refinement types (STAGE 1, bundle A)
A refinement is an annotation on a type that the compiler expands into the contracts of section 6; it adds no new
type. `Type` and ABI stay `i64` / `[i64]`, and a program without annotations is compiled byte for byte as before.
```
x: i64 | 0..=100                                   scalar value range
a: [i64 | -1_000_000_000..=1_000_000_000; <=1_000_000]   element range and length bound
a: [i64 | -10..=10]      a: [i64; <=100]           either part alone
```
**Closed value range vs half-open loop range.** In an annotation `L..=U` includes both ends. The `lo..hi` of
`forall/exists/sum/count/max/min` still excludes `hi`. `; <=N` means `len(a) <= N`. Writing `L..U` in an annotation is
`parse/refinement_syntax`.

**Where.** Only the type of a parameter, a return type, and the type of `let` / `let mut`. Struct fields, enum payloads,
`Option<..>` / `Result<..>` contents and any other place give `parse/refinement_position`. A lone `|` anywhere else is the
old `parse/bad_char`.

**Bounds** are signed decimal integer literals (`_` allowed, sign applied first; `-9223372036854775808` is fine). Exponent
notation (`1e9`), other spellings (`0x10`), expressions (a binary operator `* + - / %` after a bound, e.g. `0..=2*3`)
and variables are `parse/refinement_syntax`; so is an array length written without `<=` (`[i64; 3]`, fix: `<=N`). After parsing:
`L > U`, a bound outside i64, or `N` outside `0..=2^40` is `type/refinement_bounds`; `|` on `bool` or a user type is
`type/refinement_base` (reported after the whole file parsed). These checks run before the type checker.

**Expansion** (done by `pruvc/refine.py` at the start of the checker; the synthesized nodes sit at the line/col of the
annotation and are type-checked like user code):
| written | becomes |
|---|---|
| parameter `p: i64\|L..=U` | `requires L <= p && p <= U`, before the user's `requires`, in parameter order |
| parameter `a: [i64\|L..=U; <=N]` | `requires len(a) <= N`, then `requires forall K in 0..len(a): L <= a[K] && a[K] <= U` |
| return `-> T\|R` | the same conditions on `result` as `ensures`, before the user's `ensures` |
| `let x: T\|R = e;` | an `assert` of the condition right after the binding (`verify/refinement_init`) |
| `x = e;` on a refined local | an `assert` right after (`verify/refinement_assign`) |
| `x[i] = e;` on an element-refined array | no assert: the store node carries `elem_refine = [L, U, origin]`, and the verifier proves `L <= v && v <= U` for the already evaluated stored value `v` right after the `index_oob` obligation (`verify/refinement_assign`; one query, no re-evaluation of `i`, no `forall`) |
| a `while` that assigns a refined local alive at the loop head, or passes it as an `inout` argument | that local's conditions are put first in the loop's `invariant` list |
| an `inout` call on a refined local | an `assert` of the local's condition right after the call (`verify/refinement_assign`), proved from the callee's `ensures`; an `inout` parameter's annotation is only an entry `requires` |
`K` is the first of `k`, `k_1`, `k_2`, ... that occurs nowhere in the function, so a user variable named `k` is never
captured. Obligations are also checked on the path where the function returns right after the statement.

**Current behaviour worth knowing.**
* The line/col of an error from a synthesized `requires` / `ensures` / `invariant` / `assert` is the annotation's position
  (the comment position), not the statement that broke it; the one exception is an element store, whose
  `refinement_assign` error sits at the store statement. A store into a refined array checks only the stored value: the
  rest of the array is covered by the elements' own earlier obligations, so an out-of-range index gives just `index_oob`
  and no follow-up error.
* Synthesized `requires` / `ensures` / `invariant` conditions do not yet carry a source phrase in their messages (they read
  like ordinary contract failures); only the synthesized `assert`s and element stores name their origin. A phrase for them
  is planned for stage 1 B.
* The proven store-value fact is not added to the path afterwards (it is already implied by the path).
* Expansion runs once per parsed program (a done-marker on the program); calling the checker twice does not expand twice.
  The marker and `elem_refine` are not part of the source language; `elem_refine` appears in the checked-AST dump only on
  a store into an element-refined array.

**Meaning and run-time policy.** A refinement is a proof obligation, not an enforced type. Proof mode checks every
obligation statically (callers prove parameter refinements as `call_requires`; a return refinement fails as
`ensures`; invariants as `invariant_entry` / `invariant_preserved`). An unrefined `i64` can be passed where a refinement
is expected when the path facts (an `if`, a `requires`) prove it. The parameter conditions are ordinary `requires`: they
are checked at a public entry before the user's `requires`, share its 2^30 iteration budget (length bound first, so a too
long array fails before any element is visited; each visited element costs 1), and are skipped by `--trust-callers`.
Return and local refinements, like `assert`, `ensures` and `invariant`, are erased from the generated code; `--no-verify`
therefore does not check them (it still keeps the entry `requires` checks and the safe indexing/arithmetic).
Do not read an annotation as a run-time guarantee of local variables or results under `--no-verify`.

## 8. Errors
Compiler prints JSON lines: `{"stage": "...", "line": n, "col": n,
"code": "...", "message": "...", "fix": "...", "counterexample": {...}}`. Exit code 1 on any error; a successful
compile prints `{"ok": true}` and exits 0.

R-8-1. The stage is one of `parse`, `type`, `effect`, `verify`, `pkg` (package manifests and the dependency graph,
section 6a), `codegen` (code generation and native emission) and `link` (`--emit exe`; Rust compiler only, since
`--emit exe|obj` is Rust only, section 7). A compiler defect prints one line with stage `internal` and code
`internal_error` and exits 3. No compiler runs rustc, so no diagnostic has the stage `rustc`. Every code a compiler can
print is a row of the table below; `tests/spec_trace.py` fails if a test expects or a compiler prints a code that is
not in it. Tests: [tests/spec/r8_1_ok.pruv](tests/spec/r8_1_ok.pruv),
[tests/spec/r8_1_codegen.pruv](tests/spec/r8_1_codegen.pruv).

### Diagnostic codes

| code | stage | meaning |
|---|---|---|
| `parse/attribute_position` | parse | a recognized attribute not directly before a private `fn` item, or a `#[` that does not start a top-level item |
| `parse/bad_char` | parse | a character that starts no token |
| `parse/bad_int` | parse | a malformed integer literal, such as a trailing `_` (R-1-2) |
| `parse/cost_syntax` | parse | `cost` needs one expression and must be the last header clause |
| `parse/decreases_syntax` | parse | a malformed, empty or repeated `decreases` clause |
| `parse/element_storage_type` | parse | `u8` used outside an array element type |
| `parse/foreign_syntax` | parse | a malformed `trusted rust fn` declaration |
| `parse/generic_syntax` | parse | a malformed type parameter list or type argument list |
| `parse/inout_syntax` | parse | a malformed `inout` parameter or argument |
| `parse/lib_module_conflict` | parse | a module is found both in the program and in a library path |
| `parse/module_alias` | parse | one file is loaded as two modules |
| `parse/module_file` | parse | a module's file is missing, unreadable or not UTF-8 |
| `parse/module_syntax` | parse | a malformed `mod` declaration |
| `parse/parallel_syntax` | parse | a malformed `par { ... }` block |
| `parse/refinement_position` | parse | a refinement `\| L..=U` outside a parameter, return or `let` type |
| `parse/refinement_syntax` | parse | malformed refinement bounds |
| `parse/reserved_name` | parse | a name reserved for the compiler (`pruv_` prefix, `__`, a Rust keyword) |
| `parse/std_import_location` | parse | `mod std;` outside the program root |
| `parse/std_module_conflict` | parse | a local `std.pruv` conflicts with the bundled std module |
| `parse/std_source` | parse | the bundled std module cannot be read |
| `parse/string_character` | parse | a string literal spans lines or holds a control character |
| `parse/string_escape` | parse | an unsupported escape in a string literal |
| `parse/string_unterminated` | parse | a string literal without its closing quote |
| `parse/syntax` | parse | any other syntax error |
| `parse/unknown_attribute` | parse | an attribute other than `#[cold]`, `#[inline]` or `#[inline(always)]` |
| `parse/unsupported_type` | parse | a type the parser knows but this compiler does not implement yet |
| `parse/visibility_syntax` | parse | a misplaced or malformed `pub` |
| `pkg/dependency_cycle` | pkg | the package dependencies form a cycle |
| `pkg/identity_conflict` | pkg | one package is loaded from two places |
| `pkg/manifest_invalid` | pkg | a package manifest is not valid |
| `pkg/mount_conflict` | pkg | a module name is both a local module and a mounted package |
| `pkg/std_in_dependency` | pkg | `mod std;` inside a dependency package |
| `pkg/unused_dependency` | pkg | a declared dependency is not mounted by `mod` in the package root |
| `pkg/version_unsatisfied` | pkg | a package version does not satisfy a requirement |
| `type/arity` | type | a call or constructor with the wrong number of arguments |
| `type/array_alias` | type | a borrowed array or str is stored or returned without `copy` |
| `type/async_unconsumed` | type | a temporary `Loop` is never consumed |
| `type/bad_trigger` | type | a quantifier trigger on an aggregate, not an array read, or not mentioning the binder (R-6-3) |
| `type/call_in_contract` | type | a contract calls a function that cannot be expanded there |
| `type/contract_only` | type | a contract-only operation (`==>`, collection indexing, ...) used in code |
| `type/cost_expr` | type | a `cost` expression uses `spent`, `result` or `old` |
| `type/cost_type` | type | a `cost` expression is not `i64` |
| `type/dup_field` | type | a struct or variant declares a field twice |
| `type/dup_fn` | type | a function is defined twice |
| `type/dup_param` | type | a function declares a parameter twice |
| `type/dup_type` | type | a type is declared twice |
| `type/dup_variant` | type | an enum declares a variant twice |
| `type/duplicate_module` | type | a module is declared twice in one file |
| `type/effect_in_contract` | type | an effectful operation (alloc, store_be, ...) in a contract |
| `type/empty_enum` | type | an enum without variants |
| `type/entry_main` | type | the root `main` is not `pub fn main() -> ()` |
| `type/entry_signature` | type | a host entry has a parameter or result type the host cannot pass |
| `type/eq_type` | type | `==` or `!=` on a type that does not support whole-value equality |
| `type/field_type` | type | a field type that is not allowed (unit, an array in a generic type, ...) |
| `type/field_name` | type | a field name does not start with a lower-case letter |
| `type/foreign_borrow` | type | one value is borrowed and moved in the same call |
| `type/foreign_contract` | type | a catalog or foreign operation used against its contract rules |
| `type/foreign_export` | type | `main` takes or returns a collection |
| `type/foreign_loop_move` | type | a value moved in a loop body is not live again at its end |
| `type/foreign_move` | type | an invalid collection move, missing move, or use after move |
| `type/foreign_owned_type` | type | an owned collection in a position that cannot hold it |
| `type/foreign_path` | type | an invalid Rust path in a `trusted rust fn` |
| `type/foreign_signature` | type | a `trusted rust fn` signature the bridge does not support (R-6d-1) |
| `type/generic_bound` | type | an unknown bound, or a missing `Element` bound |
| `type/generic_main` | type | `main` declares type parameters |
| `type/generic_operation` | type | a field, variant or other concrete operation on an abstract type parameter |
| `type/immutable` | type | an assignment to a binding that is not mutable |
| `type/immutable_string` | type | a byte store into a `str` |
| `type/inout_alias` | type | an `inout` argument also used elsewhere in the same call |
| `type/inout_argument` | type | an `inout` marker where the parameter is not `inout` |
| `type/inout_context` | type | an `inout` call in a position forbidden by section 6g or by a builtin's statement-only rule |
| `type/inout_type` | type | an `inout` parameter of a type that cannot be `inout` |
| `type/int_range` | type | an integer literal outside the i64 range |
| `type/internal` | type | an unknown statement or expression reached the checker |
| `type/invariant_decl` | type | a struct invariant clause that is not bool, uses `old()` or `result`, sits on a generic struct, or a parameter `result` of that struct type |
| `type/match_arity` | type | a match arm binds the wrong number of fields |
| `type/match_arms` | type | match arms that are missing or repeated |
| `type/match_type` | type | a match on a value that is not Option, Result or an enum |
| `type/missing_return` | type | a function may reach its end without returning |
| `type/name_collision` | type | one name is both a type and a function |
| `type/net_unconsumed` | type | a temporary `Socket` or `Listener` is never consumed |
| `type/not_array` | type | indexing or storing into a value that is not an array |
| `type/not_enum` | type | an enum operation on a value that is not an enum |
| `type/not_struct` | type | a field access or field store on a value that is not a struct |
| `type/old_argument` | type | `old` applied to anything but one `inout` parameter |
| `type/old_context` | type | `old` outside `ensures`, invariants and asserts |
| `type/opaque_signature` | type | an `opaque fn` that is not a bool-valued function (R-6o-1) |
| `type/parallel_alias` | type | one `par` branch writes a binding that another reads or writes |
| `type/parallel_argument` | type | a `par` branch argument of a form or type that cannot cross the branch |
| `type/parallel_arity` | type | a `par` group with fewer than two calls |
| `type/parallel_call` | type | a `par` branch that is not a call to a Pruv function |
| `type/parallel_foreign` | type | a `par` branch reaches a trusted or catalog operation |
| `type/parallel_nested` | type | a `par` branch reaches another `par` group |
| `type/pred_in_code` | type | code calls a `pred fn` (R-6-2) |
| `type/pred_signature` | type | a `pred fn` that is not one pure bool contract expression (R-6-2) |
| `type/print_type` | type | `print` of a value that is not i64 or bool |
| `type/private_item` | type | a path names a private item of another module |
| `type/proc_unconsumed` | type | a temporary `Process` is never consumed |
| `type/public_interface` | type | a public item exposes a private type or function |
| `type/recursive_borrow` | type | a value is assigned or passed `inout` while a match on it is active |
| `type/recursive_decreases` | type | a recursive function without `decreases` (or a trusted one with it) |
| `type/recursive_generic_type` | type | a generic type that is recursive |
| `type/recursive_polymorphism` | type | a recursive call that changes the type parameters |
| `type/recursive_type` | type | a recursive type with no finite value, or recursion hidden in another type |
| `type/refinement_base` | type | a refinement on a type it cannot apply to |
| `type/refinement_bounds` | type | refinement bounds outside i64, or an empty range |
| `type/reserved` | type | a function named like a builtin |
| `type/result_error_type` | type | a `Result<T, E>` error type `E` that E1/E2 do not support (section 6c) |
| `type/try_position` | type | a `?` outside the value of a let, a local assignment, a return, an expression statement or an if condition, in a quantifier, in the right operand of `==>`, in the right operand of `&&` / `\|\|` when the statement passes `inout` or the operator has a quantifier or `subview`, moves a name used outside it, mentions one moved outside it or mentions a non-value binding used again (LG15) or a `pred fn` return, after an operand evaluated before it that cannot be bound to a temporary (it has a quantifier or `subview`, moves a name used again, mentions a non-value binding that the same call mentions again other than as a borrowed user-function argument, or is not of a value type other than an owned call result or `move` passed directly to a user function's `take` parameter of the same type, LG14/LG18, or whose arguments are not plain, LG19/LG20; in a statement that passes `inout`, also when it or any `?` operand mentions an inout root or has an effect call, or it passes `inout`, LG18), or with an `inout` argument in its operand while the statement reads a local outside it (section 6c) |
| `type/try_operand` | type | a `?` operand that is not a `Result` (section 6c) |
| `type/try_return` | type | a `?` in a function that does not return a `Result` (section 6c) |
| `type/try_error_type` | type | a `?` whose error type differs from the function's error type (section 6c) |
| `type/result_scope` | type | `result` used outside `ensures` where it is not a declared name (R-1-1) |
| `type/return_type` | type | a `return` that does not match the result type |
| `type/reveal_target` | type | `reveal` of something that is not an opaque function (R-6o-3) |
| `type/shadow` | type | a name that is already defined is bound again |
| `type/size_type` | type | `size` of a value that is not a non-generic struct or enum |
| `type/spent_context` | type | `spent` outside an invariant or assert of a cost function |
| `type/struct_fields` | type | a struct literal with missing, extra or repeated fields |
| `type/type_argument` | type | a type that cannot be a type argument or an array element |
| `type/type_arity` | type | the wrong number of type arguments |
| `type/type_mismatch` | type | an operand or argument of the wrong type |
| `type/type_name` | type | a type name does not start with an upper-case letter |
| `type/type_parameter` | type | a type parameter that is lower-case, repeated or shadows a type |
| `type/unit_let` | type | a `let` of a `()` value |
| `type/unit_param` | type | a parameter of type `()` |
| `type/unknown_field` | type | a field the type does not have |
| `type/unknown_fn` | type | a call to an unknown function |
| `type/unknown_module` | type | a path through a module that is not declared |
| `type/unknown_name` | type | an unknown variable |
| `type/unknown_type` | type | an unknown type name |
| `type/unknown_variant` | type | a variant the enum does not have |
| `type/unused_value` | type | a non-unit value is dropped |
| `type/useless_expr` | type | an expression statement that is not a call |
| `type/variant_name` | type | a variant name does not start with an upper-case letter |
| `type/view_context` | type | `subview(...)` anywhere but a direct non-`inout` array argument of a Pruv call (R-6g-6) |
| `effect/cost_callee` | effect | a cost function calls a function without `cost` |
| `effect/cost_unsupported` | effect | a cost function calls a foreign, catalog or file operation |
| `effect/missing_effect` | effect | a function uses an effect it does not declare |
| `effect/parallel_effect` | effect | a `par` branch reaches a function with an effect |
| `verify/alloc_negative` | verify | an allocation length not proved in `0..=2^40` |
| `verify/assert` | verify | an `assert` not proved |
| `verify/call_requires` | verify | a callee's `requires` not proved at the call |
| `verify/contract_domain` | verify | a partial operation in a contract not guarded (index range, divisor, ...) |
| `verify/cost_bound` | verify | `spent <= cost` not proved |
| `verify/decreases_decrease` | verify | a loop's `decreases` not proved to shrink |
| `verify/decreases_nonneg` | verify | a loop's `decreases` not proved `>= 0` |
| `verify/div` | verify | a divisor not proved non-zero (or `i64::MIN / -1` not excluded) |
| `verify/elem_range` | verify | a `[u8]` store or fill not proved in `0..=255` |
| `verify/ensures` | verify | an `ensures` not proved |
| `verify/index_oob` | verify | an index not proved in range |
| `verify/inference_timeout` | verify | `--infer` hit its time limit for a function |
| `verify/invariant_entry` | verify | a loop invariant not proved before the first iteration |
| `verify/invariant_preserved` | verify | a loop invariant not proved after one iteration |
| `verify/opaque_total` | verify | an `opaque fn` body not proved defined everywhere (R-6o-4) |
| `verify/overflow` | verify | an arithmetic result not proved in the i64 range |
| `verify/recursive_decrease` | verify | a recursive call's measure not proved smaller |
| `verify/recursive_nonneg` | verify | a recursive measure not proved `>= 0` |
| `verify/refinement_assign` | verify | a value stored into a refined local not proved in its range |
| `verify/refinement_init` | verify | a refined local's initial value not proved in its range |
| `verify/shift_range` | verify | a shift count not proved in `0..64` |
| `verify/string_slice_range` | verify | `substr` bounds not proved in `0 <= lo <= hi <= len(s)` |
| `verify/struct_invariant` | verify | a struct invariant not proved for a call argument, a result or an `inout` value at exit |
| `verify/utf8_boundary` | verify | a `substr` cut not proved at a character boundary |
| `verify/utf8_invalid` | verify | bytes passed to `str_from_utf8` not proved valid UTF-8 |
| `verify/variant_domain` | verify | a `variant_field` lookup not guarded by its variant |
| `verify/verify_unknown` | verify | the solver gave up on one obligation (see below) |
| `codegen/foreign_target` | codegen | a program with foreign or file operations sent to native emission without the Rust host build |
| `codegen/llvm_emit` | codegen | LLVM could not emit the object file (Rust compiler) |
| `codegen/llvm_target` | codegen | LLVM has no target machine for the target (Rust compiler) |
| `codegen/parallel_target` | codegen | native `--emit obj/exe` of a program with `par` |
| `codegen/proven_name_clash` | codegen | `--proven-entries` with a user item named like a generated proven-entry item |
| `codegen/specialization_limit` | codegen | too many or too deeply nested generic instantiations |
| `codegen/target_mismatch` | codegen | the object does not match the requested target (Rust compiler) |
| `codegen/unsupported` | codegen | a native output this build cannot produce (Rust compiler) |
| `codegen/unsupported_target` | codegen | a target that is not one of the four public targets (Rust compiler) |
| `codegen/unsupported_type` | codegen | a construct the chosen backend does not implement yet |
| `link/exe_entry` | link | `--emit exe` without `fn main() -> ()` |
| `link/link_failed` | link | the linker failed; the message holds its first error line |
| `link/linker_missing` | link | the linker could not be started |
| `link/target_runtime_missing` | link | no embedded runtime for the target |
| `link/unsupported` | link | linking for the target is not implemented (Windows) |
| `internal/internal_error` | internal | a compiler defect; exit 3 |

G3 added the error codes `generic_syntax`, `type_parameter`, `type_arity`, `type_argument`, `generic_operation`,
`generic_bound`, `generic_main` and `specialization_limit` (section 6c).

`verify_unknown` is not a counterexample: the solver gave up on that one obligation (the message usually ends in
`(timeout)`). Each attempt gets 8 seconds of wall time (a second attempt with aggregate lemmas when aggregates are
used), so a loaded machine can also cause it. The program may be correct; make the proof easier. What helps, in order:
- Keep each loop invariant to one simple fact, and state a fact about one position without a quantifier when you can:
  "`last` is the last negative index before `i`" is `-1 <= last && last < i`, `last == -1 || a[last] < 0` and
  `forall k in last + 1..i: a[k] >= 0`, not `forall s in 0..i: (exists k in s..i: a[k] < 0) ==> s <= last`.
- A quantifier on the left of `==>` (or under `!`) inside another quantifier is the most expensive shape; write such a
  fact once (in `ensures` and the one invariant that carries it) and support it with the simple facts above.
- Add the small facts the step needs as extra invariants (simple bounds between your variables), rather than `assert`s.
- Which shapes time out can differ slightly between the Python and Rust compilers; a proof meant to be kept should
  pass both.

## 9. Example
```
fn sum(a: [i64]) -> i64
  requires len(a) <= 1_000_000
  requires forall k in 0..len(a): a[k] >= -1_000_000_000 && a[k] <= 1_000_000_000
{
  let mut s: i64 = 0;
  let mut i: i64 = 0;
  while i < len(a)
    invariant 0 <= i && i <= len(a)
    invariant s >= -1_000_000_000 * i && s <= 1_000_000_000 * i
    decreases len(a) - i
  {
    s = s + a[i];
    i = i + 1;
  }
  return s;
}
```
