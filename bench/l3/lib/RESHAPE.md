# Rust-shape reset step 3: shared lib modules

Step 3 covers the shared `bench/l3/lib` modules that the entry, req and budget pieces build on. Each module is set
against a hand Rust file next to it (`lib/hand/*.rs`) that does the same job, written the way a Rust programmer writes
it. Rules are the ones in `bench/l3/pkgs/RESHAPE.md`: done means structurally identical to the hand Rust, function by
function; a difference is allowed only when a proof removes or adds a check the hand Rust does not have; a function
that cannot be written in the hand shape because Pruv lacks a feature stays as it is and gets a gap row.
Gap ids refer to `docs/RUST_SHAPE_RESET.md` §4. This file is the step 3 RESHAPE table (step 2 stays in
`bench/l3/pkgs/RESHAPE.md`).

## Status

| Module | Hand baseline | Differences left |
|---|---|---|
| `ent_u16` | `hand/ent_u16.rs` (new in 3.1) | U, F, P-cat, P-put, API rows (`max_alloc`, `index_of`) |
| `ent_path` | `hand/ent_path.rs` (new in 3.1) | U, J, L, R, I, BL, P-guard rows, API rows (`resolve_need*`) |
| `ent_wire` | `hand/ent_wire.rs` (new in 3.2) | O (`sections` / `pool_start` return `-1`) |
| `ent_packet` | `hand/ent_packet.rs` (new in 3.2) | K, E, O, S, L, P-start (V, P-hi closed in 3.3) |
| `ent_value` | `hand/ent_value.rs` (new in 3.3) | U, O, VE, PB, CMP, API rows (`(w, start)` wrappers) |
| `ent_state` | `hand/ent_state.rs` (new in 3.4) | OA, ENT, CK, OR, CX partly closed, DR; AD closed in 3.7 |
| `ent_dispatch` | `hand/ent_dispatch.rs` (new in 3.4) | OA, O (clip code 0); LR closed in 3.5, AD closed in 3.7 |
| `ent_bound` | `hand/ent_bound.rs` (new in 3.5) | SA, HK, PB, U, J, O; AD closed in 3.7 |
| `ent_canon` | `hand/ent_canon.rs` (new in 3.5) | RA, O, OA, OR, TM; AD closed in 3.6 / 3.7 |
| `ent_sources` | `hand/ent_sources.rs` (new in 3.6) | LB, MP, CK, RA, OA, OR, TM, U, PB |
| `entry_api` | `hand/entry_api.rs` (new in 3.7) | OR, TM, VB, CK, O, PB, VE |
| `ent_accounts` | `hand/ent_accounts.rs` (new in 3.8) | MP, O, U, PB, VE |
| `entry/root` | `hand/entry_root.rs` (new in 3.8) | none (the shared hand crate root moved here from `entry_api.rs`) |

## Step 3.1: `ent_u16` and `ent_path`

Neither module had a hand baseline with the same meaning (`entry/hand.rs` is the serde_json R02 workload, with no
per-operation counterpart), so both hand files are new. They are checked with `rustc --edition 2021 --test` (the
command is in each file's header); `hand/ent_path.rs` carries Node v26.4.0 `path.posix` cases.

### Gap U (not changed, every function)

The public `[i64]` unit arrays are consumer API: about 1,600 call sites in `ent_*`, `req/*` and `budget/*` pass and
index them as `[i64]` with `i64` indexes. The hand uses `&[u16]` / `Vec<u16>` and `usize`. Changing the element type
is an API change across all consumers, so every function below keeps `[i64]` and `i64`; this is one open gap row (U)
for both modules, not a per-function difference.

### `ent_u16` (`lib/ent_u16.pruv`, hand `lib/hand/ent_u16.rs`)

| Pruv | Hand | Before 3.1 | After 3.1 | Same as hand |
|---|---|---|---|---|
| `rd` | `a.get(i).copied().unwrap_or(0)` | bounds test + index | unchanged | = |
| `wr` | `if let Some(x) = a.get_mut(i)` | bounds test + store | unchanged | = |
| `max_alloc` | `const MAX_ALLOC` | function | unchanged (consumer API) | API |
| `zeros` | `vec![0; k]` | clamp + `alloc(k, 0)` | unchanged | = |
| `empty` | `Vec::new()` | `alloc(0, 0)` | unchanged | = |
| `slice` | `a[lo..hi].to_vec()` | `alloc(m, 0)` + `copy_range` (zero fill, then copy) | `copy(subview(a, lo, hi))` | = |
| `cat` | `with_capacity(na + nb)` + 2 x `extend_from_slice` | `alloc(na + nb, 0)` + 2 x `copy_range` | `alloc(0, 0)` + `arr_reserve(na + nb)` + 2 x `arr_append` | = except P-cat |
| `lit` | `s.bytes().map(u16::from).collect()` | `alloc(n, 0)` + store loop | `arr_reserve(n)` + `arr_push` loop | = |
| `one` | `vec![u]` | `alloc(1, 0)` + store | `alloc(1, u)` | = |
| `eq` | `a == b` | length test + element loop | length test + `eq_range(a, 0, b, 0, n)` | = |
| `eq_lit` | length test + zip compare | length test + loop | unchanged | = |
| `index_of` | `a[from..].iter().position(..)` | clamp + loop, `-1` for none | unchanged | API (`-1` for `None`, consumer API) |
| `is_trim_unit` | `matches!(..)` | `if` chain | unchanged (`q_mutants` M01 line kept) | = |
| `trim` | two scans + `a[s..e].to_vec()` | two scans + `slice` (clamps again) | two scans + `copy(subview(a, s, e))` | = |
| `clamp` | `x.clamp(0, hi)` | three branches | gains `ensures x < 0 ==> result == 0` | = (`requires 0 <= hi` replaces the hand panic) |
| `put_range` | clamp m, `room`, `k`, `copy_from_slice` + `fill(0)` | fast `copy_range` when fully in range, else a per-unit `wr(rd(..))` loop | `room = clamp(len(o) - at, mm)`, `k = clamp(len(src) - s, room)`, `copy_range` of k, zero loop for `room - k` | = except F, P-put |
| `put_all` | `put_range(o, at, src, 0, len)` | same | unchanged | = |

P-cat (proof-only row): `cat` keeps `if na > 1073741823 || nb > 1073741823 { return empty(); }`. The verifier needs
it to prove the `arr_reserve` / `arr_append` length preconditions (`len + n <= 2^40`); hand Rust panics on capacity
overflow instead. Removing it was tried in 3.1 and fails verification. `lit` needed no guard (its old `n > 2^40` test
is gone).

P-put (proof-only): `put_range` / `put_all` require `at, s <= 2^32` so `at + mm` stays in range; the hand has
`usize` arithmetic with `saturating_sub`. All callers pass offsets inside one packet.

F (gap, open): the zero tail of `put_range` is an index loop; the hand writes `o[at + k..at + room].fill(0)`. Pruv has
no range fill for `[i64]`.

### `ent_path` (`lib/ent_path.pruv`, hand `lib/hand/ent_path.rs`)

| Pruv | Hand | Before 3.1 | After 3.1 | Same as hand |
|---|---|---|---|---|
| `is_abs` | `p.first() == Some(&SLASH)` | `rd(p, 0) == 47 && len(p) > 0` | `len(p) > 0 && p[0] == 47` | = |
| `copy_into` | none | per-unit `wr(rd(..))` copy | removed | = |
| `last_slash_idx` | `res.iter().rposition(..)` (inline) | `rd` loop with a length argument | direct-index reverse loop, no length argument; `-1` for `None` | I |
| `normalize_string` | `Vec::with_capacity(len)`, `for i in 0..=len` with `break` / `continue`, `truncate` / `clear`, `push`, `extend_from_slice` | `alloc(cap, 0)` buffer with a manual used-length `u`, `rd` / `wr` per unit, `slice(res, 0, u)` copy at the end | `arr_reserve(n)`, `for i in 0..n + 1` with `break` / `continue`, `truncate`, `arr_push(46, 46)`, `arr_append(p, last_slash + 1, ..)`, returns the buffer itself | = except P-ns |
| `normalize` | `with_capacity(r.len() + 2)`, push / extend / push | `cat(r, empty())`, then `cat(x, lit("/"))`, then `cat(lit("/"), x)`: up to five arrays | one buffer: `arr_reserve(len(r) + 2)`, `arr_push(47)`, `arr_append(r)`, `arr_push(47)` | = except P-norm, L |
| `push_part` | `push_part(&mut Vec, &[u16])` | `pjoin` = `cat(cat(a, lit("/")), b)` per argument | `push_part(inout joined, part)` | = |
| `join2` / `join3` / `join4` / `join_segs` | `join(&[&[u16]])`, one buffer with capacity sum + count | chained `pjoin`: two new arrays per argument | one buffer: `arr_reserve(sum + count)`, `push_part` per argument, `normalize` | J, P-join |
| `dirname` | reverse `for` with `break`, `p[..e].to_vec()` | `rd` loop with a stop flag, `slice` | `while` with `break`, `p[i]`, `copy(subview(p, 0, endp))` | = except L, R |
| `basename` | reverse `for` with `break`, `p[start..e].to_vec()` | `rd` loop with a stop flag, `slice` | `while` with `break`, `p[i]`, `copy(subview(p, start, endp))` | = except R |
| `resolve_needs_first_cwd` | none (`!is_abs(p)` at the call) | `!is_abs(p)` | unchanged (consumer API) | API |
| `is_fast` | `p.is_empty() \|\| p == [DOT]` | `rd(p, 0)` | `p[0]` | = |
| `resolve_need` | `(p, ncwd, cwd1)` | `(p, ncwd, cwd1, cwd2)` | unchanged (`cwd2` unused, consumer API) | API |
| `used_cwd` (new) | `let c = if ncwd >= 2 { cwd2 } else { cwd1 };` | none (`cat` copies of the chosen cwd) | `fn used_cwd(..) -> ref [i64] from(cwd1, cwd2)`; the generated Rust is `fn used_cwd<'a>(..) -> &'a [i64]` | = (helper, see BL) |
| `resolve_finish` | one reserved `resolved` buffer, `normalize_string`, `with_capacity(r.len() + 1)` for the absolute result | `cat(p, lit("/"))`, `cat(cat(c, lit("/")), tail)`, `cat(lit("/"), r)`: about seven arrays | one reserved buffer (`arr_reserve` + `arr_append` + `arr_push`), `copy(cwd1)` fast path, `normalize_string`, one reserved output | = except P-rf |

Pruv-only and open rows in `ent_path`:

- **P-ns** (proof-only): `normalize_string` returns early when `len(p) > 1073741823` and carries `n <= 1073741823` in
  the loop invariant, so `arr_push` / `arr_append` preconditions are provable. Removing it was tried in 3.1 and fails
  (`cannot prove precondition of arr_push`, lines 84-90). Hand Rust panics on capacity overflow instead.
- **P-norm** (proof-only): `normalize` returns `r` when `len(r) > 1073741823` before `arr_reserve(len(r) + 2)`. Removing
  it fails (`cannot prove precondition of arr_reserve`, line 117).
- **P-join**, **P-rf** (proof-only): the same length guards on each argument of `join2..4` / `join_segs` and on
  `p`, `cwd1`, `cwd2` and `r` in `resolve_finish`.
- **J** (gap, open): the hand takes `parts: &[&[u16]]`, a slice of borrowed slices. Pruv has no array of borrowed
  `[i64]` views (an owned `[[i64]]` argument would copy every part), so the fixed-arity `join2`, `join3`, `join4`
  and `join_segs` stay as the consumer API. Each now builds the same single buffer the hand `join` builds; only the
  argument form differs.
- **L** (literal form): `normalize` and `dirname` build short results with `one(46)`, `one(47)` and `lit("./")` /
  `lit("//")`; the hand writes `vec![DOT, SLASH]`. Pruv has no `[i64]` array literal for owned arrays; `lit` is one
  reserve plus pushes, the same allocation count.
- **R** (loop form): `dirname` and `basename` use a reverse `while` with `break` where the hand uses
  `for i in (a..b).rev()`. Pruv `for` ranges only count up.
- **I** (gap, open): `last_slash_idx` is a helper with `-1` for "none" where the hand writes `rposition` inline with
  `Option`; Pruv has no iterator adapters or `Option<i64>` result here.
- **BL** (`used_cwd`): `let mut c: ref [i64]` assigned in two branches is rejected (`borrow_signature`), so the hand's
  `let c = if .. { cwd2 } else { cwd1 };` is one helper returning `ref [i64] from(cwd1, cwd2)`. The generated Rust is
  the hand expression behind a function call.

### Consumers

The public API of both modules is unchanged (names, argument lists, results), so no consumer source changed:
`entry/root.pruv`, `entry/entry_api.pruv`, `req/*.pruv`, the other `lib/ent_*.pruv` modules and `budget/*.pruv`.
`entry/q_mutants.py` and `req/q_mutants.py` targets are intact; only M01 touches `ent_u16` (`is_trim_unit`, unchanged).
Consumer-side `lit(..)` allocations at join call sites (for example `join3(home, lit(".config"), ..)`) belong to the
later step 3 modules that own those call sites (`ent_sources`, `ent_canon`, `ent_accounts`).

### Consumer tests (step 3.1)

All runs use `PRUVC_RS=/Users/spica4345/Projects/pruv/pruvc-rs/target/release/pruvc`; py pairs not run (frozen).

| Test | Pairs | Result |
|---|---|---|
| `entry/test_impl.py` | rs/rust, rs/llvm | 296 variants x 2 = 592 runs, unexplained 0 |
| `entry/judge_test.py` | rs | failures 0 |
| `entry/q.py --count 400` | rs/rust, rs/llvm | 399 exact, 1 excluded-domain, unexplained 0 |
| `req/test_impl.py` | rs/rust, rs/llvm | 160 variants x 2 = 320 runs, unexplained 0 |
| `req/q.py` (q_gen, 2000 cases) | rs/rust, rs/llvm | 2000/2000 exact on each pair |
| `budget/m12_test/run_m12.py --pairs rs` | rs/rust | Q7P 300 exact, L 10, crafted 8, lifecycle 7, fail 0 |
| `budget/m11_test/run_m11.py --pairs rs` | rs/rust | Q6 exact 213, prefix-ok 87 (same as `M11_NOTES.md`), fail 0 |
| `entry/integrate.py` (after qwen 26/26 + codex 136/136 on both pairs, opencode 199/199 and grok `--no-ts` 254/254 on rs/rust) | rs/rust | 10 integration cases, 18 checks PASS, 2 integration limits (I01/I02 claude, the packet-only API; known) |

### Independent structure comparison

Codex gpt-6.2-astra (high), research mode, read the two Pruv modules and the two hand files only (not generated code).
It reported, apart from the accepted rows U, G (proof guards), J, L and R: **no extra intermediate arrays, no extra
passes over the data, no missing early return and no input with a different path result.** Its findings and what was
done with each:

| Finding | Action |
|---|---|
| `trim` passes `s, e` through `slice`, which clamps and tests the empty range again | fixed: `copy(subview(a, s, e))` |
| `put_range` zero tail is an index loop, not `fill(0)`; `at, s <= 2^32` contract | rows F and P-put |
| `put_all` `at <= 2^32` contract | row P-put |
| `clamp` has `requires 0 <= hi` where the hand panics | proof contract, no change |
| `slice` handles a negative `e` by the `hi <= lo` return, the hand clamps it with `max(0)` | same result and same copy count; no change |
| `eq` writes the length test then `eq_range`; the hand writes `a == b` (which does the same) | same work; no change |
| `max_alloc` is a function, the hand a `const`; `index_of` returns `-1`, the hand `Option` | API rows (consumer API) |
| `is_trim_unit` is an `if` chain, the hand one `matches!` | kept: the `q_mutants` M01 target line; same comparisons |
| `last_slash_idx` helper with `-1` vs inline `rposition` | row I |
| `normalize_string` pushes `46, 46` where the hand extends `&[DOT, DOT]`; second `rposition` miss folded into `-1` | same work; follows row I |
| `resolve_needs_first_cwd` exists, `resolve_need` has an unused `cwd2` | API rows (consumer API) |
| `used_cwd` helper vs inline `if` | row BL |
| `resolve_finish` sets `absolute = true` first and overwrites it in the relative branch | Pruv needs an initialised `let mut`; no extra work |

After the `trim` fix the entry root verifies and `entry/test_impl.py` (592 runs) and `req/test_impl.py` (320 runs)
were rerun: unexplained 0 on both.

### Rows still open after step 3.1

- U: `[i64]` units and `i64` indexes in both modules (consumer API, about 1,600 call sites).
- J: `join(&[&[u16]])` needs an array of borrowed `[i64]` views; fixed-arity joins stay.
- R: reverse `for` ranges (`dirname`, `basename`).
- L: owned `[i64]` array literals (`one` / `lit` for two-unit results).
- F: range fill for `[i64]` (`put_range` zero tail).
- I: `rposition`-style search returning an option (`last_slash_idx`, `index_of` returns `-1`).
- API rows kept for consumers: `max_alloc`, `index_of` result, `resolve_needs_first_cwd`, `resolve_need(.., cwd2)`.
- Proof-only guards P-cat, P-ns, P-norm, P-join, P-rf stay by rule (they replace the hand's capacity-overflow panic).

## Step 3.2: `ent_wire` and `ent_packet`

Both hand files are new (`lib/hand/ent_wire.rs`, `lib/hand/ent_packet.rs`; `ent_packet.rs` includes `ent_wire.rs` as a
module). They are checked with `rustc --edition 2021 --test` (command in each header): 2 and 3 tests pass, no warnings.

The packet words are the external wire format (host.rs is the layout authority), so the hand keeps them as `&[i64]` in
and `Vec<i64>` out; gap U does not apply to the packet itself. What changed is the inside: the hand parses at the
boundary once (one section-start table, one view of the pool) and never builds a second flat `[i64]` packet for its
own book-keeping. The Pruv now does the same. The old `meta` array (`2 * nsec + 3` words: high-water, claimed flags,
section offsets and a bad flag in one `[i64]`) is gone; it is the `Check` struct below.

### `ent_wire` (`lib/ent_wire.pruv`, hand `lib/hand/ent_wire.rs`)

| Pruv | Hand | Before 3.2 | After 3.2 | Same as hand |
|---|---|---|---|---|
| `byte` (was `byte_at`) | `byte(pool, k)` | `rd` + word range test per byte, then `ent_value::pool_byte` (clamps `p` and `k` again) | one shift and mask on the pool view (words were range checked once) | = |
| `at` (new) | the `at` closure (`-1` past the stream) | inside `byte_at` | helper | = |
| `sections` (was `shape_ok`) | `sections(w) -> Option<usize>` | `bool`; `wire_in` then re-read `n` and `ns` and tested them again | returns nsec or `-1`, `wire_in` uses it | O |
| `pool_start` (new) | `pool_bounds(w) -> Option<(usize, usize)>` | inside `with_pool_len` | returns the start or `-1`; the length is `w[7]` (the `ensures` carries its range) | O |
| `poison` (`with_pool_len` removed) | `with_capacity(m)` + `extend` + `push(-1)` + `extend` | `alloc(m, 0)` (zero fill) + two `put_range` + a write of `-1` | `arr_reserve(m)` + `arr_append` + `arr_push(-1)` + `arr_append`; copy fallback unchanged | = |
| `wire_in` | `sections` + `w[4]` + `pool_ok` | `shape_ok`, then the header tests again | `sections`, `w[4]`, `pool_ok` | = |
| `pool_ok` | `w.get(p0..p0 + l0)` then `pool_valid` | the whole WTF-8 walk over `w` with a `p0` offset in every read | bounds test + `pool_valid(subview(w, p0, p0 + l0))` | = |
| `pool_valid` (new) | `pool_valid(&[i64])` | (inside `pool_ok`) | header tests, exact length (`q_mutants` M17 line kept), `words_ok`, form loop on the view | = |
| `words_ok` (was `ascii_pool_ok`) | one `iter().all(range && !(ascii && high bits))` | ASCII path only; other words were range checked per byte read | one loop over `pool[2..]`: range test, and the high-bit test when ASCII | = |
| separator test | inline `!hi3 \|\| !(k + 3 < nb && at(k + 1) == 237 && ..)` (short-circuit) | `ent_value::starts_low(w, p0, k + 1, nb - k - 1)` | `hi3` and the room tested first, then the two bytes read | = |
| `padding_ok` | `(nb..end).all(byte == 0)` | `byte_at(.., end)` | `byte(pool, k)` | = |
| `same_packet` | `a == b && wire_in(b)` | length + `eq_range` | unchanged | = |

`ent_wire` no longer calls `ent_value` (`pool_byte`, `starts_low`): the wire check reads its own pool view.

### `ent_packet` (`lib/ent_packet.pruv`, hand `lib/hand/ent_packet.rs`)

| Pruv | Hand | Before 3.2 | After 3.2 | Same as hand |
|---|---|---|---|---|
| `m_*` masks | `pub const ANY: i64 = ..` | nullary functions | unchanged; unused `max_words`, `pos_cap`, `m_proto`, `rd` removed | K |
| `nsec`, `sec_len`, `in_mask`, `zero_run`, `put_vs` | same | | unchanged | = |
| `sec_start` | iterator sum over the table | loop | unchanged | = except S |
| `section_starts`, `start_in` (were `new_meta`, `new_meta_at`, `sec_start_m`, `meta_start`) | `section_starts(w, head) -> Vec<usize>`, `start_in(&[usize], i)` | offsets stored at `meta[nsec + 1 + j]` of a `zeros(2 * nsec + 3)` array | `arr_reserve(ns + 1)` + `arr_push` loop; `start_in` clamps the index into the table | = except P-start |
| `Check` struct, `checker` (were `meta`, `setbad`, `is_bad`, `pool_high_water`) | `struct Check<'a> { w: &'a [i64], starts: Vec<usize>, claimed: Vec<bool>, pool_hi: i64, bad: bool }` | flags and the high-water in `[i64]` meta words | `pub struct Check { w: ref [i64], starts: [i64], claimed: [bool], pool_hi: i64, bad: bool }`, `checker(w) -> Check from(w)`; the generated Rust is `struct Check<'a>` with `w: &'a [i64]` | = |
| `vs` (was `vs_ok` + `vs`) | `Check::slot`, one `match tag` | `vs_ok` returned 0..3, then `vs` read `a`, `b` again and wrote meta | one function on `inout c: Check`, `if` chain over the tag, sets `pool_hi` / `claimed[a]` / `bad` directly | = except P-hi |
| `finish` (was `is_bad` + `all_claimed` + `pool_high_water`) | `finish(&self) -> Option<i64>` | three calls over meta | one function | O |
| `rows1`, `ets`, `session`, `err` (were `*_check(w, p, inout meta)`) | methods on `Check` | | `(inout c: Check, p)`; `zero3` folded into `zero_run(.., 3)` | = |
| `input_check` | `match` on `(op, rl)` | per-op `if rl != n { return -1; }` | `op == k && rl == n` branches, `finish(c)` | O |
| `comp_check` | one `failed` branch for the error record | the error-record branch written out in five request codes | one `failed` branch, then per-code checks | O |
| `env_check`, `env_check_wire`, `env_check_at` | `Result<(), Reject>` | 0 / 1 / 2 | header tests in one condition; codes kept | E, V |
| `boundary` | `vec![1, 3, id, .., 5, c]` | `zeros(15)` + 13 writes + `pool_words(pool_new())` (an allocation for the constant 2) | `arr_reserve(15)` + one `arr_push` of the 15 words | = except L |
| `mk_packet` | `with_capacity(total)` + header + `extend_from_slice` per part | `zeros(total)` (zero fill) + `put_all` chain with clamps | `arr_reserve(total)` + `arr_push` header + `arr_append` per part (`0, 0` for an empty pool) | = |
| `empty_state` | `Vec::new()` | `ent_u16::empty()` | `alloc(0, 0)` | = |

Rows:

- **K** (gap, open): no module constants. `const` exists only in a single-file program (`parse/const_module`, parity
  row P7-3), so the masks stay nullary functions with an `ensures`.
- **E** (API row): the envelope result stays `0 / 1 / 2` where the hand returns `Result<(), Reject>`; 13 consumer
  sites (`env_check` 8, `env_check_wire` 5) map the code to a boundary packet. They move with their owners
  (`entry_api`, step 3.7; `req/*`).
- **O** (API row, same family as I): `-1` for `None` in `sections`, `pool_start`, `finish`, `input_check`,
  `comp_check` (and `ent_accounts::io_check`).
- **S** (consumer row): `sec_start` walks the section table on every call (56 call sites in `entry_api`, `ent_*` and
  `req/*`). The hand of a consumer that reads many sections builds `section_starts` once; each step that owns those
  call sites (3.3 onward) does that, as `ent_bound` already does.
- **V** (step 3.3 boundary): `env_check_at` calls `ent_value::pkt_size` and `vs` calls `ent_value::span_ok`; the hand
  has local copies (`logical_size`, `span_ok`). `ent_value` still owns `pool_byte`, `span_ok`, `starts_low`
  (`req/req_list` and `ent_value` itself use them). 3.3 must: give `span_ok` an `ensures` (ok implies
  `0 <= a`, `a + b <= nb <= 2^30`), which removes P-hi; replace the per-call clamped pool readers with one pool view
  like `ent_wire::pool_valid`; keep `pkt_size` / `logical_pool_words` on `ent_packet::sec_len` / `sec_start` or move
  them to `section_starts`.
- **L** (literal form): `boundary` pushes its 15 words where the hand writes `vec![..]`; one reserve, no zero fill.
- **P-hi** (proof-only): `vs` adds `clamp(a, 2^32) + clamp(b, 2^32)` for the high-water. `a + b` fails
  verification (`cannot prove no i64 overflow`, `span_ok` has no `ensures`); see V.
- **P-start** (proof-only): `start_in` clamps the stored start into `0..2^25` for its `ensures`; the verifier has no
  per-element fact about an array built by `section_starts`. Removing it fails (`cannot prove postcondition`).
- `env_check_wire` builds `poison(w)` to classify a wire rejection (the same in the hand, kept for the existing error
  classification); `entry_api` (3.7) can classify without the copy.

### Consumers

- `lib/ent_accounts.pruv` `io_check`: `new_meta` + `vs(w, .., inout meta)` + `is_bad` / `all_claimed` /
  `pool_high_water` became `checker(w)` + `vs(inout c, ..)` + `finish(c)`; the record start is `start_in(c.starts, 1)`.
- `lib/ent_bound.pruv`: `new_meta` / `new_meta_at` + `meta_start(om, ns, i)` became `section_starts(w, head)` +
  `start_in(starts, i)`; `ses_pairs` lost its `ns` argument.
- No other source changed: `entry/root.pruv`, `entry_api.pruv`, `req/req_packet.pruv` use `wire_in`, `poison`,
  `pool_ok`, `same_packet`, `env_check*`, `input_check`, `comp_check`, `boundary`, `mk_packet`, `put_vs`, `sec_*`,
  `nsec`, `m_any`, `m_str` with unchanged signatures. `q_mutants.py` targets intact (M17 is the `ent_wire` exact-length
  line; M13, M15, M16 in `ent_bound` untouched).
- Budget m11 / m12 do not use these modules (grep); their runners only switched to rs-only by default (`--vs-py` adds
  the frozen py pair), like commit 655b6219.

### Codex structure comparison

One read-only comparison (Codex gpt-6.2-astra high) of both modules against the hands and HEAD. It found no difference
in acceptance, rejection class or returned packet words at the packet entry points on 64-bit. Its findings and what
was done:

| Finding | Done |
|---|---|
| hand `pool_valid` panics on a 0- or 1-word slice; the Pruv returns `false` | hand: the `l0 < 2 \|\| l0 > MAX_WORDS` test added (same as the Pruv) |
| hand ran the word range and the ASCII high-bit test as two passes, the Pruv as one | hand: one `all` pass, the same as `words_ok` |
| at the `0xFF` separator the Pruv read the next two bytes before testing `hi3` and the room | Pruv: `hi3` and `k + 3 < nb` first, then the two reads (hand order) |
| `session`, `err` are methods `pub` in the hand, private in the Pruv | hand: made private (only `Check` uses them) |
| `Reject` doc named the wrong codes | hand: doc says `env_check` 1 / 2 (boundary 0 / 1) |
| `start_in` differs off a `section_starts` table (empty table, a value above `2^25`) | kept: equal on every table `section_starts` builds; the value clamp is P-start |
| `zero_run` counts at most 10^6 words in the Pruv (also at HEAD) | kept: record calls pass 3, 9 or 18 |
| a 32-bit target narrows `i64` lengths to `usize` before the range test in the hand | kept: the bench target is 64-bit; recorded here |
| `-1` for `None` (`sections`, `pool_start`), the start table `[i64]` vs `Vec<usize>`, `claimed` sized by `nsec(w)` | rows O and U (index table); no change |
| `input_check` writes the 4 / 5 slot tests where the hand loops a mask array | row L (no array literal); same order and count |

### Tests (after the last code change)

`bench/l3/lib/hand`: `rustc --edition 2021 --test` on `ent_wire.rs` (2 tests) and `ent_packet.rs` (3 tests), pass.

| Test | Pairs | Result |
|---|---|---|
| `entry/test_impl.py` | rs/rust, rs/llvm | 296 variants x 2 = 592 runs, unexplained 0 |
| `entry/judge_test.py` | rs | failures 0 |
| `entry/q.py --count 400` | rs/rust, rs/llvm | 399 exact, 1 excluded-domain, unexplained 0 |
| `req/test_impl.py` | rs/rust, rs/llvm | 160 variants x 2 = 320 runs, unexplained 0 |
| `req/q.py` (q_gen, 2000 cases) | rs/rust, rs/llvm | 2000/2000 equal on each pair |
| `entry/integrate.py` (after qwen 26/26 + codex 136/136 on both pairs, opencode 199/199 and grok `--no-ts` 254/254 on rs/rust) | rs/rust | 10 integration cases, 18 checks PASS, 2 integration limits (I01/I02 claude, the packet-only API; known, exit 2 as in 3.1) |

### Rows open after 3.2

- U (unit arrays, from 3.1), K (module constants, P7-3), L (array literal): language or API gaps.
- E, O: API rows, owned by the consumer steps (`entry_api` 3.7, `req/*`).
- S: consumer rows, closed by each step that owns the `sec_start` call sites.
- V, P-hi: closed by 3.3 (`span_ok` `ensures`, one pool view in `ent_value`).
- P-start: proof-only, open until the verifier keeps element facts for an array built by a push loop.

## Step 3.3: `ent_value` (`ent_state` deferred)

`ent_value` had no hand counterpart, so `hand/ent_value.rs` is new (`rustc --edition 2021 --test`, 4 tests: round
trip and units, the lone-surrogate separator and `span_ok`, value semantics, UTF-16 order). The hand parses a slot once
into `enum Value` and reads the pool through one borrowed view `Pool<'a>(&'a [i64])` (`Pool::at(w, s)` is `&w[s..]`),
bytes through `Pool::byte`. The Pruv now has the same pool shape: `pool_at(w, s) -> ref [i64] from(w)` (generated Rust
`fn pool_at<'a>(w: &'a [i64], s: i64) -> &'a [i64]`), `nbytes(view)`, `byte(view, k)`, and every reader takes the view.

### `ent_value` pool readers (`lib/ent_value.pruv`, hand `lib/hand/ent_value.rs`)

| Pruv | Hand | Before 3.3 | After 3.3 | Same as hand |
|---|---|---|---|---|
| `pool_at` (new, `pub`) | `Pool::at(w, s)` | none: every reader took `(w, start)` | `subview(w, clamp(s, len(w)), len(w))`, a borrowed view | = |
| `nbytes`, `byte` (new, private) | `Pool::nbytes`, `Pool::byte` | `pool_byte(w, pool0, k0)` clamped `pool0` and `k0` on every byte, `pool_bytes(w, pool0)` | `byte` reads `rd(view, 2 + k/7)` with a `requires` on `k`; no clamp per byte | = |
| `pool_bytes(w, start)` | `Pool::at(w, s).nbytes()` | | `nbytes(pool_at(w, start))`; kept for its 25 consumer sites | API row |
| `span_ok(w, pool, off, n)` | `span_ok(Pool, off, n)` | no `ensures`; `pool_byte` per byte | one view; `ensures result ==> 0 <= off && 0 <= n && off + n <= 2^30 - 1` | = (closes V, P-hi) |
| `form_code`, `units_in`, `prefix_in`, `decode_into`, `decode_span` | `form_code`, `units_in`, `prefix_in`, `units` / `decode_into`, `decode_span` | `(w, start, ..)` and `pool_byte` | take the view; positions clamped once at entry | = except U |
| `span_units`, `prefix_bytes` | | the readers themselves | thin `pub` wrappers `units_in(pool_at(w, s), ..)` / `prefix_in(..)` for consumers | API row |
| `starts_low`, `ends_high` | inside `PoolBuf::append_span` / `ends_high` | `(w, start, ..)` | take the view (`req_list` passes `pool_at(crec, ps)`) | = |
| `append_span`, `append_units` | `PoolBuf::append_span`, `PoolBuf::append_units` | `pool_byte` per byte, `ends_high(o, dest, ..)` | source read through one view `sv`, destination through one view `dv = pool_at(o, dest)` (`nbytes(dv)`, `ends_high(dv, nb)`) | = except PB |
| `pool_base`, `pool_cat` | `PoolBuf::join`, `PoolBuf::cat` | `pool_bytes(a, 0)` | `nbytes(a)`, view readers | = except PB |
| `str_len`, `str_units`, `str_eq_arr`, `str_eq_lit`, `str_eq` | `units_in`, `decode_span`, `str_eq_lit`, `str_eq` over a `Span` | `(w, sec_start(w, 0))` passed down to per-byte readers | one `pool_at(w, sec_start(w, 0))` per call | = except VE |
| `pstr_len` (`pstr_len_at` removed), `pstr_chunk_bytes`, `pstr_part`, `pstr_units` | the same readers on a `Span` | `(pool, 0, ..)` | `units_in` / `prefix_in` / `decode_span` on the pool | = except VE |
| `pstr_eq_lit` (`pstr_eq_lit_at` merged), `pstr_eq` (`pstr_eq_at` merged), `span_eq` | `str_eq_lit`, `str_eq` | a `_at(pool, start, ..)` core plus a wrapper passing 0 | one function each, `byte(view, ..)` | = except VE |
| `pstr_find_byte`, `title_cut`, `prefix_pool_units`, `key_put_at`, `pool_copy_into` | iterator `position` / `prefix_in` / `decode_into` | `pool_byte(w, ps, ..)` | one view `pv` per call | = except VE |
| `put_prefix`, `write_prefix`, `put_title_prefix` | `PoolBuf::append_span` | `write_prefix` took `(w, ps)` and re-derived the source per byte | `put_prefix` builds the view `src` once; `write_prefix(.., src: [i64], ..)` | = except PB |
| `pstr_cmp`, `pair_cmp` | `units(a).cmp(units(b))` | `pool_byte(pool, 0, ..)` | `byte(pool, ..)`; still the byte walk with the surrogate fix-ups | CMP |
| `truthy`, `nullish`, `ntag`, `strict_eq`, `num_int`, `key_len`, `key_put` | `Value::truthy`, `nullish`, `strict_eq`, `num_int -> Option<i64>`, `key -> Option<Vec<u16>>` | | unchanged | VE, O |
| `pkt_size`, `logical_pool_words` | `ent_packet` `logical_size` | | unchanged, on `ent_packet::sec_len` / `sec_start` (the V option "keep") | = |

`lib/ent_packet.pruv` `vs`: the high-water is `let hi: i64 = a + b;` (was `clamp(a, 2^32) + clamp(b, 2^32)`); it
verifies from the `span_ok` `ensures`. That closes **V** and **P-hi**.

Rows:

- **VE** (API row, owner: the consumer steps): a slot stays three words read by position (`tag_at(w, p)`, `slot_off`,
  `slot_bytes`) where the hand parses it once into `enum Value` (`Undefined`, `Null`, `Bool`, `Ref`, `Num`, `Str(Span)`,
  `Id`, `Absent`, `Object`, `Other`). Pruv has payload enums (SPEC 6a), so this is no language gap: about 75 public
  `(w, p)` functions are called from `req/*`, `ent_*` and `entry_api`, and each consumer step parses its slots.
- **PB** (API row): writers take `inout o: [i64]` plus a word offset `dest` (the pool lives inside a packet being
  built), where the hand appends to a `PoolBuf { units, nbytes, words: Vec<i64> }`. Same consumers as VE.
- **CMP** (kept): `pstr_cmp` walks bytes and fixes up the four-byte / lone-surrogate order (`pair_cmp`); the hand
  compares the lazy UTF-16 unit iterators. Pruv has no iterator adaptors (row I), so the byte walk stays; same order.
- **O** (from 3.2): `key_len` returns `-1` and `num_int` returns `2^53` for `None`.
- **U** (from 3.1): units are `[i64]`, offsets `i64`.

### `ent_state` (deferred)

`ent_state` is not reshaped in 3.3 (time cap). No language gap blocks it; recorded so 3.4 can take it with
`ent_dispatch`:

- **CX** (open, whole module): `Ctx` is threaded by value (`fn f(cx0: Ctx, ..) -> Ctx`, `let mut cx: Ctx = cx0;`
  then return it) with `stalled` / `bad` as `i64` flags checked after every observation. The hand shape is
  `&mut Ctx` methods (`inout cx: Ctx`, closed as IF / B5) and an observation result `enum Obs { Answer, Stalled, Bad }`
  (or `Result`), with the request (`rcode`, `raux`, `rpool`, `rrec`, `rtab`, `rntab`) and the recorded `prev` request
  (`qon`, `qexact`, `qns`, `qpool`, `qrec`, `qtab`) as two `Option<Request>` fields instead of 13 loose fields.
  `ctx_new` fills ten `empty()` temporaries (hand `Vec::new()`), and `ctx_new` / `obs` copy `blob`, `lg`, `rpool`,
  `rrec`, `rtab`.
- **Boundary with `ent_dispatch`**: `ent_state::with_prev` calls `ent_dispatch::blob_pool(prev)`, and `ent_dispatch`
  calls `ent_state` 17 times (`ctx_new`, `with_prev`, `finish`, `replay_await`, `state_ok`, `st_*`, `log_*`). `Ctx`
  is used by `ent_sources` (59 sites), `ent_canon` (63), `entry_api` (18), `ent_bound` (12), `ent_dispatch` (17), and
  `q_mutants.py` (M04 targets `ent_state`; M08 to M12 target `ent_dispatch`). 3.4 must: move `blob_pool` next to the pool readers (it is a
  pool slice of a packet: `ent_value`), so `ent_state` no longer calls `ent_dispatch`; then change `obs` / `ctx_*` to
  `inout cx: Ctx` one caller module at a time (`ent_dispatch` first, then `ent_bound`, `ent_sources`, `ent_canon`,
  `entry_api`), keeping the item contract "check after every observation" as the `Obs` result.
- `ent_state` state words (`[magic, op, code, aux, ck0..7, L, G, blob, log]`) are the await packet format (section 2),
  external input: the hand parses them once (`state_ok` + `st_*`) into a struct; today every `st_*` slices again.

### Codex structure comparison

One read-only comparison (Codex gpt-6.2-astra high) of `ent_value` against the hand and HEAD. Findings and what was done:

| Finding | Done |
|---|---|
| no per-byte view or clamp left in the readers | = |
| `append_span` / `append_units` re-derived the destination (`pool_at(o, dest)` in `ends_high` plus `rd(o, dest + 1)`) | Pruv: one `dv` view per call, `nbytes(dv)` and `ends_high(dv, nb)` (the hand reads `self` once) |
| `write_prefix` calls `append_span(src, 0)`; `append_slot`, `pool_head`, `pool_cat`, `pslot_ok` build the view at the call | kept: `(w, start)` API wrappers for the consumers (VE / PB) |
| `append_units` past 16,777,216 units, `pstr_cmp` on a span `span_ok` rejects, decode of an `F4 90..` form, `truthy` on a number word `>= 2^32` differ from the hand | kept: all outside the checked domain (`wire_check` / `span_ok` reject the input first) |
| `enum Value`, the `(w, start)` API, `PoolBuf`, iterators / `pair_cmp`, `Option` results | rows VE, PB, CMP, O, API |

### Tests (after the last code change)

`bench/l3/lib/hand`: `rustc --edition 2021 --test` on `ent_value.rs` (4 tests), pass. Budget m11 / m12 do not use
`ent_value` or `ent_state` (they use only `ent_u16`), so they were not run.

| Test | Pairs | Result |
|---|---|---|
| `entry/test_impl.py` | rs/rust, rs/llvm | 296 variants x 2 = 592 runs, unexplained 0 |
| `entry/judge_test.py` | rs | failures 0 |
| `entry/q.py --count 400` | rs/rust, rs/llvm | 399 exact, 1 excluded-domain, unexplained 0 |
| `req/test_impl.py` | rs/rust, rs/llvm | 160 variants x 2 = 320 runs, unexplained 0 |
| `req/q.py` (q_gen, 2000 cases) | rs/rust, rs/llvm | 2000/2000 equal on each pair |
| `entry/integrate.py` (after qwen 26/26 + codex 136/136 on both pairs, opencode 199/199 and grok `--no-ts` 254/254 on rs/rust) | rs/rust | 10 integration cases, 18 checks PASS, 2 integration limits (I01/I02 claude, known; exit 2 as in 3.1 / 3.2) |

The first opencode run (concurrent with the entry tests) failed one fixture in setup (`F01-xdg-blank`: the two sides
built different trees, before any implementation result is compared); run alone it passed 199/199.

### Rows open after 3.3

- Closed: **V**, **P-hi**.
- New: **VE**, **PB** (API rows, owned by the consumer steps), **CMP** (row I, no iterator adaptors), **CX**
  (`ent_state`, 3.4 with `ent_dispatch`).
- Still open: U, K, L (language / API gaps), E, O (API rows), S (consumer rows), P-start (proof-only).

## Step 3.4: `ent_state` and `ent_dispatch`

Hand baselines `lib/hand/ent_state.rs` (`Ctx` with `&mut self` methods, `State` view parsed once, `Request` / `Prev` /
`Output`, `Halt { Stalled, Bad }`, `obs(..) -> Result<(), Halt>`) and `lib/hand/ent_dispatch.rs` (`find_source`,
`route`, `invoke`, `rethrow`, `list_recipe`, `list_external`, `read_external`, each `fn(cx: &mut Ctx)`), written first,
with rustc tests (9 in `ent_state.rs`, 3 more in `ent_dispatch.rs`; `rustc --edition 2021 --test ent_dispatch.rs`
runs all 12, pass).

### What changed

- `blob_pool` moved from `ent_dispatch` to `ent_value` (next to the pool readers); `ent_state::use_prev`, `ent_bound`
  (3 sites) and `entry_api` (1 site) call `ent_value::blob_pool`. `ent_state` no longer calls `ent_dispatch`.
- `ent_state`: `pub struct State { code, aux, ck, blob, log }` of `ref` views, built once by `state_view(st)`;
  `state_ok` and `st_replay_ok` read it. `pub enum Halt { Stalled, Bad }`. New `inout cx: Ctx` functions with the hand
  shape: `observe(..) -> Result<(), Halt>` (hand `Ctx::obs`), `use_prev` (`with_prev`), `drop_entries` (`drop_level`),
  `accounts`, `account_at`, `nullary`, `ambient`, `homedir`, `cwd`, `path_req` (`path`), `realpath`, `stat`,
  `put_ck`, `put_done` (`set_done`), `stop_too_big` / `stop_fail` (`too_big` / `fail`, return `Err(Halt::Bad)`).
- `ent_dispatch`: `find_source`, `rethrow` take `inout cx`; `list_recipe`, `list_external`, `read_external` return
  `Result<(), Halt>` and observe with `observe(..)?` (no `stalled` / `bad` check after the call); `req9` (7 positional
  arguments, two call sites working out `r0` and `route`) became `invoke(blob, read: bool)` as in the hand.
- `q_mutants.py`: M11 text follows the new `rethrow` (`let e: ref [i64]` after the abort check); every mutant text
  occurs exactly once (checked by script).

### Rows

- **CX** partly closed: observation is a `Result` with `?`, `inout cx` in `ent_state` / `ent_dispatch`, state words
  parsed once (`State`). Left: rows OA, ENT, CK, AD below.
- **OA** (new, language gap): `Option<T>` with a struct payload that holds arrays is rejected (`type_argument`), so the
  request (`rcode`, `raux`, `rpool`, `rrec`, `rtab`, `rntab`), the prev request (`qon`, `qexact`, `qns`, `qpool`,
  `qrec`, `qtab`), the output (`opool`, `orec`, `otab`, `ontab`) and the fault (`bad` = 1 protocol / 2 too big, hand
  `Option<Fault>`) stay loose `Ctx` fields. Not worked around.
- **ENT** (new): `Ctx.ent` is a copy of the current log entry (hand: a `Range` into `lg`). Needs a stored range or a
  view field with a lifetime tied to `cx.lg`; left for the language.
- **CK** (new, language gap): a fixed array (`ck: [i64; 8]`) cannot appear in a type used in a `pub` signature
  (`fixed_array_position`), so `ck` stays `[i64]` padded to 8 words.
- **DR** (new): `drop_entries` rebuilds `lg` with `cat(slice, slice)`; the hand uses `Vec::drain`. No in-place range
  removal in Pruv (row P-cat family).
- **LR** (new, structural): `list_recipe` stores the clip code in `tab[7i+4]` and reads it back; the hand keeps a
  `Vec<Option<Clip>>` and writes the table in one pass. Fixable in Pruv without a gap; left for the `ent_dispatch`
  pass of 3.5 (time cap).
- **AD** (new, adapters): by-value wrappers kept so that the other `Ctx` users compile unchanged. Each is
  `let mut cx = cx0; f(inout cx, ..); return cx;`:

| Adapter (by value) | `inout` function | Users | Removed in |
|---|---|---|---|
| `obs` | `observe` | `ent_bound` | 3.5 |
| `too_big` | `stop_too_big` | `ent_bound` | 3.5 |
| `set_done` | `put_done` | `ent_bound`, `ent_canon`, `ent_sources` | 3.5, 3.6 |
| `fail` | `stop_fail` | `ent_canon`, `ent_sources`, `entry_api` | 3.5, 3.6, 3.7 |
| `with_prev` | `use_prev` | `ent_canon`, `ent_sources`, `entry_api` | 3.5, 3.6, 3.7 |
| `drop_level`, `ctx_ambient`, `ctx_homedir`, `ctx_cwd`, `ctx_realpath`, `ctx_stat` | `drop_entries`, `ambient` .. `stat` | `ent_canon` | 3.5 |
| `ctx_accounts`, `ctx_account_at`, `set_ck` | `accounts`, `account_at`, `put_ck` | `ent_sources` | 3.6 |
| `st_ck`, `st_blob`, `st_log` (copy of a `State` field) | `state_view` | `ent_canon`, `ent_sources`, `entry_api` | 3.5, 3.6, 3.7 |
| `find_item`, `list_item`, `read_item` | `find_source`, `list_external`, `read_external` | `entry_api` | 3.7 |

### Structure comparison

Two read-only comparisons (hand `.rs` against `.pruv`, function by function). The default Codex account was at its
usage limit, so the first was done by a fresh Claude agent that had not seen the conversion, and the second (Codex
gpt-6-astra high, second account) after its fixes. Neither found a semantic mismatch in the Pruv. Findings and what
was done:

| Finding | Done |
|---|---|
| hand `repack` set the units word of a cut pool with `prefix_in(.., usize::MAX)` (a byte count); Pruv `prefix_pool_units` counts units | hand fixed (`units_in`); the Pruv was right |
| `req9` took 7 positional arguments; both callers worked out `r0` and `route` | Pruv `invoke(blob, read: bool)` as in the hand |
| `log_ok` / `log_cont` copied each entry with `slice` | `subview` |
| `canon_ck_ok` had a dead `d > 2^32` test after the clamp | removed |
| `path_req` walked the path twice (`slot_len_of` before `encode_pool`) | `pool_bytes(pool, 0)` after encoding, as the hand |
| `repack` guard `pkt_size > 2^24` (hand none; `env_check` rejects that input first) | kept (proof domain) |
| `log_*` / `observe` repeat the entry parse (`n < 12 or n > g - pos`) four times; hand has `entry_at` | kept: needs an iterator / `Option<Entry>` (rows CMP, OA) |
| `state_ok` returns `bool` and `st_replay_ok` re-views the words (hand `State::parse` / `replay_ok(&self)`) | kept: callers in `ent_canon` / `entry_api` use the bool form (AD, 3.5 / 3.7) |
| `ck_phase_only` / `s0_of` are named `set_ck_arr0` / `st_s0` | kept: names used by `ent_canon` (3.5) |
| `rethrow` `has_detail` branch is mutant M12 scaffolding | kept (M12 site) |
| `list_recipe` clip codes stored in `tab` | row LR |
| (Codex) `log_append`, `encode_state`, `repack` zero-fill a buffer and write by position; the hand reserves and appends | `arr_reserve` / `arr_push` / `arr_append` as the hand (`encode_state` pads `ck` to 8 words, row CK; `repack` clamps the pool word range and returns empty past `2^25` words, proof-only) |
| (Codex) `blob_pool` copied the pool (`slice`); the hand borrows | returns `ref [i64] from(blob)`; callers that need an owned pool copy it (`ent_dispatch` before `observe`, as the hand `to_vec`) |
| (Codex) `find_source` scans every row; hand `position` stops at the first match | kept: mutant M08 (last match wins) needs the full scan to be observable |
| every other function: same result as the hand on checked input | = |

### Tests (after the last code change)

`bench/l3/lib/hand`: `rustc --edition 2021 --test ent_dispatch.rs` (12 tests: 9 from `ent_state.rs`, 3 dispatch), pass.
The rs compiler builds and verifies the entry root (rust and llvm backends) and the req root.

| Test | Pairs | Result |
|---|---|---|
| `entry/test_impl.py` | rs/rust, rs/llvm | 296 variants x 2 = 592 runs, unexplained 0 |
| `entry/judge_test.py` | rs | failures 0 |
| `entry/q.py --count 400` | rs/rust, rs/llvm | 399 exact, 1 excluded-domain, unexplained 0 |
| `req/test_impl.py` | rs/rust, rs/llvm | 160 variants x 2 = 320 runs, unexplained 0 |
| `req/q.py` (q_gen, 2000 cases) | rs/rust, rs/llvm | 2000/2000 equal on each pair |
| `entry/integrate.py` (after qwen 26/26 + codex 136/136 on both pairs, opencode and grok `--no-ts` all fixtures on rs/rust) | rs/rust | 10 integration cases, 18 checks PASS, 2 integration limits (I01/I02 claude, known; exit 2 as in 3.1 to 3.3) |

Budget m11 / m12 import none of `ent_state`, `ent_dispatch` or `ent_value` (grep), so they were not run.

### Rows open after 3.4

- Closed: none fully; **CX** partly (rest is OA, ENT, CK, AD).
- New: **OA**, **CK** (language gaps), **ENT**, **DR**, **LR**, **AD** (adapters, removed in 3.5 to 3.7).
- Still open: U, K, L, E, O, S, P-start, VE, PB, CMP.
- Next (3.5): `ent_bound` and `ent_canon` to `inout cx` (removes most AD rows), `list_recipe` one pass (LR).

## Step 3.5: `ent_bound`, `ent_canon` and `list_recipe`

Hand baselines `lib/hand/ent_bound.rs` (ops 7 / 8 / 9: `ses_pairs` into `Vec<Pair>`, `session_threads`,
`excluded_from_bound`, `index_collect`, `index_progress_ok`, `index_request`, `index_complete` with a `HashMap` index)
and `lib/hand/ent_canon.rs` (`home_dir` / `home_join` / `resolve_cwd` / `canonical` / `codex_home` returning
`Result<Vec<u16>, Halt>`, `same_file` / `is_codex_home` returning `Result<bool, Halt>`, `folder_item`,
`codex_folder_begin` / `codex_folder_step`), written first, with rustc tests (`rustc --edition 2021 --test` on
`ent_bound.rs`: 15 tests, `ent_canon.rs`: 17, `ent_dispatch.rs`: 12, `ent_state.rs`: 9; all pass).

### What changed

- `Halt` gets a third variant, `Done` (hand and Pruv): a nested step already set the output, e.g. a thrown
  `homedir` / `cwd` / `realpath` answer rethrown as the item result. It replaces the `early(cx)` check
  (`stalled || bad || done`) after every nested call in `ent_canon`.
- `ent_bound`: every item takes `inout cx: Ctx` and returns `Result<(), Halt>` (`observe(..)?`, `put_done`,
  `stop_too_big`). `ses_pairs` is one pass with `arr_push` (was count, then fill into preallocated arrays);
  `throw_at` / `throw_existing_at` (by-value `Ctx` in and out) became `throw_rec(answer, p) -> [i64]` as the hand
  `thrown`; tables are `arr_reserve` + `arr_push` (no positional fill; `index_complete` truncates a key it just
  pushed when the hash says it is a duplicate, as the hand skips the insert); `index_complete` builds its
  throw packet with `mk_packet` directly instead of a throwaway `ctx_new` + `finish`. Renamed to the hand names:
  `threads_item` body -> `session_threads`, `excluded_item` body -> `excluded_from_bound`.
- `ent_canon`: every step takes `inout cx: Ctx` and returns `Result<(), Halt>` or `Result<bool, Halt>` with `?` on
  `homedir`, `cwd`, `ambient`, `realpath`, `stat`; `drop_entries`, `use_prev`, `state_view`, `put_done`,
  `stop_fail` replace the by-value adapters. Hand names: `throw_existing` -> `rethrow_answer` (returns
  `Halt::Done`), `homedir_units` -> `home_dir`, `home_sub` -> `home_join`, `host_home` -> `codex_home`,
  `is_host_codex_home` -> `is_codex_home`; `has_double_slash` is shared by `norm_abs` / `norm_rel` as in the hand.
- `ent_dispatch::list_recipe` (row LR): the first loop collects the clip codes into their own array (`clips`, a
  non-string title is not cut), a second loop counts the copies, the third builds the table in one pass with
  `arr_reserve` + `arr_push` (was: clip code parked in `tab[7i+4]`, table allocated full and filled by position).
- `ent_state`: adapters with no user left are removed: `obs`, `drop_level`, `too_big`, `ctx_ambient`,
  `ctx_homedir`, `ctx_cwd`, `ctx_realpath`, `ctx_stat`.
- `q_mutants.py`: M07 text follows the new `is_codex_home` (`return Ok::<bool, crate::ent_state::Halt>(true)`, the
  `.` target check); M06, M10, M13, M15, M16
  texts unchanged and still found. Every mutant original occurs exactly once in its file (checked by script).

### Rows

- **LR** closed.
- **RA** (new, language gap): `Result<[i64], Halt>` is rejected (`type_argument`: an array is not a valid type
  argument), so `home_dir`, `home_join`, `resolve_cwd`, `canonical`, `codex_home` (hand `-> Result<Vec<u16>, Halt>`)
  leave the path in `cx.opool` and return `Result<(), Halt>`. `Result<bool, Halt>` works and is used.
- **SA** (new, language gap): no array of structs (the llvm backend rejects `Vec<S>`), so `ses_pairs` /
  `index_complete` keep a pair as parallel arrays (`ep`, `el`, `tp`, `sp`); hand `Vec<Pair>`.
- **HK** (new, language gap): `index_complete` builds its first-write-wins index as a hand-written open-addressing
  table; hand `HashMap<Vec<u16>, usize>`. Pruv maps take no array key, and `String` cannot hold lone surrogates.
- `index_complete` keys stay in a flat buffer with `ko` / `kl` offsets (rows PB, J, O); `eng_len` / `eng_put`
  (hand `eng_units`) rows PB, O; `slash_from_end` returns `-1` (row O); `state_ok_ie` returns `bool` (hand
  `Option<State>`, row OA); pools stay `[i64]` words (row U).
- **AD** after 3.5:

| Adapter (by value) | `inout` function | Users | Removed in |
|---|---|---|---|
| `set_done`, `set_ck`, `ctx_accounts`, `ctx_account_at` | `put_done`, `put_ck`, `accounts`, `account_at` | `ent_sources` | 3.6 |
| `fail`, `with_prev`, `st_code`, `st_aux`, `st_ck`, `st_blob`, `st_log` | `stop_fail`, `use_prev`, `state_view` | `ent_sources`, `entry_api` | 3.6, 3.7 |
| `ent_canon::early`, `with_pool`, `throw_existing`, `homedir_units`, `home_sub`, `host_home`, `is_host_codex_home` | `rethrow_answer`, `home_dir`, `home_join`, `codex_home`, `is_codex_home` | `ent_sources` | 3.6 |
| `ent_bound::threads_item`, `excluded_item` | `session_threads`, `excluded_from_bound` | `entry_api` | 3.7 |
| `find_item`, `list_item`, `read_item` | `find_source`, `list_external`, `read_external` | `entry_api` | 3.7 |

### Structure comparison

One read-only comparison by Codex (gpt-6-astra, second account): hand `.rs` against `.pruv`, function by function,
for `ent_bound`, `ent_canon` and `list_recipe`. It found no semantic mismatch on checked input; the structure
findings and what was done:

| Finding | Done |
|---|---|
| F1 `colon_at` read a non-string slot; hand returns 0 | tag check, returns 0 |
| F2 `ses_pairs` failed inside the loop at the pair limit; hand checks the count after | count check after the loop; the in-loop `to > 16777216` guard stays as the proof bound (same failure) |
| F3 `index_complete` ran a length pass, then a key pass, then a hash pass | one pass per candidate (support check, key push, size check, lookup; a hit truncates the key) |
| F4 the value span check stopped at the first kept entry | span check and sum over every kept entry, `full = sum > pl` |
| F5 `index_complete` table zero-filled and written by position; key range a failure branch | `arr_reserve` + `arr_push`, key slots in `kat` / `kb`, the range is a loop invariant |
| F6 `ent_str` did not check the tag | tag check (hand `str_at`) |
| F7 `canonical` had an `f0 > 2^25` failure the hand does not have | removed; the loop proves without it |
| F8 `canonical` built the stat id and the climb paths in several copies | one reserved buffer each, `slash_cat` helper; length guards are proof-only |
| F9 `codex_home` trimmed a copy of the slot | `trim(ent_str(cx, 1))` |
| F10 `codex_folder_step` copied the state words and called `state_view` three times | `st` is a `ref` subview; the three `state_view` calls stay (`st_replay_ok` takes `[i64]`, see below) |
| F11 `folder_item` compared through `str_eq_arr` | tag check, then `eq(str_units, expected)` as the hand |
| F12 `list_recipe` cut a non-string title | only a string title (tag 6) is cut |
| F13 `list_recipe` counted copies inside the clip loop; hand counts the `Some` clips after | separate count loop |
| every other function: same result as the hand on checked input | = |

### Tests (after the last code change)

`bench/l3/lib/hand`: `rustc --edition 2021 --test` on `ent_bound.rs` (15 tests), `ent_canon.rs` (17),
`ent_dispatch.rs` (12), `ent_state.rs` (9), pass. The rs compiler builds and verifies the entry root (rust and llvm
backends) and the req root.

| Test | Pairs | Result |
|---|---|---|
| `entry/test_impl.py` | rs/rust, rs/llvm | 296 variants x 2 = 592 runs, unexplained 0 |
| `entry/judge_test.py` | rs | failures 0 |
| `entry/q.py --count 400` | rs/rust, rs/llvm | 399 exact, 1 excluded-domain, unexplained 0 |
| `req/test_impl.py` | rs/rust, rs/llvm | 160 variants x 2 = 320 runs, unexplained 0 |
| `req/q.py` (q_gen, 2000 cases) | rs/rust, rs/llvm | 2000/2000 equal on each pair |
| `entry/integrate.py` (after qwen 26/26 + codex 136/136 on both pairs, opencode and grok `--no-ts` all fixtures on rs/rust) | rs/rust | 10 integration cases, 18 checks PASS, 2 integration limits (I01/I02 claude, known; exit 2 as in 3.1 to 3.4) |

Budget m11 / m12 import only `ent_u16` (grep), none of the changed modules, so they were not run.

### Rows open after 3.5

- Closed: **LR**; **AD** rows for `obs`, `too_big`, `drop_level`, `ctx_ambient`, `ctx_homedir`, `ctx_cwd`,
  `ctx_realpath`, `ctx_stat` (adapters removed).
- New: **RA**, **SA**, **HK** (language gaps).
- Kept (F10 rest): `codex_folder_step` calls `state_view` three times because `st_replay_ok` takes the state words
  (`[i64]`) rather than a `State`; the hand parses once and calls `replay_ok(&self)`. Changing `st_replay_ok` is an
  `ent_state` change for its other users (`entry_api`), left for 3.7 with row OA.
- Still open: U, K, L, E, O, OA, S, P-start, VE, PB, CMP, ENT, CK, DR, J, AD (3.6 / 3.7 adapters above).
- Next (3.6): `ent_sources` to `inout cx`, removing the `ent_canon` and `ent_state` adapters it still uses.

## Step 3.6: `ent_sources`

`ent_sources` (import of the source list: home rows, per-engine account rows, codex home, opencode, qwen) now follows
the hand shape in `hand/ent_sources.rs`: every item takes `inout cx: Ctx` (and `inout rows: Rows` where it emits),
returns `Result<(), Halt>` and propagates with `?`. It calls `put_done`, `put_ck`, `accounts`, `account_at`,
`stop_fail`, `use_prev`, `state_view`, `rethrow_answer`, `home_dir`, `home_join`, `codex_home` and `is_codex_home`
directly, so the by-value adapters it alone used are gone.

### What changed

- `hand/ent_sources.rs` (new): the baseline, with `rustc --test` tests (begin emits the home row then stalls on the
  accounts observation; accounts and the next item run in one step; claude account rows skip an invalid id; rows
  are placed after the front pool; the last phase is done). It includes `ent_canon.rs` (which includes
  `ent_state.rs` and the rest), so the included modules' tests run too.
- `hand/ent_state.rs`: `Ctx::next_item` (reset per-item observation state between items). `hand/ent_canon.rs`: the
  helpers `ent_sources` calls are `pub`.
- `ent_sources.pruv` rewritten function by function against the hand file: same names, same split
  (`advance`, `ck_word`, `rec0`, `env_trim`, `row_head`, `push_row`, `home_row`, `account_row`, `env_or_home`,
  `home_item`, `accounts_item`, `account_ok`, `account_of`, `claude_acct`, `codex_acct`, `codex_home_item`,
  `opencode_item`, `qwen_item`, `phase_item`, `rebase`, `place_rows`, `drive`, `import_begin`, `import_step`).
  Types match: `struct Rows { pool, rec }`, `enum Name { Str(units), Raw(tag, a, b) }`, `struct Account { id, name }`.
  `drive` loops over items with `let Ok(()) = r else { break; }` and matches the stop (`Halt::Stalled` -> replay
  await, `Halt::Done` -> done packet, bad -> boundary), as the hand `match`.
- `ent_state.pruv`: `next_item` added (`inout cx`); `ctx_accounts`, `ctx_account_at`, `set_ck`, `set_done` removed.
- `ent_canon.pruv`: `early`, `with_pool`, `throw_existing`, `homedir_units`, `home_sub`, `host_home`,
  `is_host_codex_home` removed (grep: no other users).
- `q_mutants.py`: M02 (`env_or_home(inout cx, 6, ".codex")?` before the codex home row), M04 (the `ent_sources` half
  now matches the `a.id` hash, the checkpoint-6 skip and the row/advance/`put_ck` lines) and M05 (`stop_fail` on an
  out-of-range account index) follow the new text; M03 is unchanged. Every mutant original occurs exactly once
  (checked by script). M02, M03, M04, M05 are killed (S08, E02, A07, A08).
- `entry/IMPL_NOTES.md`: the Ctx API line names the `inout` functions.
- Fix from the structure comparison (F1): `rebase` clamps byte offsets to 2^30 instead of 2^24, in both files.

### Rows

- **LB** (new, language gap): `arr_append(inout a, b, s, n)` requires `len(a) + n <= 2^40`. Neither the length of
  `rows.rec` nor that of `rows.pool` is bounded by a type, so `push_row` checks
  `len(rows.rec) > 2^40 - len(rec)` and returns; hand `extend_from_slice` (a `Vec` cannot reach that length before
  allocation fails). The guard is dead on every real input (packets are at most 16,777,216 words).
- **RA** used: `env_or_home` and `codex_home` leave the path in `cx.opool` and return `Result<(), Halt>`
  (hand `Result<Vec<u16>, Halt>`); `home_item` passes `cx.opool` to `home_row`, `codex_home_item` likewise.
- **CK** used: `advance` writes checkpoint words 0..2 and clears 3..8 with `put_ck` in a `for` loop (hand
  `cx.ck = [ph, idx, cnt, 0, 0, 0, 0, 0]`); `import_step` seeds the checkpoint with `set_ck_arr0(s.ck)`.
- **OA** used: the per-item output is `Rows` threaded through `inout rows` rather than an `Option<Rows>` return.
- **MP** (closed by parity bundle P9a-6: `_` arms, literal and nested variant patterns; or-patterns are P9a-7) (new, language gap): `codex_acct` matches the `is_codex_home` result as `Ok(host)` then `Err(h)` with an
  inner `match h` listing `Done`, `Stalled`, `Bad`; the hand writes `Ok(false)`, `Ok(true) | Err(Halt::Done)`,
  `Err(h) => return Err(h)`. Checked with pruvc: `Ok(false)` is `parse/syntax` (no literal pattern),
  `Err(H::A)` is `parse/syntax` (no nested variant pattern) and a `_` arm is `parse/reserved_name`, so the inner
  match spells out every variant and rebuilds the `Err`.
- **place_rows**: both rebase `rows.rec` in place (Pruv `rebase` has `ensures len(rec) == len(old(rec))`). The hand
  builds the pool with two `PoolBuf::append_span` calls and a `view_words` copy of `rows.pool`; Pruv calls
  `pool_base` and `pool_cat` (row U: pools are `[i64]` words), one copy fewer.
- **AD** after 3.6:

| Adapter (by value) | `inout` function | Users | Removed in |
|---|---|---|---|
| `set_done`, `set_ck`, `ctx_accounts`, `ctx_account_at` | `put_done`, `put_ck`, `accounts`, `account_at` | none | 3.6 (removed) |
| `ent_canon::early`, `with_pool`, `throw_existing`, `homedir_units`, `home_sub`, `host_home`, `is_host_codex_home` | `rethrow_answer`, `home_dir`, `home_join`, `codex_home`, `is_codex_home` | none | 3.6 (removed) |
| `fail`, `with_prev`, `st_code`, `st_aux`, `st_ck`, `st_blob`, `st_log` | `stop_fail`, `use_prev`, `state_view` | `entry_api` | 3.7 |
| `ent_bound::threads_item`, `excluded_item` | `session_threads`, `excluded_from_bound` | `entry_api` | 3.7 |
| `find_item`, `list_item`, `read_item` | `find_source`, `list_external`, `read_external` | `entry_api` | 3.7 |

### Structure comparison

One read-only comparison by Codex (gpt-6.2-astra, effort high; engine, model and account named by the user): hand
`.rs` against `.pruv`, function by function, for `ent_sources`. It found one behaviour bug common to both (F1);
the other findings and what was done:

| Finding | Done |
|---|---|
| F1 `rebase` clamped the old offset and `add` to 2^24, but offsets are bytes (7 per pool word), so a string past byte 2^24 (a few million three-byte units, inside the packet limit) was mis-placed | both clamp to 2^30 (the `pool_base` bound); hand test `rebase_keeps_offsets_past_2_pow_24` |
| F4 hand `env_trim` went through `str_at` + `trim` for a non-string slot; Pruv checks the tag first | hand checks the tag first (and clamps `k` to 16 as Pruv) |
| F5 Pruv `account_ok` computed `sec_start` before the nullish test | the nullish test comes first (`&&` as the hand) |
| F5 rest: `valid_account_id` decodes before / after the length check | done in 3.8: both check the length (20 units) before decoding |
| F2 `pool_put` encodes each string into a temporary pool and copies the whole pool | kept: row U / PB (pools are `[i64]` words; no `PoolBuf` with reserve) |
| F3 `place_rows`: hand has an extra `view_words` copy, Pruv decides the join boundary twice (`pool_base`, `pool_cat`) | kept: row U / PB |
| F6 path pieces: hand `join` over a slice, Pruv `join2` / `join4` | kept: no array of arrays (row SA), `ent_path` is outside this step |
| F7, F8, F10 request, replayed answer, `put_done` record and `ctx_new` buffers are copied where the hand moves them | kept: `ent_state` ownership, with `entry_api` in 3.7 (rows OA, AD) |
| F9 hand `drive` takes `cx.req` before the size check, Pruv leaves it | kept: the `Ctx` is dropped right after, not observable |
| F11 `import_step` parses the state three times | kept: F10 rest from 3.5 (`st_replay_ok` on `State`, 3.7) |
| F12 hand defines `rd` and `pool_put` locally | kept: hand `put_vs` lives in `ent_packet`, which depends on `ent_value`, so `pool_put` cannot move there; `rd` is the `i64` word reader (row U) |
| F13 tests only in the hand file | kept: the Pruv side is covered by `test_impl` / Q on both pairs |

### Tests

All after the last code change (the F1 / F4 / F5 fixes).

| Test | Pairs | Result |
|---|---|---|
| `rustc --test hand/ent_sources.rs` (includes `ent_canon`, `ent_state`, `ent_bound` and the rest) | hand | 23 passed |
| `entry/dev_build.sh --no-link` | rs | verified, `{"ok": true}` |
| `entry/q_mutants.py --only M02,M03,M04,M05` | rs/rust | 4/4 killed (S08, E02, A07, A08), 0 verification-rejected |
| `entry/test_impl.py` | rs/rust, rs/llvm | 296 variants x 2 = 592 runs, unexplained 0 |
| `entry/judge_test.py` | rs/rust, rs/llvm | failures 0 |
| `entry/q.py --count 400` | rs/rust, rs/llvm | 399 exact, 1 excluded-domain, unexplained 0 |
| `req/test_impl.py` | rs/rust, rs/llvm | 160 variants x 2 = 320 runs, unexplained 0 |
| `req/q.py` (q_gen, 2000 cases) | rs/rust, rs/llvm | 2000/2000 equal on each pair |
| `qwen/test_impl.py` | rs/rust, rs/llvm | 26/26 each, verifier PASS |
| `codex/test_impl.py` | rs/rust, rs/llvm | 136/136 each |
| `opencode/test_impl.py --pairs rs/rust`, `grok/test_impl.py --no-ts --pairs rs/rust` | rs/rust | all fixtures passed |
| `entry/integrate.py` | rs/rust | 10 integration cases, 18 checks PASS, 2 integration limits (I01/I02 claude, known; exit 2 as in 3.1 to 3.5) |

### Rows open after 3.6

- Closed: **AD** rows for `set_done`, `set_ck`, `ctx_accounts`, `ctx_account_at` and the seven `ent_canon`
  adapters (removed).
- New: **LB**, **MP** (language gaps).
- Still open: U, K, L, E, O, OA, S, P-start, VE, PB, CMP, ENT, CK, DR, J, RA, SA, HK, LB, MP, AD (3.7 adapters above).
- Next (3.7): `entry_api` to `inout cx`, removing the last adapters (`fail`, `with_prev`, `st_*`, `find_item`,
  `list_item`, `read_item`, `threads_item`, `excluded_item`); fold `st_replay_ok` onto `State` (F10 rest from 3.5).

## Step 3.7: `entry_api`

`hand/entry_api.rs` is new. It is the crate root of a shared build: the hand modules `ent_dispatch`, `ent_bound`,
`ent_canon` (and its `ent_state` / `ent_path` / `ent_u16`) and `ent_sources` now declare their dependencies as
`#[cfg(not(shared_ctx))] #[path] mod X; #[cfg(shared_ctx)] pub use crate::X;`, so under `--cfg shared_ctx` one `Ctx`
type runs every item, and each module still checks alone without the cfg. `ent_packet::ent_wire` and
`ent_state::ent_value_pkt_size` became `pub` for it. `account_home_begin` belongs to `ent_accounts`, which has no hand
baseline; `hand/entry_api.rs` carries a stand-in. Check:
`rustc --edition 2021 --cfg shared_ctx --test entry_api.rs -o /tmp/entry_api_hand && /tmp/entry_api_hand` (38 tests,
7 of them new: find / list / read round trips, begin rejects, account home, ops without an await, the op 9 round trip).

### What changed

- `entry_api.pruv`: `run_item(inout cx: Ctx) -> Result<(), Halt>` calls `find_source`, `list_external`,
  `read_external`, `session_threads`, `excluded_from_bound` and `stop_fail` directly (an if-chain for the hand
  `match`). `entry_begin_with_env` folds the op 9 reject into the kind / seq check and builds the `Ctx` with
  `move blob, move lg`. `entry_step_at`: `e1` / `e2` are value `if`s; the `seq >= 2^53` test joins the completion
  header test; `st` is a `ref` subview (it was a `slice` copy); the state is viewed once
  (`let s = state_view(st); if !state_ok(st, s, ..) || !replay_ok(s, ..)`) and `s.code`, `s.aux`, `s.log`, `s.ck`
  are read from the view; `ctx_new(op, id, move blob, move lg, s.ck)` with `blob = copy(s.blob)` (hand `to_vec`),
  then `use_prev(inout cx, prev, true)`.
- `ent_state.pruv`: `ctx_new` takes `take blob`, `take lg` and moves them into the `Ctx` (it copied both; 3.6 F7 /
  F8 for the step paths). `state_ok(st, s: State, op, op_id)` takes the caller's view; `st_replay_ok(st, ..)` is
  `replay_ok(s: State, op, seq)` (hand `State::replay_ok(&self)`). Removed: `fail`, `with_prev`, `st_code`, `st_aux`,
  `st_ck`, `st_blob`, `st_log`.
- `ent_canon.pruv` `codex_folder_step` and `ent_sources.pruv` `import_step`: one `state_view` instead of three (F10
  rest from 3.5, F11 from 3.6); `codex_folder_begin` / `import_begin` move the repacked blob into the `Ctx`.
- Begins pass an empty checkpoint to `ctx_new` (it zero-pads to 8 words) instead of `zeros(8)`; hand `&[0; 8]`.
- `ent_dispatch.pruv`: `find_item`, `list_item`, `read_item` removed. `ent_bound.pruv`: `threads_item`,
  `excluded_item` removed. Headers updated.
- `q_mutants.py`: no original text changed; every original occurs exactly once (checked by script).

### Rows

- **OR** (new, language gap): hand `State::parse(st, op, id) -> Option<State>` and
  `.filter(|s| s.replay_ok(op, seq))`. A borrowing struct (`-> State from(st)`) cannot be the payload of an
  `Option` / `Result` (pruvc: `type/borrow_signature`), so the caller views once and checks with `state_ok(st, s, ..)`
  returning `bool`. Used by `entry_step_at`, `codex_folder_step` (`state_ok_ie`), `import_step`.
- **TM** (new, language gap): an argument for a `take` parameter must be a named local with `move x`
  (`type/foreign_move` on a call result or on `copy(..)`), so `repack(..)`, `empty()` and `copy(s.blob)` are bound to
  a local before `ctx_new`; the hand passes `repack(input, hi)`, `Vec::new()`, `s.blob.to_vec()` directly. One extra
  `let` per argument, no extra copy.
- **VB** (new, language gap): a value `if` arm is one expression (SPEC R-6w-1). `index_begin_checked` binds
  `strings` with `let mut strings = empty(); if units <= pl { ..loop..; strings = pool_finish(..) } else { strings =
  copy(blob_pool(blob)) }`; the hand writes `let strings = if .. { ..loop..; compact.view_words() } else { .. }`.
  The `empty()` is a zero-length array (no heap block).
- **CK** used: the begin checkpoint is an empty `[i64]` padded by `ctx_new` (hand `&[0; 8]`, no fixed-array literal);
  `import_step` seeds `set_ck_arr0(s.ck)` as a `[i64]` (hand `[i64; 8]`).
- **AD** closed: every by-value adapter is removed (grep: no users left).

| Adapter (by value) | `inout` function | Users | Removed in |
|---|---|---|---|
| `fail`, `with_prev`, `st_code`, `st_aux`, `st_ck`, `st_blob`, `st_log`, `st_replay_ok` | `stop_fail`, `use_prev`, `state_view`, `replay_ok` | none | 3.7 (removed) |
| `ent_bound::threads_item`, `excluded_item` | `session_threads`, `excluded_from_bound` | none | 3.7 (removed) |
| `find_item`, `list_item`, `read_item` | `find_source`, `list_external`, `read_external` | none | 3.7 (removed) |

### Structure comparison

One read-only comparison by Codex (gpt-6.2-astra, effort high; engine, model and account named by the user): hand
`.rs` against `.pruv`, function by function, for `entry_api` and the begin / step / `Ctx` functions touched here (20
functions). No behaviour difference found. Findings and what was done:

| Finding | Done |
|---|---|
| `entry_begin_with_env`, `codex_folder_begin`, `import_begin` allocate `zeros(8)` for the checkpoint, then `ctx_new` allocates its own 8 words; the hand borrows `&[0; 8]` | the begins pass `empty()` (no heap block), `ctx_new` zero-pads (row CK) |
| `import_step` phase checkpoint is a `[i64]` temporary (hand `[i64; 8]`) | kept: row CK |
| `ctx_new` writes zeros into the already-zero tail for a short `ck`; `Ctx` fields are flat (`qexact: i64`, `qtab` cleared first) where the hand has `[i64; 8]`, `Option<Prev>`, `bool` | kept: rows CK, CX / OA (`Ctx` layout) |
| `stop_fail` stores `bad = 1`, hand `Some(Fault::Protocol)` | kept: row CX (`Ctx` layout) |
| `index_begin_checked` reads the pool through `(blob, start)` helpers where the hand builds one `Pool` view | kept: row VE / API (`(w, start)` wrappers) |
| `index_*` lengths / offsets are `i64` with range `requires` (hand `usize`) | kept: row U |
| `account_home_begin` (`ent_accounts`) allocates the invalid-id record `[3, 0]` (stand-in borrows a literal) | kept: `ent_accounts` has no hand baseline (Next) |

### Tests

All after the last code change (the empty begin checkpoint from the structure comparison).

| Test | Pairs | Result |
|---|---|---|
| `rustc --cfg shared_ctx --test hand/entry_api.rs` (all hand modules in one crate) | hand | 38 passed; each hand module alone passes as before |
| `entry/dev_build.sh --no-link` | rs | verified, `{"ok": true}` |
| `entry/q_mutants.py` originals | - | every original occurs exactly once (script; no mutant text changed) |
| `entry/test_impl.py` | rs/rust, rs/llvm | 296 variants x 2 = 592 runs, unexplained 0 |
| `entry/judge_test.py` | rs/rust, rs/llvm | failures 0 |
| `entry/q.py --count 400` | rs/rust, rs/llvm | 399 exact, 1 excluded-domain, unexplained 0 |
| `req/test_impl.py` | rs/rust, rs/llvm | 160 variants x 2 = 320 runs, unexplained 0 |
| `req/q.py` (q_gen, 2000 cases) | rs/rust, rs/llvm | 2000/2000 equal on each pair |
| `qwen/test_impl.py` | rs/rust, rs/llvm | 26/26 each, verifier PASS |
| `codex/test_impl.py` | rs/rust, rs/llvm | 136/136 each |
| `opencode/test_impl.py`, `grok/test_impl.py --no-ts` | rs/rust, rs/llvm | all fixtures passed on 2 pairs each |
| `entry/integrate.py` | rs/rust | 10 integration cases, 18 checks PASS, 2 integration limits (I01/I02 claude, known; exit 2 as before) |

### Rows open after 3.7

- Closed: **AD** (all adapters removed).
- New: **OR**, **TM**, **VB** (language gaps).
- Still open: U, K, L, E, O, OA, S, P-start, VE, PB, CMP, ENT, CK, DR, J, RA, SA, HK, LB, MP, OR, TM, VB.
- Next (3.8, `docs/RUST_SHAPE_RESET.md` §6, step 3 item 8): `entry/root`, then `req`. Still missing from step 3: a hand
  baseline for `ent_accounts` (step 3.4 listed it with `ent_dispatch`; `hand/entry_api.rs` only has an
  `account_home_begin` stand-in, and 3.6 F5 rest waits on it). The `req` port waits for the P10H experiment.

## Step 3.8: `ent_accounts` and `entry/root`

`hand/ent_accounts.rs` is new (step 3.4 listed `ent_accounts` with `ent_dispatch`; until now only stand-ins existed in
`hand/ent_canon.rs` and `hand/entry_api.rs`). It declares `ent_state`, `ent_path`, `ent_u16` with the `shared_ctx`
pattern and is used by `ent_canon` (op 6), `ent_sources` (op 1) and `entry_api` (op 5); the stand-ins are removed.
Check: `rustc --edition 2021 --test ent_accounts.rs` (15 tests, 4 of them new). `hand/entry_root.rs` is new and is
now the crate root of the shared build, as `entry/root.pruv` is for the Pruv build: it declares every hand module
once and the four host functions; `hand/entry_api.rs` takes its modules from `crate::`. Check:
`rustc --edition 2021 --cfg shared_ctx --test entry_root.rs -o /tmp/entry_root_hand && /tmp/entry_root_hand`
(44 tests, 3 of them new: begin forwards, begin rejects a bad wire pool, index_step reads the poison).

### What changed

- `ent_accounts.pruv` `valid_account_id`: tag test, then the pool view and the slot span once, the unit count
  (20) before decoding (3.6 F5 rest), then the prefix test and a `for i in 4..20` loop; the hand
  `Value::parse` / `units_in` / `decode_span` order. `ent_value.pruv` makes `units_in`, `decode_span` and `slot_off`
  `pub` for it (structure comparison F1).
- `io_check`: the checker and the record start first, then the hand `match (op, record length)` as an if-chain
  (row MP); op 1's seven env slots are a `for k in 1..8` loop (it was a mask table).
- `account_home_begin`: the invalid-id record is the literal `[3, 0]` (it was `zeros(2)` plus writes; 3.7 finding),
  empty sections are one `let e: ref [i64] = []`, the done record is `zeros(4)` without the redundant write.
- `ent_canon.pruv` `state_ok_ie`: `hi != pool_bytes(..)` alone, as the hand compares `io_check(..) != Some(nbytes)`
  (the `hi < 0 ||` short circuit is gone; `pool_bytes >= 0`, so `-1` is still rejected; F4).
- Hand: `account_home(data_dir, engine, id)` takes `engine: &[u16]` (callers pass `&lit("codex")` /
  `&lit("claude")`), as the Pruv `[i64]` argument.
- `entry/root.pruv` already had the hand shape; header note only.
- `q_mutants.py`: no original text changed; all 23 originals occur exactly once (checked by script).

### Rows

No new row. Used:

- **MP**: `io_check` if-chain for `match (op, rl)` (the MP row itself is left to the P9a-6 bundle).
- **U / PB**: `account_home_begin` hand `encode_pool(&home)` then `pool.view_words()` (one copy into words); Pruv
  pools are words already (`pool_new` / `pool_put`).
- **VE**: `str_units(w, p)` is the `(w, p)` wrapper in `ent_value` and decodes without a tag test; the hand's local
  `str_units` returns empty for a non-string. Only called after `io_check` proved the three slots strings.
- **J**: `account_home` calls `join4`, which unrolls the four `push_part`s (hand loops over `&[&[u16]]`); `ent_path`
  row, not changed here.
- **O**: `io_check` returns `-1` for the hand `None`.

### Structure comparison

One read-only comparison by Codex (gpt-6.2-astra, effort high; engine, model and account named by the user): hand
`.rs` against `.pruv` for the 6 `ent_accounts` functions, the 4 root functions and the 12 callers of `ent_accounts`
(22 functions). Both already checked the id length before decoding. Findings and what was done:

| Finding | Done |
|---|---|
| F1 `valid_account_id`: Pruv resolved the pool and the span twice (`str_len`, `str_units`); hand once. Codex: a direct call with slot `[6, -1, 20]` differs (unreachable: `io_check` checks every slot span first) | one pool view and span, `units_in` then `decode_span` (as hand) |
| F2 `account_home` → `join4` unrolled, an over-long part drops the rest (hand loops) | kept: row J (`ent_path`, unreachable on checked packets) |
| F3 `str_units` decodes without a tag test (hand: empty for non-string) | kept: row VE (only after `io_check`) |
| F4 `state_ok_ie`: Pruv `hi < 0 ||` short circuit, hand compares the `Option` | removed, plain compare |
| root `entry_begin` / `entry_step` / `index_begin` / `index_step`, `io_check`, `account_home_begin`, other callers | same |

### Tests

All after the last code change (F1 / F4 from the structure comparison).

| Test | Pairs | Result |
|---|---|---|
| `rustc --cfg shared_ctx --test hand/entry_root.rs` (all hand modules in one crate) | hand | 44 passed; each hand module alone passes (`ent_accounts` 15) |
| `entry/dev_build.sh --no-link` | rs | verified, `{"ok": true}` |
| `entry/q_mutants.py` originals | - | all 23 occur exactly once (script; no mutant text changed) |
| `entry/test_impl.py` | rs/rust, rs/llvm | 296 variants x 2 = 592 runs, unexplained 0 |
| `entry/judge_test.py` | rs/rust, rs/llvm | failures 0 |
| `entry/q.py --count 400` | rs/rust, rs/llvm | 399 exact, 1 excluded-domain, unexplained 0 |
| `req/test_impl.py` | rs/rust, rs/llvm | 160 variants x 2 = 320 runs, unexplained 0 |
| `req/q.py` (q_gen, 2000 cases) | rs/rust, rs/llvm | 2000/2000 equal on each pair |
| `qwen/test_impl.py` | rs/rust, rs/llvm | 26/26 each, verifier PASS |
| `codex/test_impl.py` | rs/rust, rs/llvm | 136/136 each |
| `opencode/test_impl.py`, `grok/test_impl.py --no-ts` | rs/rust, rs/llvm | all fixtures passed on 2 pairs each |
| `entry/integrate.py` | rs/rust | 10 integration cases, 18 checks PASS, 2 integration limits (I01/I02 claude, known; exit 2 as before) |

### Rows open after 3.8

- New: none. Closed: none (MP is left to the P9a-6 bundle).
- Still open: U, K, L, E, O, OA, S, P-start, VE, PB, CMP, ENT, CK, DR, J, RA, SA, HK, LB, MP, OR, TM, VB.
- Next: step 3 is done except `req` (`docs/RUST_SHAPE_RESET.md` §6 step 3 item 8), which waits for the P10H
  experiment. Step 4 (fragments) starts with the MCP stdio check (§6 step 4 item 1): it is the first product
  candidate and its `R` modules make it the smallest port; then the supervisor, Grok RPC and history.

## Step 4.1-4.2: MCP recheck and the command supervisor

- 4.1 (MCP stdio check): `mcp/RESHAPE.md` "4.1 recheck". The product path uses `pkgs/*` only (no `lib/` JSON copy);
  rows R, T and L of that file are closed by `arr_reserve`/`arr_append`, in-place `truncate` and literal `[u8]`
  arguments; B (one copy per trimmed line) stays: the language side exists (borrowing structs on LLVM, R-6g9-2),
  the change is a borrowed document in `pkgs/json`/`pkgs/lines` with all their consumers. `test_impl.py` 22/22 on
  rs/rust and rs/llvm. No code changed.
- 4.2 (supervisor): `supervisor/RESHAPE.md`. The supervisor now uses `pkgs/tail` and `pkgs/args` (its private
  `btail.pruv`, `args.pruv` and `packet.pruv` are gone), the result packet is the state from the start as the hand's
  `packet`, the file half of the chunk arm is `capfile::write` (hand `write`, split out of the hand loop for the
  table), and the prefix is written from a view instead of a copy. New rows:
- **TH** (new, language/host gap): no threads or channels; a child's pipes are read through the host event `Loop`
  (`ev_wait`, `ev_proc_read_into`, slot observers, fallible `ev_new`/`ev_add_*` with kind-3 early returns) where the
  hand runs two reader threads with `sync_channel` + `recv_timeout`. Same family as pkgs X1/X2 (**W-c**). Minimal
  change: a scoped-thread + bounded-channel API over owned pipe halves, or accept the event model as the host shape.
- **FO** (new, language gap): `File` cannot be in an `Option` or an `inout` parameter (SPEC §6h placement), so the
  hand's `&mut Option<File>` is a handle threaded through `take f -> File` with `file_writable` as "Some"
  (`capfile::write`, `supervise::take`). Minimal change: `inout f: File` and `Option<File>` with must-consume on both
  arms. Not worked around.
- **FV** (new, language gap): a builtin's array argument cannot be a `subview` (`type/view_context` covers Pruv calls
  only), so `file_write_bytes(f, &bytes[..n])` goes through the one-line Pruv wrapper `capfile::write_all` (no copy;
  before 4.2 the prefix was `truncate(copy(data), n)`). Minimal change: allow views for builtin read-only `[u8]`
  parameters (`file_write_bytes`, `net_write_bytes`).
- **UL** (new, language gap): no unbounded loop. `while` needs a `decreases` measure and `loop { }` is
  `parse/loop_unsupported`, so the supervisor's main loop (hand `loop`, ends when the child ends or the controller
  stops it) carries an `events < 10000000` cap and the drain loop an `n < 10000000` cap; reaching the event cap
  ends as kind 3. Minimal change: a `while`/`loop` without `decreases` in a function marked possibly non-terminating
  (an effect such as `! diverge`), with partial-correctness proofs of its invariants. Not worked around.
- Codex structure review (hand vs Pruv) fixes, in `supervisor/RESHAPE.md` §1: after both pipes close the socket is
  still polled and the exit is taken by a 2 ms `ev_proc_wait_for` (`try_wait`), as the hand's `try_wait` arm (before,
  a blocking `ev_proc_wait` missed a stop request sent after the pipes closed); exit fields are written into the
  packet where decided; `arg_spans` runs after the range tests; the control read is 1 byte; `core_root::core` takes
  the hand's assert as `requires`.
- SPEC R-7-7 still names `bench/l3/supervisor/btail.pruv` (`put_cp`) as its example; the file is gone. SPEC is not
  edited in bench steps; the example should move to `pkgs/tail` when R-7-7 is next touched.

### Rows open after 4.2

- New: TH, FO, FV, UL. Closed: none here (MCP R, T, L were already closed in that bench's own table; recorded in 4.1).
- Still open: U, K, L, E, O, OA, S, P-start, VE, PB, CMP, ENT, CK, DR, J, RA, SA, HK, LB, MP, OR, TM, VB, TH, FO, FV, UL.
- Next: step 4.3, Grok RPC and history (`docs/RUST_SHAPE_RESET.md` §6 step 4). Separately: MCP gap B (borrowed
  document in `pkgs/json`/`pkgs/lines`, all consumers) and the supervisor performance remeasure on the new shape.

## Step 4.3: Grok RPC (history deferred to 4.3b)

- `grok/RESHAPE.md`. All of `grok/rpc.pruv` is in the `grok/perf/hand_rpc.rs` shape: no JSON tape on any line (escaped
  keys matched while decoding, `key_is`; the object `error` read from spans, `detail` and `compact`), the 16-word `ctl`
  and the `aux` buffer replaced by a `Check` struct, `req_load` one reservation and appends, `not_handled_line` one
  reservation and three appends, the perf split by `find_byte`. The hand gained the load check (`Rpc`, `Check`,
  `load_check`, `failure`) so every Pruv function has a counterpart; the only word packet left is the mode 3 result
  packet at the host boundary.
- Codex structure review: two behaviour differences were hand bugs (a write to a closed stdin, the 8 MiB line cap tested
  after the append) and are fixed in the hand; six structure findings applied in Pruv; four left as open rows of that
  bench (4.3b-U, -V, -C, -D in `grok/RESHAPE.md` §3; they are structure work, not language gaps).
- **VP** (new, verifier gap): independent `if` statements fork the symbolic path at each one, so a run of N
  mutually exclusive `if x == k { ... }` costs 2^N paths. `value` had sixteen after its recursive call (plus six
  before); the whole grok root did not finish verifying in 150 minutes. The hand writes `match slot { ... }`; Pruv has
  no literal arms on an `i64` scrutinee (R-6u-6 literals are payload positions only), so `value` uses `else if` chains,
  which verify in seconds and keep one test per arm. Minimal reproducer (work notes `vp16_*.pruv`): a recursive
  `walk(b, p, d, inout c)` whose loop body does `let v = walk(...)` and then sixteen `if k == i { c.f{i%4} = v + i; }`
  verifies in 147 s; the same with `else if` in 1 s. Minimal change: literal arms on integer scrutinees (lowered to the
  same decision tree), and/or path merging after an `if` without `else` that only assigns. Not worked around: the chain
  is the closest form of the hand's `match`.
- Uses of existing rows: TH (the child read on the event loop), L (`req_init` literal), UL (pump and drain caps),
  B family (`text_span` returns a tagged span, no borrowed return).
- Length contracts added for verification only: `compact` (`e - s <= 8388608`, `len(out) <= 6 * (e - s)`),
  `push_escaped`, `dec` (`result <= e - s`), `hexd`, `on_reply` (`len(line) <= 8388608`); the perf root's `rpc_line`
  tests `c_e - c_s <= 8388608` before `compact`.
- Tests (`grok/RESHAPE.md` §4): `test_impl.py --no-ts` 254/254 on rs/rust and rs/llvm; q 300 random cases + fixtures,
  3 load-timing differences per pair that pass when rerun alone; `run_perf.py check` 8/8 equal on both pairs.
- `bench/l3/lib` and `bench/l3/pkgs` unchanged.

### Rows open after 4.3

- New: VP. Closed: none. Bench rows open in `grok/RESHAPE.md`: 4.3b-U, 4.3b-V, 4.3b-C, 4.3b-D.
- Still open: U, K, L, E, O, OA, S, P-start, VE, PB, CMP, ENT, CK, DR, J, RA, SA, HK, LB, MP, OR, TM, VB, TH, FO, FV, UL, VP.
- Next (4.3b): first the four rpc rows above (`grok/rpc.pruv` only). Then the history path against
  `grok/perf/hand_history.rs`: the line scanner (`line.pruv`, `framing.pruv`) already follows the hand per function;
  what differs is (1) the collector store: hand `Vec<Slot>` of 200 owned prompt/reply Vecs, Pruv a 4-word state, an
  8-word-per-slot `[i64]`, offsets and a stamp array threaded through `read::{window,event_step,line_step}` and
  `entry::{feed,user_view,assistant_view}`; (2) the success packet: hand reserves the final `Vec<i64>` once, Pruv builds
  `rows`, `meta` and the final packet (`read::assemble_store`); (3) `entry::query_range` allocates a 2-word result where
  the hand returns `Option<(usize, usize)>`; (4) `Canon` keeps an offset where the hand copies 24 bytes (Pruv also accepts
  numeric stamps; keep that behaviour). The collector half sits in `pkgs/turns` (`new_state`, `new_slots`,
  `new_store_hint`, `begin2`, `skip_user`, `answer2`, `tool`, `text_layout`), so its consumers must be run.

## Step 4.3b: Grok RPC rows U, V, C, D and `entry::query_range`

- `grok/RESHAPE.md` §3c. The four rpc structure rows are closed: UTF-8 is checked per sequence inside strings (U, hand
  `utf8_width`); `array` and `object` are separate loops under `value` (V); `Cap` holds only the members and the callers
  trim and pass the view (C); `dec` appends plain runs found with `find_byte` and `text_of` returns
  `ref [u8] from(t, buf)` (D, the two-source borrow; the tagged `text_span` and the `resize` zero fill are gone).
- `entry::query_range` returns a `text::Span` instead of a 2-word `[i64]` (history item 3). `Canon` (item 4) already
  borrows an offset where the hand copies 24 bytes: no change.
- Codex structure review (one, per function): five findings applied, one of them an output difference in the perf lane
  (a `code` token over 8 MiB was not compacted); one left as bench row 4.3b-R (the `load_check` line buffer is
  zero-filled because `pkgs/transport::receive_grow` needs length >= 1).
- **IW** (new, language gap): an `inout` struct parameter cannot be assigned a fresh value as a whole
  (`c = cap_new()` on `inout c: Cap` is `type/immutable`, "inout parameter c cannot be reassigned as a whole"). The
  hand's `scan` returns a fresh `Cap`; Pruv resets its 22 fields one by one (the closest form; same cost, no
  allocation). Same family as the array/map cases G-R06-1 and D63 (`docs/L3_RESTRUCTURE_SPEC.md`), but those block a
  grow or a put-back; this one only costs text. Minimal change: allow whole assignment of an `inout` struct (the
  storage is the caller's, as for a field).
- Length contracts changed for verification only: `compact` `e - s <= 16777216` (was 8388608), `dec`
  `len(out) + (e - s) <= 100000000`, `push_cp` `len(o) <= 100000000`, `text_of` `len(t) <= 16777216`.
- Tests: `grok/RESHAPE.md` §4. `bench/l3/lib` and `bench/l3/pkgs` unchanged (no consumer runs).

### Rows open after 4.3b

- New: IW. Closed: none (bench rows 4.3b-U, -V, -C, -D closed in `grok/RESHAPE.md`; 4.3b-R opened there).
- Still open: U, K, L, E, O, OA, S, P-start, VE, PB, CMP, ENT, CK, DR, J, RA, SA, HK, LB, MP, OR, TM, VB, TH, FO, FV, UL, VP,
  IW.
- Next:
  1. Grok history path against `grok/perf/hand_history.rs` (items 1-2 of the 4.3 list): the collector store in
     `pkgs/turns` (`new_state`, `new_slots`, `new_store_hint`, `begin2`, `skip_user`, `answer2`, `tool`,
     `text_layout`) and `read::assemble_store` (hand reserves the final packet once, Pruv builds `rows`, `meta` and the
     packet). Write the missing hand Rust first; run every `pkgs/turns` consumer on rs/rust and rs/llvm. Bench row
     4.3b-R (`pkgs/transport` `receive_grow` from length 0) can go with it if transport consumers are run anyway.
  2. Qwen, then Codex, then OpenCode together with Claude R08.
  3. Step 4.4 (`docs/RUST_SHAPE_RESET.md` §6).

## Step 4.3c: Grok history packet and bench row 4.3b-R

- `grok/RESHAPE.md` §3d. `read::assemble_store` has the shape of the hand `Collector::packet`: one `arr_reserve` of the
  final size on an empty array, rows pushed oldest turn first, text pushed, header and meta written in place (was
  `rows` + `meta` + a zero-filled packet, three allocations). `packet::rows_ok` removed (no caller). `event_step` /
  `window` take the event ranges as preconditions (no `j`, no `bad` flag) and `line_step` drops its unreachable 16 MiB
  check (Codex review).
- Bench row 4.3b-R closed: `pkgs/transport::receive_grow` accepts and keeps `len(line) <= cap` (was `1 <= len(line)`),
  grok `load_check` reserves 4096 on an empty buffer like the hand; `mcp/root.pruv` relaxes its loop invariant only.
- Collector store: VL (`../pkgs/RESHAPE.md` step 2.12) re-probed with this `pruvc-rs`, still rejected on LLVM (a
  `Vec<Slot>` field or `inout Vec<Slot>` parameter of a struct with `[u8]` fields). `pkgs/turns` unchanged, so its
  consumers other than grok were not run; transport consumers (grok, codex, mcp) were.
- Tests: `grok/RESHAPE.md` §4.

### Rows open after 4.3c

- New: none (language). Bench rows: 4.3b-R closed; 4.3c-T, 4.3c-B, 4.3c-G opened in `grok/RESHAPE.md` §3d.
  VL (pkgs) reconfirmed.
- Still open: U, K, L, E, O, OA, S, P-start, VE, PB, CMP, ENT, CK, DR, J, RA, SA, HK, LB, MP, OR, TM, VB, TH, FO, FV, UL, VP,
  IW, VL.
- Next:
  1. Qwen, then Codex, then OpenCode together with Claude R08.
  2. Step 4.4 (`docs/RUST_SHAPE_RESET.md` §6).
  3. Grok bench rows 4.3c-T and 4.3c-B (`entry.pruv` only) and 4.3c-G (`pkgs/transport`, all transport consumers);
     `pkgs/turns` dead code (`begin3`, `answer3`, `clip_span`, `put_nl_span`, `put_pad`, `write_span`) with all turns
     consumers run.

## Step 4.3d: Qwen head, tail and window import

- `qwen/RESHAPE.md`. New hand baseline `qwen/handrust/src/edge.rs` (`head_import`, `tail_import`; the hand had only
  `window_import`), checked against the 21 qwen fixtures (0 differences). Byte loops the hand does not write became
  `find_byte` (`framing::skip_first`, `window_next` without its 16-byte pre-pass, `chain::capacity`) and `eq_range`
  (`chain::same_span` removed); `put_str` / `chain::put` became `copy_range`; zero-filled exact-size results became
  `arr_reserve` + `arr_append`; eight `copy(v)` of owned match binders in `entry` became `move`; `replay` is a
  literal `match` on `event.kind` (R-6u-8); dead `idle`, `ep`, `seen` and `Block::Veto` removed.
- No module moved to `lib` / `pkgs`; no shared module changed, so no consumer runs were needed.
- Tests: `qwen/RESHAPE.md` §4.

### Rows open after 4.3d

- New (language): 4.3d-M, `move s.f` of a `[u8]` field of a `let`-bound struct returned by value is `type/array_alias`;
  TK / LB9 cover a consuming match binder only (`qwen/import.pruv` `tail_info` keeps `copy(iso.text)`).
- New (bench, `qwen/RESHAPE.md` §2-§3): 4.3d-H (no hash map, arena + bucket index kept), 4.3d-C (LF pre-pass sizing),
  4.3d-T (title / stamp decided in the scan), 4.3d-O (`publish` / `make_output` take ownership), 4.3d-P (`{` prefilter
  before parse), 4.3d-W (`store_le` / `load_le` wire words).
- Still open: U, K, L, E, O, OA, S, P-start, VE, PB, CMP, ENT, CK, DR, J, RA, SA, HK, LB, MP, OR, TM, VB, TH, FO, FV, UL, VP,
  IW, VL.
- Next:
  1. Codex, then OpenCode together with Claude R08.
  2. Step 4.4 (`docs/RUST_SHAPE_RESET.md` §6).
  3. Grok bench rows 4.3c-T, 4.3c-B, 4.3c-G and `pkgs/turns` dead code (as listed after 4.3c).
  4. Qwen bench rows 4.3d-C, 4.3d-T, 4.3d-O, 4.3d-P, 4.3d-W (`qwen/` only; 4.3d-T must keep the `last-title` mutant
     in `q_mutants.py` meaningful or move it).

## Step 4.3e: Qwen bench rows C, P, W, O, T

- `qwen/RESHAPE.md` §1-§3. 4.3d-C (uuid index, `selected` and `Events` grow per object, no LF pre-count),
  4.3d-P (`{` prefilter in `entry::parse_line`), 4.3d-W (`store_le` / `load_le` wire words), 4.3d-O
  (`publish` / `make_output` take and move), 4.3d-T (`entry::role_of` / `custom_title`: role, title and stamp
  decided in the scan) are done. A second Codex review led to three more changes: `human_text` stops before
  provenance on a non-user type; only parsed objects are entries; `resolve` returns early without a leaf.
- 4.3d-H is now a language row: Pruv maps (R-6e-5) take owned `String` keys only, so the hand's
  `HashMap<&str, &Entry>` with borrowed slice keys cannot be written.
- No `lib` / `pkgs` module changed, so no consumer runs were needed. Tests: `qwen/RESHAPE.md` §4 (verifier PASS and
  26/26 on `rs/rust` and `rs/llvm`; q 300 cases, unexplained 0 on both).

### Rows open after 4.3e

- Language: 4.3d-M (`move s.f` of a `[u8]` field of a `let`-bound struct), 4.3d-H (map keys must be owned `String`).
- New (bench, `qwen/RESHAPE.md` §3): 4.3e-D (prompt / stamp / parts computed before selection and turn linkage),
  4.3e-L (leaf candidate kept during the scan; role decided before uuid / parent), 4.3e-F (`put_field` decodes then
  copies; needs a `lib/access` decode-into-buffer form), 4.3e-J (`array_content` owns each part; `title_key` owns
  blank titles), 4.3e-S (`write_stamp` str round trip).
- Closed: 4.3d-C, 4.3d-P, 4.3d-W, 4.3d-O, 4.3d-T.
- Still open: U, K, L, E, O, OA, S, P-start, VE, PB, CMP, ENT, CK, DR, J, RA, SA, HK, LB, MP, OR, TM, VB, TH, FO, FV, UL, VP,
  IW, VL.
- Next:
  1. Codex, then OpenCode together with Claude R08.
  2. Step 4.4 (`docs/RUST_SHAPE_RESET.md` §6).
  3. Grok bench rows 4.3c-T, 4.3c-B, 4.3c-G and `pkgs/turns` dead code (as listed after 4.3c).
  4. Qwen bench rows 4.3e-D, 4.3e-L, 4.3e-J, 4.3e-S (`qwen/` only), then 4.3e-F (touches `lib/access`; run all
     consumers).

## Step 4.3f: Codex list and read import

- `codex/RESHAPE.md`. New hand baseline `codex/hand/codex.rs` (the whole import, one function per Pruv function; the
  older `reader.rs` / `replay.rs` cover only the R09 perf lanes), built with `rustc -O` against `host/` serde_json, no
  warnings. `ctl: [i64]` became `rpc::Ctl` (two write-only slots dropped) and the error class a `match`; request
  builders reserve once and append (`push_int`, `push_escaped`; `join` / `int_bytes` / `escaped` removed); `kind_of`
  and `not_handled` read the id token in place (`id_is` on the span); the reply folds into the turn's own `[u8]`
  (`fs` / `fb` scratch, `put_piece`, `head` removed); `receive` and the plan line split use `find_byte`; `take_segment`
  trims a borrowed `subview`; `list` appends into one `[u8]` (no `String` + `adapter_probe` + `to_bytes`); `finish`
  copies title / model with `copy_range`; `trim_span` returns `Bounds`. One Codex review: `not_handled` writes `null`
  for an absent id (bug), `np` / `off` parallel state and dead `packet::wire_read` removed.
- No module moved to `lib` / `pkgs`; no shared module changed, so no consumer runs were needed.
- Tests: `codex/RESHAPE.md` §4.

### Rows open after 4.3f

- New (language): 4.3f-S, `eq_range` rejects `str` operands (`folder::same`, the `"lair"` originator prefix keep byte
  loops).
- New (bench, `codex/RESHAPE.md` §3): 4.3f-A (failure-path allocations and checks), 4.3f-O (`absorb` full signal and
  cursor order), 4.3f-N (`ctl.n` dead writes, `parse_line` inout), 4.3f-K (`kind_of` early return), 4.3f-L (phrase
  lowercase and 300-unit decode), 4.3f-T (`item_text` state), 4.3f-P (plan `pl` / `pb` store, `marker_end` -1),
  4.3f-R (`child_args` copies), 4.3f-D (`str_at` decoder).
- New (pkgs): 4.3f-B (zero-filled receive buffer and unread stderr tail, also `pkgs/transport`), 4.3f-E (element
  cursor in `pkgs/access` instead of `util::elements`), 4.3f-F (double reply clip, replay re-fold, double layout in
  `pkgs/turns`; with VL).
- Still open: U, K, L, E, O, OA, S, P-start, VE, PB, CMP, ENT, CK, DR, J, RA, SA, HK, LB, MP, OR, TM, VB, TH, FO, FV, UL, VP,
  IW, VL, 4.3d-M, 4.3d-H.
- Next:
  1. OpenCode together with Claude R08.
  2. Step 4.4 (`docs/RUST_SHAPE_RESET.md` §6).
  3. Grok bench rows 4.3c-T, 4.3c-B, 4.3c-G and `pkgs/turns` dead code; Qwen bench rows 4.3e-D, 4.3e-L, 4.3e-J,
     4.3e-S, then 4.3e-F (touches `lib/access`; run all consumers).
  4. Codex bench rows 4.3f-A, 4.3f-O, 4.3f-N, 4.3f-K, 4.3f-L, 4.3f-T, 4.3f-P, 4.3f-R, 4.3f-D (`codex/` only), then
     4.3f-B, 4.3f-E, 4.3f-F (shared modules; run all consumers).

## Step 4.3g: OpenCode import

- `opencode/RESHAPE.md`. New hand baseline `opencode/hand/opencode.rs` (rusqlite, `std::process`, serde_json; one
  function per Pruv function or step group), `cargo build --release`, no warnings. Zero-filled buffers filled by index
  became `arr_reserve` + `arr_push` / `arr_append` (`plan::fit`, `packet::cat_*` / `append_str`, `export::with_span`,
  `util::to_bytes`, `convert::model_bytes` / `assemble`, `path::join` / `normalize`); `path::contained` / `same_key`
  use `eq_range`; span copies are `copy(subview(..))`; `skip` / `bad` / `found` / `total` flags became `continue` or
  early returns; the attach slots `mt: [i64]` became `entry::Attach`. One structure review (Claude Opus fallback):
  three hand-side behavior differences fixed (lossy UTF-8 cells, bank bound, configuration check order).
- No module moved to `lib` / `pkgs`; no shared module changed, so no consumer runs were needed.
- Tests: `opencode/RESHAPE.md` §4 (199/199 on both pairs, `q.py` 499 cases with 0 unexplained).

### Rows open after 4.3g

- New (language / host): 4.3g-SQ, no SQLite or file-system host API (the OpenCode step machine stands in for direct
  calls).
- New (bench, `opencode/RESHAPE.md` §3): 4.3g-P (pool append returns a new array), 4.3g-R (`rows::decode` buffers and
  copies), 4.3g-Q (select SQL / pool chain), 4.3g-X (export prefix normalizer and spawn pool), 4.3g-K (span copies made
  only to compare), 4.3g-D (`OPENCODE_DB` read twice), 4.3g-N (`normalize` scratch), 4.3g-A (result packet zero fill,
  double shape check), 4.3g-F (`format_i64`, `format_ext`, zero-filled item arrays), 4.3g-T (`Attach` tape ids,
  `dirname` / `iso_str` round trips), 4.3g-E (hand spawn failure lacks the native / raw / name words).
- Still open: U, K, L, E, O, OA, S, P-start, VE, PB, CMP, ENT, CK, DR, J, RA, SA, HK, LB, MP, OR, TM, VB, TH, FO, FV, UL, VP,
  IW, VL, 4.3d-M, 4.3d-H, 4.3f-S (now also `convert::has_prefix`, `path::key_eq`, `sql::lit_eq`), 4.3f-E.
- Next:
  1. 4.3h: Claude R08 (`bench/l3/claude`).
  2. Step 4.4 (`docs/RUST_SHAPE_RESET.md` §6).
  3. OpenCode bench rows 4.3g-P, 4.3g-R, 4.3g-Q, 4.3g-K, 4.3g-D, 4.3g-N, 4.3g-A, 4.3g-X, 4.3g-F, 4.3g-E (`opencode/`
     only; 4.3g-P first, it turns most pool builders into one `inout` pool).
  4. Grok bench rows 4.3c-T, 4.3c-B, 4.3c-G and `pkgs/turns` dead code; Qwen bench rows 4.3e-D, 4.3e-L, 4.3e-J,
     4.3e-S, then 4.3e-F (touches `lib/access`; run all consumers).
  5. Codex bench rows 4.3f-A, 4.3f-O, 4.3f-N, 4.3f-K, 4.3f-L, 4.3f-T, 4.3f-P, 4.3f-R, 4.3f-D (`codex/` only), then
     4.3f-B, 4.3f-E, 4.3f-F (shared modules; run all consumers).

## Step 4.3h: Claude R08 import

- `claude/RESHAPE.md`. The hand baseline `claude/handrust/src/lib.rs` (safe std Rust) gained `// Pruv: module::fn`
  comments on every function and a header naming the Pruv functions with no hand counterpart; its code did not
  change, `cargo build --release`, no warnings. This step supersedes the AGENTS.md "claude frozen" note (§6).
- Arena writes are `copy_range` / `copy_within` (`turns::put_str` / `put_nl` / `move_stamp` removed); the clip is
  `text::clip_len` with no `substr` copy; the LF search is `find_byte`; the dropped-user scan takes a `subview`;
  `idle` flags and `offset = len(data)` exits are `break`; binders and owned locals `move`; the timestamp is read
  where the hand reads it and `consume_entry` is one function.
- One Codex review (gpt-6.2-astra, high): one behavior difference fixed (`entry::window_line` returned a document
  parsed from a shortened copy; the projection is removed and `read_window` calls `parse_line`), plus the first-piece
  hold in `array_content`, the rank check before the title read, `move` / `copy_range` in `time`, and `break` in
  `trim_span`. `q_mutants.py` originals 4 and 6 follow the new text (18/18 present exactly once).
- D-R08-3 (text crosses as one `i64` per byte): the hand widens the same way, so it needs a packet ABI change on
  both sides; kept as a row. D-R08-2 (drop evicted turns): the ring stays (row VL).
- No module moved to `lib` / `pkgs`; no shared module changed, so no consumer runs were needed.
- Tests: `claude/RESHAPE.md` §4.

### Rows open after 4.3h

- New (bench, `claude/RESHAPE.md` §3): 4.3h-R (`root_of` 7 lookups per object), 4.3h-W (double trim and early
  `origin` read), 4.3h-O (owned decode for read-only uses, touches `pkgs/access` / `pkgs/jstr`), 4.3h-Z (stamp `str`
  round trip through `adapter_probe`), 4.3h-T (`turns::answer` empty / full path), 4.3h-P (slot ordinals and stored
  plan ends), 4.3h-V (two ISO validators, `format_iso` zero fill), 4.3h-L (`one_line` per-char `substr`),
  4.3h-D (hand `dropped_user` multi-pass; hand candidate), 4.3h-A (`new_packet` zero fill), 4.3h-Q (`claude/framing`
  equals `qwen/framing`; a `pkgs` candidate, run both consumers when moved).
- Kept as rows from the step text: D-R08-3 (packet ABI, same widening as the hand), D-R08-2 with VL.
- Still open: U, K, L, E, O, OA, S, P-start, VE, PB, CMP, ENT, CK, DR, J, RA, SA, HK, LB, MP, OR, TM, VB, TH, FO, FV, UL, VP,
  IW, VL, 4.3d-M (also `last = copy(iso.text)` in `claude/import`), 4.3d-H, 4.3f-S, 4.3f-E, 4.3g-SQ.
- Next:
  1. Step 4.4 (`docs/RUST_SHAPE_RESET.md` §6).
  2. OpenCode bench rows 4.3g-P, 4.3g-R, 4.3g-Q, 4.3g-K, 4.3g-D, 4.3g-N, 4.3g-A, 4.3g-X, 4.3g-F, 4.3g-E (`opencode/`
     only; 4.3g-P first).
  3. Claude bench rows 4.3h-T, 4.3h-R, 4.3h-W, 4.3h-Z, 4.3h-P, 4.3h-V, 4.3h-L, 4.3h-A (`claude/` only), then 4.3h-O
     (shared packages; run all consumers) and 4.3h-Q (move `framing` to `pkgs` with the Qwen consumer).
  4. Grok bench rows 4.3c-T, 4.3c-B, 4.3c-G and `pkgs/turns` dead code; Qwen bench rows 4.3e-D, 4.3e-L, 4.3e-J,
     4.3e-S, then 4.3e-F (touches `lib/access`; run all consumers).
  5. Codex bench rows 4.3f-A, 4.3f-O, 4.3f-N, 4.3f-K, 4.3f-L, 4.3f-T, 4.3f-P, 4.3f-R, 4.3f-D (`codex/` only), then
     4.3f-B, 4.3f-E, 4.3f-F (shared modules; run all consumers).

## Step 4.4: Entry / req

- P10H gate (synchronous host facts; req fresh-query / order and timing contract): **not met**, so `req` stays
  blocked and its decisions were not ported. Evidence: `docs/L3_RESTRUCTURE_SPEC.md:232`, `:378` (P10H row), `:485`
  (req row 11), `:493` (grid), `:529` (R11), `:1245`, `:333` (LLVM pending P17b/P17c), `:1690` (verdict "blocked on
  the LLVM backend", `rq_find` rs/llvm 1.193x); `bench/l3/hostfact/README.md:41-44` ("The full request engine is not
  built"); `req/root.pruv:2-3`, `:54-58` (world / packet bundles, world seal on every bundle). `git log` has
  P16 / P17a (a7f7e68b) only.
- Entry recheck (fragment code outside `lib/ent_*`): `entry/root.pruv` (`entry_begin`, `entry_step`, `index_begin`,
  `index_step`) matches `lib/hand/entry_root.rs` function by function (same forwarders, same wire-in / poison check
  order). `entry/hand.rs` is the R02 serde_json workload lane, not a per-function counterpart (row ENT). No change, so
  no entry tests or `entry/integrate.py` run.
- `req`: new hand baseline `req/hand/req.rs` in the synchronous-fact shape (trait `Host`, `Requests { opening }` with
  `sources` / `list` / `open`, fresh `session_list` per lookup; `rustc --edition 2021 --test`, 19 tests pass, no warnings; `// Pruv:` names on every function) and
  `req/RESHAPE.md` with the per-function structure table (hand vs Pruv), the port left as rows. One Codex structure
  comparison (gpt-6.2-astra, high): five hand-side behavior differences fixed (Grok verify as the copy side's
  callback, no `rm` of an empty path, truthy seed account, missing vs null fields, `lastPlan` object), the rest kept as
  rows (`req/RESHAPE.md` §3b). No Pruv code changed.
- No module moved to `lib` / `pkgs`; no shared module changed, so no consumer runs were needed.
- Tests: `req/RESHAPE.md` §4 (Pruv unchanged; baseline run).

### Rows open after 4.4

- New (gate): P10H, req waits for (1) the LLVM G6 host-call boundary, P17b / P17c (`rq_find` rs/llvm 1.193x, needs
  <=1.0); (2) a natural hand request engine (`req/hand/req.rs` is the shape; it needs a perf lane) and the complete
  list / open x none / first / last match x S=1000/5000/20000, M=1/10/50 grid on both backends; (3) a synchronous-fact
  `RequestStore` in Pruv (G6 facts in place of the world / packet bundles) with the Q3 / Q7 fresh-observation controls
  and the 420k-call overhead check. Then the req port follows `req/RESHAPE.md` §1.
- New (bench, `req/RESHAPE.md` §3): REQ-PORT (the port itself, every row of §1), REQ-W (world / packet / adapter
  functions with no hand counterpart, removed by the port), REQ-K (Pruv releases the opening key before cleanup),
  REQ-D (`"default"` literal), and hand-model rows REQ-B, REQ-T, REQ-X, REQ-O, REQ-S, REQ-L.
- Still open: U, K, L, E, O, OA, S, P-start, VE, PB, CMP, ENT, CK, DR, J, RA, SA, HK, LB, MP, OR, TM, VB, TH, FO, FV, UL, VP,
  IW, VL, 4.3d-M, 4.3d-H, 4.3f-S, 4.3f-E, 4.3g-SQ, and the 4.3g / 4.3h bench rows above.
- Next:
  1. OpenCode bench rows 4.3g-P, 4.3g-R, 4.3g-Q, 4.3g-K, 4.3g-D, 4.3g-N, 4.3g-A, 4.3g-X, 4.3g-F, 4.3g-E (`opencode/`
     only; 4.3g-P first).
  2. Claude bench rows 4.3h-T, 4.3h-R, 4.3h-W, 4.3h-Z, 4.3h-P, 4.3h-V, 4.3h-L, 4.3h-A (`claude/` only), then 4.3h-O
     (shared packages; run all consumers) and 4.3h-Q (move `framing` to `pkgs` with the Qwen consumer).
  3. Grok bench rows 4.3c-T, 4.3c-B, 4.3c-G and `pkgs/turns` dead code; Qwen bench rows 4.3e-D, 4.3e-L, 4.3e-J,
     4.3e-S, then 4.3e-F (touches `lib/access`; run all consumers).
  4. Codex bench rows 4.3f-A, 4.3f-O, 4.3f-N, 4.3f-K, 4.3f-L, 4.3f-T, 4.3f-P, 4.3f-R, 4.3f-D (`codex/` only), then
     4.3f-B, 4.3f-E, 4.3f-F (shared modules; run all consumers).
  5. req port (REQ-PORT) once P10H passes on both backends.

## Step 4.4a: OpenCode bench rows (4.3g)

- `opencode/RESHAPE.md` §5. `opencode/` only; no `lib` / `pkgs` module changed, so no consumer runs were needed.
- Closed: 4.3g-P (every pool is one buffer grown `inout`; `cat_bytes`, `append_str`, `put_bytes`, `pool_*` removed),
  4.3g-Q (one SQL buffer, borrowed folder key), 4.3g-D (`enum DbPath`, `OPENCODE_DB` read once), 4.3g-A (result /
  error / step packets reserve + append), 4.3g-E (hand exec failure carries native / raw / name like the TS-judged
  Pruv form).
- Partly closed: 4.3g-K (`util::span_view -> ref [u8] from(t)` in `read_handoff`, `utility`, `select_pool`, `admit`,
  `p200`), 4.3g-F (`format_i64` / `pool_dec` removed, `items_at` / `rows_result` push).
- One Codex structure comparison (gpt-6.2-astra, high): hand raw code is now the errno symbol and a missing /
  unexecutable program fails at stage 16; `code_bytes`, `rm_step`, `path_step` borrow, `on_mkdtemp` writes the path
  once, `select_pool` mode 1 copies once. Rest kept as rows (4.3g-A3, K3, M07).
- Tests: `opencode/RESHAPE.md` §4 (verify A-D, 199/199 on both pairs, `q.py` 499 cases with 0 unexplained, 16/16
  mutants present, hand 0 warnings).

### Rows open after 4.4a

- New (language): 4.3g-BS, `arr_append` of a `str` into `[i64]` is a type error, so `packet::build_error` takes the
  name / detail as `[u8]` (literals pass as static views).
- New (bench, `opencode/RESHAPE.md` §5): 4.3g-A2 (`result_shape` twice), 4.3g-K2 (`rows::clean_title` /
  `updated_number` copies), 4.3g-F2 (`rows::format_ext` civil repeat), 4.3g-A3 (error / step packet header grown
  after a fixed buffer), 4.3g-K3 (`p100` / `db_path` / `utility` owned copies), 4.3g-M07 (identity order array kept as
  the mutant hook).
- Still open from 4.3g: 4.3g-R, 4.3g-N, 4.3g-X, 4.3g-T, 4.3g-SQ.
- Still open: U, K, L, E, O, OA, S, P-start, VE, PB, CMP, ENT, CK, DR, J, RA, SA, HK, LB, MP, OR, TM, VB, TH, FO, FV, UL, VP,
  IW, VL, 4.3d-M, 4.3d-H, 4.3f-S, 4.3f-E, the 4.3h bench rows and the 4.4 rows (P10H, REQ-*).
- Next:
  1. OpenCode bench rows 4.3g-R (with K2, F2), 4.3g-N, then 4.3g-X, 4.3g-T (`opencode/` only).
  2. Claude bench rows 4.3h-T, 4.3h-R, 4.3h-W, 4.3h-Z, 4.3h-P, 4.3h-V, 4.3h-L, 4.3h-A (`claude/` only), then 4.3h-O
     (shared packages; run all consumers) and 4.3h-Q (move `framing` to `pkgs` with the Qwen consumer).
  3. Grok bench rows 4.3c-T, 4.3c-B, 4.3c-G and `pkgs/turns` dead code; Qwen bench rows 4.3e-D, 4.3e-L, 4.3e-J,
     4.3e-S, then 4.3e-F (touches `lib/access`; run all consumers).
  4. Codex bench rows 4.3f-A, 4.3f-O, 4.3f-N, 4.3f-K, 4.3f-L, 4.3f-T, 4.3f-P, 4.3f-R, 4.3f-D (`codex/` only), then
     4.3f-B, 4.3f-E, 4.3f-F (shared modules; run all consumers).
  5. req port (REQ-PORT) once P10H passes on both backends.

## Step 4.4b: OpenCode bench rows (4.3g-R, K2, F2, N, X, T)

- `opencode/RESHAPE.md` §6. `opencode/` only (and its `test_impl.py` scope C now loads `path`); no `lib` / `pkgs`
  module changed, so no consumer runs were needed.
- Closed: 4.3g-R (`rows::decode` pushes into empty `rows` / `pool`, refs are text-bank offsets, `relocate` and the
  `p208` `lit` copy removed), 4.3g-K2 (`clean_title` / `updated_number` pass checked `subview`s to `str` parameters),
  4.3g-N (`path::normalize` in the hand shape: piece list, one result buffer), 4.3g-X (`export::join_prefix` removed,
  `start_export` calls `path::normalize`; `spawn_pool` reserve + append with `str` literals).
- Partly closed: 4.3g-F2 (`format_ext` reserve + push; civil repeat stays), 4.3g-T (dead `path::dirname` / `to_str`
  removed, `iso_str` moves its text).
- One Codex structure comparison (gpt-6.2-astra, high; Codex 2 account): path edge cases equal, no code change; findings
  kept as rows 4.4b-NV, 4.4b-NC, 4.4b-P208 and the check 4.4b-SP (spawn cwd / argv thread source vs the hand).
- Tests: `opencode/RESHAPE.md` §4 (verify A-D on both pairs, 199/199, `q.py` 499 cases with 0 unexplained, 16/16
  mutants present, hand 0 warnings).

### Rows open after 4.4b

- New (language): 4.4b-NV, a list of borrowed views (`Vec<ref [u8]>` / `Vec<&str>`) is a type error, so
  `path::normalize` keeps (start, end) offset pairs and pushes the root `/` first; a byte buffer built from `str`
  pieces loses the UTF-8 fact, so `path::join` checks `utf8_valid` before `str_from_utf8`.
- New (pkgs): 4.4b-CV, `pkgs/time` has no public `civil`, so `rows::format_ext` repeats the date arithmetic.
- New (bench): 4.3g-T2 (`entry::Attach` tape ids vs the hand's two owned strings per assistant message), 4.4b-NC
  (`normalize` reserves `n + 2`, hand sizes from the pieces), 4.4b-P208 (absent counts pushed after the move), 4.4b-SP
  (check spawn cwd / last argv source against the TypeScript reference; Pruv and hand read different fields).
- Still open from 4.3g / 4.4a: 4.3g-A2, 4.3g-A3, 4.3g-K3, 4.3g-M07, 4.3g-BS, 4.3g-SQ.
- Still open: U, K, L, E, O, OA, S, P-start, VE, PB, CMP, ENT, CK, DR, J, RA, SA, HK, LB, MP, OR, TM, VB, TH, FO, FV, UL, VP,
  IW, VL, 4.3d-M, 4.3d-H, 4.3f-S, 4.3f-E, the 4.3h bench rows and the 4.4 rows (P10H, REQ-*).
- Next:
  1. Claude bench rows 4.3h-T, 4.3h-R, 4.3h-W, 4.3h-Z, 4.3h-P, 4.3h-V, 4.3h-L, 4.3h-A (`claude/` only), then 4.3h-O
     (shared packages; run all consumers) and 4.3h-Q (move `framing` to `pkgs` with the Qwen consumer).
  2. Grok bench rows 4.3c-T, 4.3c-B, 4.3c-G and `pkgs/turns` dead code; Qwen bench rows 4.3e-D, 4.3e-L, 4.3e-J,
     4.3e-S, then 4.3e-F (touches `lib/access`; run all consumers).
  3. Codex bench rows 4.3f-A, 4.3f-O, 4.3f-N, 4.3f-K, 4.3f-L, 4.3f-T, 4.3f-P, 4.3f-R, 4.3f-D (`codex/` only), then
     4.3f-B, 4.3f-E, 4.3f-F (shared modules; run all consumers).
  4. OpenCode leftovers 4.4b-SP (check first), 4.3g-T2, 4.4b-NC, 4.3g-A2 / A3, 4.3g-K3 (`opencode/` only), and
     4.4b-CV with a `pkgs/time` `civil`.
  5. req port (REQ-PORT) once P10H passes on both backends.

## Step 4.4c: Claude bench rows (4.3h-T, R, W, Z, V, L, A, part of P)

- `claude/RESHAPE.md` §5. `claude/` only (`q_mutants.py` retargeted to the new code); no `lib` / `pkgs` module changed,
  so no consumer runs were needed.
- Closed: 4.3h-T (`answer` writes `completedAt` first and returns before any copy), 4.3h-R (`root_of` / `user_text_of`
  removed, keys read where the hand reads them), 4.3h-W (`is_human_text`: one trim, `task_origin` only after both
  tags), 4.3h-Z (stamps cross as borrowed `[u8]`, no `adapter_probe`), 4.3h-V (one `canonical_at`, `format_iso` from a
  template with `put_digits`), 4.3h-L (`one_line` in the hand loop shape, called only for a ranked title), 4.3h-A
  (packets reserve once and extend: header, turns, widened text; `text` and `turns` built apart; `write_bytes` /
  `put_str` / `write_turns` removed).
- Partly closed: 4.3h-P (`plan` no longer stores planned ends; the ordinal column itself stays).
- One Codex structure comparison (gpt-6.2-astra, high; Codex 2 account, the default was at its limit): no packet
  difference found; two new structure findings applied (tail text reserve, A3 single reserve); older findings kept
  as rows.
- Tests: `claude/RESHAPE.md` §5 (all roots verify under the 300 s cap, test_a0..a4 and `q.py --count 300` on both rs
  pairs, mutants all detected or proof-rejected).

### Rows open after 4.4c

- New (language): 4.4c-SV (a non-literal `str` cannot be lent as `[u8]`; `time::from_string` copies 24 bytes before
  the check, the A2 probe keeps `adapter_probe`), 4.4c-IS (`time::Iso` holds a heap `[u8]`; no fixed-size array value).
- New (bench): 4.4c-TS (`completedAt` moves 24 bytes per later answer, with VL), 4.4c-DC (`stamp_from` defects
  collapse to -4), 4.4c-TR (`from_string` owns the trimmed text), 4.4c-MB (model / title replaced, not reused),
  4.4c-OL (`one_line` reads the first byte twice).
- Still open from 4.3h: 4.3h-P (ordinal column), 4.3h-O, 4.3h-D (hand candidate), 4.3h-Q.
- Still open from 4.4b and before: 4.4b-NV, 4.4b-CV, 4.3g-T2, 4.4b-NC, 4.4b-P208, 4.4b-SP, 4.3g-A2, 4.3g-A3, 4.3g-K3,
  4.3g-M07, 4.3g-BS, 4.3g-SQ, U, K, L, E, O, OA, S, P-start, VE, PB, CMP, ENT, CK, DR, J, RA, SA, HK, LB, MP, OR, TM, VB,
  TH, FO, FV, UL, VP, IW, VL, 4.3d-M, 4.3d-H, 4.3f-S, 4.3f-E and the 4.4 rows (P10H, REQ-*).
- Next:
  1. Grok bench rows 4.3c-T, 4.3c-B, 4.3c-G and `pkgs/turns` dead code; Qwen bench rows 4.3e-D, 4.3e-L, 4.3e-J,
     4.3e-S, then 4.3e-F (touches `lib/access`; run all consumers).
  2. Codex bench rows 4.3f-A, 4.3f-O, 4.3f-N, 4.3f-K, 4.3f-L, 4.3f-T, 4.3f-P, 4.3f-R, 4.3f-D (`codex/` only), then
     4.3f-B, 4.3f-E, 4.3f-F (shared modules; run all consumers).
  3. OpenCode leftovers: 4.4b-SP first (check spawn cwd / argv against the TypeScript reference), then 4.3g-T2,
     4.4b-NC, 4.3g-A2 / A3, 4.3g-K3 (`opencode/` only), and 4.4b-CV with a `pkgs/time` `civil`.
  4. Claude leftovers: 4.3h-P (ordinal column, with the A3 snapshot), 4.4c-TS / 4.4c-MB / 4.4c-TR / 4.4c-OL, then 4.3h-O
     (shared packages; run all consumers) and 4.3h-Q (move `framing` to `pkgs` with the Qwen consumer).
  5. req port (REQ-PORT) once P10H passes on both backends.


## Step 4.4d: Grok rows 4.3c-T, B, G and Qwen rows 4.3e-S, L, part of D

- `grok/RESHAPE.md` §3e and `qwen/RESHAPE.md` §3b. Changed: `grok/entry.pruv`, `qwen/import.pruv`,
  `qwen/chain.pruv`, and two shared packages, `pkgs/transport` (`receive_grow`; dead `grow_line` removed) and
  `pkgs/turns` (dead `begin3`, `answer3`, `put_nl_span`, `put_pad`, `write_span` removed after a grep of every
  fragment). Every consumer of both packages was rerun.
- Closed: 4.3c-T (the stamp is resolved after the user filters, `at` / `at_view` lend a canonical stamp), 4.3c-G
  (over-cap piece fails before any copy), 4.3e-S (the ISO bytes go straight to the log), 4.3e-L (leaf chosen in
  `resolve` as `last_role.or(last_entry)`; 7-word slots).
- Partly closed: 4.3c-B (one `begin2` call site; rows 4.4d-CB and 4.4d-CL), 4.3e-D (answers and tools only for an
  attached turn; the scan still decodes every entry).
- One Codex structure comparison (gpt-6.2-astra, high; Codex 2 account, the default was at its limit). Applied: Grok
  reads `prompt_index` only in the no-query branch. Kept as rows: 4.4d-RC, PO, T0 (Grok), 4.4d-QS, QA, QC (Qwen).
- Tooling note: `/opt/homebrew/bin/timeout` is not installed on this machine; the 300 s cap used a perl `alarm`
  wrapper. qwen `q_mutants.py` has no `--check` (markers counted by hand; the full run was also done), and qwen
  `q.py` loops over the py pairs too (run with `PAIRS` set to the rs pairs).

### Rows open after 4.4d

- New (language): 4.4d-CB (no char-boundary fact from an adjacent ASCII byte).
- New (bench): 4.4d-CL (Grok content bound), 4.4d-LC (lazy line clear in `pkgs/transport`), 4.4d-TD (`turns`
  `finish2` / `finish_ref` / `empty_buffer` without callers, cited by comments), 4.4d-RC, 4.4d-PO, 4.4d-T0,
  4.4d-QS, 4.4d-QA, 4.4d-QC.
- Still open from 4.3e: 4.3e-D (scan-time decode), 4.3e-J and 4.3e-F (need a decode-into-buffer form in
  `pkgs/jstr` / `pkgs/access`; shared, all consumers).
- Still open from 4.4c and before: as listed after 4.4c (minus 4.3c-T, 4.3c-G, 4.3e-S, 4.3e-L).
- Next:
  1. Codex bench rows 4.3f-A, 4.3f-O, 4.3f-N, 4.3f-K, 4.3f-L, 4.3f-T, 4.3f-P, 4.3f-R, 4.3f-D (`codex/` only).
  2. 4.3f-B, 4.3f-E, 4.3f-F (shared modules; run all consumers).
  3. OpenCode leftovers: 4.4b-SP first (spawn cwd / argv checked against the TypeScript reference), then 4.3g-T2,
     4.4b-NC, 4.3g-A2 / A3, 4.3g-K3, 4.4b-CV.
  4. Then Qwen 4.3e-J / 4.3e-F with one decode-into-buffer form, 4.3e-D, and the 4.4d transport rows (LC, PO, RC)
     in one `pkgs/transport` change.


## Step 4.4e: Codex rows 4.3f-A, O, N, K, L, T, P, R, D

- `codex/RESHAPE.md` §3b. Changed: `codex/root.pruv`, `rpc.pruv`, `read.pruv`, `item.pruv`, `plan.pruv`,
  `folder.pruv`, `util.pruv` only; no `lib/` or `pkgs/` module changed, so no other fragment was rerun. The codex
  `q_mutants.py` targets that matched rewritten code were moved to the new lines (same faults).
- Closed: 4.3f-A (allocation after `start` and after the cwd check; one cwd decode; no request after stdin closed),
  4.3f-O (`read::absorb` returns `enum Absorbed`), 4.3f-N (`parse_line(&[u8])`), 4.3f-K (`kind_of` returns
  `enum Kind`, early on no `id`), 4.3f-L (one 300-unit walk for `has_phrase` and `error_detail`), 4.3f-T, 4.3f-R.
- Partly closed: 4.3f-P (`marker_end` returns 0; plan state still `pl` / `pb`), 4.3f-D (`str_at` uses
  `string_text_scanned_owned`; the length test stays, row 4.4e-DL).
- One Codex structure comparison (gpt-6.2-astra, high; Codex 2 account). It confirmed the 4.4e changes (`head` cut and
  case fold, phrase order, `kind_of`, `lone_at`, `marker_end`, page ids, `Full` before the cursor, `more` at 40). Its
  other findings are behavior that predates 4.4e or open shared rows; none was applied: 4.4e-LP (a lone surrogate or
  `1e400` in an unread member is accepted; the hand's serde rejects the line), 4.4e-EC (the pump cap counts `step`
  calls, the hand counts lines; the cap is also the `decreases` measure), 4.4e-NU (an argument with an inner NUL is
  split by the host), and 4.3f-B / E / F / P / D again.
- Tooling note: the whole codex root verify takes 285-330 s on this machine at load about 10 (the unchanged
  baseline also passed 300 s); the 300 s cap was met only with a warm proof cache (row 4.4e-VT).
- Results: codex `test_impl.py` 136/136 on `rs/rust` and `rs/llvm`; `q.py` (300 random + probes, rs pairs) same
  classes as 4.3f, unexplained 0; `q_mutants.py` 16/16 originals match once, the full run was stopped at the time
  cap (1 killed, 3 compile-wall timeouts, 12 not run; see `codex/RESHAPE.md` §4).

### Rows open after 4.4e

- New (bench): 4.4e-LP, 4.4e-EC, 4.4e-NU, 4.4e-FB (the `InputTooLarge` folder bound stays for the `req_list` size proof), 4.4e-DL (`str_at` keeps its test; the
  stated reason, no source bound, was wrong: closed in 4.4z), 4.4e-VT (codex verify time).
- Still open from 4.3f: 4.3f-P (`Option<Plan>`), 4.3f-B, 4.3f-E, 4.3f-F (shared modules), 4.3f-S (language).
- Still open from 4.4d and before: as listed after 4.4d (minus the closed 4.3f rows above).
- Next:
  1. OpenCode leftovers: 4.4b-SP first (spawn cwd / argv checked against the TypeScript reference), then 4.3g-T2,
     4.4b-NC, 4.3g-A2 / A3, 4.3g-K3, 4.4b-CV.
  2. Claude leftovers: 4.3h-P (ordinal column, with the A3 snapshot), 4.4c-TS / 4.4c-MB / 4.4c-TR / 4.4c-OL, then
     4.3h-O (shared packages; run all consumers) and 4.3h-Q.
  3. req port (REQ-PORT) once P10H passes on both backends.
  4. Then Codex 4.3f-B, 4.3f-E, 4.3f-F and Qwen 4.3e-J / 4.3e-F / 4.3e-D with the 4.4d transport rows (shared
     modules; run all consumers).

## Step 4.4f: OpenCode rows 4.4b-SP, 4.4b-NC, 4.3g-A3; 4.4b-NV after L-gaps 2

- `opencode/RESHAPE.md` §7. Changed: `opencode/path.pruv` (`normalize`), `opencode/packet.pruv` (`meta_cap`,
  `build_error`, `assemble_step`), `opencode/hand/opencode.rs`; no `lib/` or `pkgs/` module changed, so no other
  fragment was rerun.
- Closed: 4.4b-SP (the TypeScript spawns the export with `cwd: home` and the requested thread; Pruv already did, the
  hand used the selected row's directory / id and was fixed), 4.4b-NC (`normalize` sizes from the kept pieces),
  4.3g-A3 (error and step packets reserve the total on an empty array, then resize to the header).
- 4.4b-NV stays open as language rows: the L-gaps 2 local `Vec<ref str>` is LLVM `codegen/unsupported_type`
  (4.4f-NVL), and a `String` builder reaches `str` only through the copying `string_to_str` (4.4f-SS), so
  dropping `join`'s `utf8_valid` scan would cost a copy of the result. `RUST_PARITY_SPEC.md` §8's 4.4b-NV entry
  assumes a copy-free handover; not edited here.
- Results: opencode `test_impl.py` 199/199 on `rs/rust` and `rs/llvm`; scopes A-D VERIFIED; `q.py` (300 random +
  199 fixed, rs pairs) exact 406, unexplained 0, same classes as 4.4b; `q_mutants.py --check` 16/16. Hand
  `cargo build --release` 0 warnings.

### Rows open after 4.4f

- New (language): 4.4f-NVL (`[ref str]` locals on LLVM), 4.4f-SS (no copy-free `String` to `str` handover).
- New (bench): 4.4f-CW (`ctl[34..37]` written, no longer read; DESIGN wire words, with 4.3g-SQ).
- Still open in OpenCode: 4.4b-NV, 4.4b-P208, 4.4b-CV, 4.3g-T2, 4.3g-A2, 4.3g-K3, 4.3g-M07, 4.3g-BS, 4.3g-SQ.
- Still open from 4.4e and before: as listed after 4.4e.
- Next:
  1. Language: 4.4f-NVL / 4.4f-SS (or a `ref str` view of a checked byte span, 4.3f-S), then 4.4b-NV and 4.3g-K3.
  2. OpenCode: 4.3g-T2 (decide against the hand's per-message clones), 4.4b-CV with a `pkgs/time` `civil`.
  3. Claude leftovers: 4.3h-P, 4.4c-TS / 4.4c-MB / 4.4c-TR / 4.4c-OL, then 4.3h-O and 4.3h-Q.
  4. req port (REQ-PORT) once P10H passes on both backends.
  5. Then Codex 4.3f-B, 4.3f-E, 4.3f-F and Qwen 4.3e-J / 4.3e-F / 4.3e-D (shared modules; run all consumers).

## Step 4.4g: Claude rows 4.3h-P (rest), 4.4c-TR, 4.4c-OL

- `claude/RESHAPE.md` §6. Changed: `claude/turns.pruv`, `claude/import.pruv`, `claude/a3_root.pruv`,
  `claude/time.pruv`, `claude/text.pruv`; no `lib/` or `pkgs/` module changed, so no other fragment was rerun.
- Closed: 4.3h-P (the `sl[1600..1800)` ordinal column and its invariant are gone; the A3 snapshot exports
  `begun - 1`), 4.4c-TR (`from_string` borrows the trimmed span), 4.4c-OL (`one_line` takes one `char_step` per
  character).
- 4.4c-MB stays open as a `pkgs` row: no JSON string decode into an existing buffer or borrowed view in
  `pkgs/access` / `pkgs/jstr`. 4.4c-TS stays open with D-R08-2; a re-probe shows `Vec` of a struct with `[u8]` fields
  (local, struct field, `inout` + `vec_at_mut`) now compiles on LLVM, so VL looks closed and SPEC.md line 393 is stale.
- One Codex structure comparison (Codex 2 account) confirmed the ordinal removal, TR and OL; new bench rows 4.4g-DV
  (`digit_value` single pass vs the hand's check / strip / parse) and 4.4g-MC (model compared before decode).
- Results: claude `test_a0`-`test_a4` on `rs/rust` and `rs/llvm` all pass (272, 2884, 2443, 134, 270); `q.py` (300
  random + 270 fixed, rs pairs) 570/570, agrees with the oracle; `q_mutants.py` 18/18, 9 detected, 9 proof-rejected.

### Rows open after 4.4g

- New (bench): 4.4g-DV, 4.4g-MC.
- Changed: 4.4c-MB is now a `pkgs` row (decode into a reused buffer / borrowed view); VL is closed as a language row
  pending a SPEC.md fix and a runtime check, D-R08-2 / 4.4c-TS remain as the bench rewrite to `Vec<Draft>`.
- Still open in Claude: 4.4c-SV, 4.4c-IS, 4.4c-DC, 4.4c-TS, 4.4c-MB, 4.4g-DV, 4.4g-MC, 4.3h-O, 4.3h-D, 4.3h-Q, CK, D-R08-2, 4.3d-M.
- Still open from 4.4f and before: as listed after 4.4f (minus the Claude rows closed above).
- Next:
  1. Language: 4.4f-NVL / 4.4f-SS, then 4.4b-NV and 4.3g-K3; fix SPEC.md line 393 after a runtime test of `Vec` of a
     struct on LLVM.
  2. Claude: D-R08-2 / 4.4c-TS (`turns` as `Vec<Draft>` with a front index), then 4.3h-O and 4.4c-MB together (one
     `pkgs/access` borrowed-or-into-buffer string API; run all consumers), and 4.3h-Q.
  3. OpenCode: 4.3g-T2, 4.4b-CV with a `pkgs/time` `civil`.
  4. req port (REQ-PORT) once P10H passes on both backends.
  5. Then Codex 4.3f-B, 4.3f-E, 4.3f-F and Qwen 4.3e-J / 4.3e-F / 4.3e-D (shared modules; run all consumers).

## Step L-gaps 3: language rows 4.4f-NVL, 4.4f-SS, 4.3f-S, VL

- Compiler only (`pruvc-rs/`), SPEC.md and tests; no bench fragment changed, so no fragment was rerun.
- Closed: 4.4f-NVL (R-6g9-10: the local `Vec<ref str>` is a native LLVM vector of `(ptr, len, 0)` views; output and
  allocation balance match the Rust backend), 4.4f-SS (R-6e-23: `string_into_str(move s)` consumes the `String` and
  hands its buffer over with no copy and no `! alloc`; `string_to_str(s)` stays the borrowing copy), 4.3f-S (already
  done as R-6b-4, recorded), VL (SPEC.md line 393 fixed after a both-backend runtime test of a `Vec` of a plain struct
  taken read-only and returned, `tests/spec/lg3_vec_struct.pruv`).
- 4.4b-NV stays open (bench). The `path::normalize` rewrite to a view list was not made: the list has no `vec_pop`
  (the `..` rule pops), and `export.pruv` calls `normalize` on unvalidated config bytes, which a `Vec<ref str>`
  cannot hold (`Vec<ref [u8]>` is still "not yet"). A right-to-left pass with a pending-`..` counter needs no pop;
  `join` can then build a `String` from the views and end with `string_into_str` (no `utf8_valid` scan).

### Rows open after L-gaps 3

- New (language): L3-VB (`Vec<ref [u8]>` local view lists, both backends), L3-VP (`vec_pop` on a view list; optional
  once `normalize` scans right to left).
- Still open in OpenCode: 4.4b-NV and the rows listed after 4.4f.
- Still open from 4.4g and before: as listed after 4.4g (minus VL and the 4.4f language rows closed above).
- Next:
  1. OpenCode 4.4b-NV: `normalize_text(p: str) -> str` over a `Vec<ref str>` (right to left, pending-`..` count,
     pieces read back in reverse), `join` without `utf8_valid`; keep or port the byte caller in `export.pruv`
     (L3-VB); then `test_impl.py` on `rs/rust` and `rs/llvm` and `q.py`.
  2. Then the Next list after 4.4g (items 2-5).

## Step 4.4h: Claude D-R08-2 / 4.4c-TS (`turns` as `Vec<Draft>`)

- `claude/RESHAPE.md` §7. Changed: `claude/turns.pruv`, `claude/import.pruv`, `claude/a3_root.pruv`,
  `claude/q_mutants.py`, `claude/q_measure.py` (print only), `claude/handrust/src/lib.rs` (comments); no `lib/` or
  `pkgs/` module changed.
- Closed: D-R08-2 and 4.4c-TS. The collector is `Collector { drafts: Vec<Draft>, begun, active, lost }` with
  `Draft` owning its `[u8]` fields; `completedAt` is its own field; evicted drafts are reused in place at the front
  index `begun % 200`. `Vec` of a struct with `[u8]` fields ran on both backends (A3 134/134, A4 270/270 on
  `rs/llvm`); VL itself was closed in L-gaps 3.
- Different-model structure comparison run (`claude/RESHAPE.md` §7): new rows 4.4h-ST, 4.4h-CT, 4.4h-AE,
  4.4h-PL, 4.4h-PT, 4.4h-CU; no code change.
- Also closed: 4.4h-PC (the `begin` / `answer` / `tool` / `plan` per-field ensures restated over `c.drafts[k]`).
- Results: claude `test_a0`-`test_a4` on `rs/rust` and `rs/llvm` all pass; `q.py` 20270/20270 on both rs pairs;
  `q_mutants.py` 18/18, 9 detected, 9 proof-rejected (9, 11, 13, 17 back in proof; 18 now too).

### Rows open after 4.4h

- New: 4.4h-VD (language: no `VecDeque`), 4.4h-FA (language: no in-place append to a `[u8]` field), 4.4h-TR
  (language: `truncate` cannot take `move`; LLVM may shrink on truncate to 0).
- Still open in Claude: 4.4c-SV, 4.4c-IS, 4.4c-DC, 4.4c-MB, 4.4g-DV, 4.4g-MC, 4.3h-O, 4.3h-D, 4.3h-Q, CK, 4.3d-M,
  4.4h-VD, 4.4h-FA, 4.4h-TR, 4.4h-ST, 4.4h-CT, 4.4h-AE, 4.4h-PL, 4.4h-PT, 4.4h-CU; perf debt as in `claude/RESHAPE.md` §7.
- Still open from 4.4f and before: as listed after 4.4g.
- Next:
  1. Claude: a same-load before/after `q_measure.py` incl. `ring_wrap` and `depth_limit`, at the end of a later
     stage.
  2. Language: L3-VB / L3-VP (L-gaps 4), then 4.4h-VD / 4.4h-FA / 4.4h-TR; OpenCode 4.4b-NV and 4.3g-K3.
  3. Claude: 4.3h-O and 4.4c-MB together, and 4.3h-Q; then the rest of the 4.4g list (OpenCode, req port, Codex, Qwen).

## Step 4.4i: Claude 4.4c-MB (model), 4.4g-MC, 4.3h-Q, 4.4h-TR

- `pkgs/framing` (new package, `framing -> lines`): the byte-identical `claude/framing.pruv` and `qwen/framing.pruv`
  are one file; `limit_line_bytes` / `limit_drop_head` moved into it from both `boundary.pruv` (a package cannot
  name `crate::boundary`). Claude a0/a2/a4/q/root and Qwen q_root/root mount it; `claude/q_mutants.py` copies it to
  `work/qm/<id>/pkg_framing` for mutants 7 and 8.
- `pkgs/jstr::decode_append(inout out, ...)` and `pkgs/access::string_append(doc, id, inout out)`: decode into the
  caller's buffer (plain token = one `arr_append` of the span). `decode_utf8` / `string_text_owned` are now thin
  wrappers, unchanged contracts.
- Claude `read_window` keeps `model: [u8]` and refills it with `model = truncate(model, 0)` + `string_append` (hand
  `clear` + `extend_from_slice`); `consume_entry` drops an empty model like the hand. Closes 4.4c-MB for `model`
  and 4.4g-MC.
- 4.4h-TR closed: the copy form `truncate(x0, 0)` emits `truncate` + `shrink_to_fit` on rs/rust; `claude/turns.pruv`
  `refill` / `refill_text` now move into the local and truncate it in place (R-6g4-3).
- New 4.4i-UF (proof, `pkgs/parse` / `pkgs/lex`): no exported UTF-8 fact for a string token span, so a borrowed
  `ref str` / a `str` view of a reused `[u8]` cannot be formed without a run-time check. 4.3h-O and the title part of
  4.4c-MB stay on `string_text_owned`.
- After the Codex structure comparison: `entry::array_content` hands `joined` over with `string_into_str(move
  joined)` (R-6e-23) instead of a copy. New 4.4i-DP (pkgs/jstr, shape): an escaped token is decoded by a count pass,
  one reserve and a write pass; the hand decodes once while parsing.

### Bundle L-gaps 4 (2026-11-27, capped at 1 hour)

- L3-VB: done (R-6g9-11). `let mut parts: Vec<ref [u8]> = vec_new::<ref [u8]>();` is the view list with `[u8]`
  views on both backends (Rust `Vec<&[u8]>`, LLVM the same native `{ptr, len, 0}` vector); the borrow rules are
  R-6g9-10's, each operation's type argument must match the list's element, and the allocation balance matches Rust.
- L3-VP: done (R-6g9-11). `vec_pop::<ref T>(inout parts)` on a `str` or `[u8]` list drops the last view, requires
  `len(parts) >= 1`; Rust `{ parts.pop(); }`, LLVM stores the length less one.
- Still open: P10 `replace`/`swap` (not reached within the cap); 4.4b-NV (the `normalize` rewrite can now pop, and
  the byte caller in `export.pruv` can keep a `Vec<ref [u8]>`); the rows listed above.
- Audit (A: Codex gpt-6.2-astra high, spec and tests). Fixed: `vec_clear`/`vec_reserve` on an immutable view list
  were accepted (now `type/immutable`, test `lgaps4_immutable_clear`); the "other operation" message on a `[u8]` list
  named `ref str`; R-6g9-10's operation list lacked `vec_pop`. Advisory rows, tests not yet written:
  - LG4-T1: `[u8]` list `vec_clear`, `vec_get` out of range, `vec_reserve` with a negative size.
  - LG4-T2: `[u8]` pushes of a field, a view local and a `from` result; `borrow_escape` of a `[u8]` list.
  - LG4-T3: a `vec_get` view of a `[u8]` list live after the list's last use.
  - LG4-T4: `[u8]` list rejects: missing type argument, other `vec_*` op, other initialiser, parameter name, generic.
  - LG4-T5: `vec_pop` arity and a non-list argument; the exact length after a `[u8]` pop as an assert.
  - LG4-T6: an empty pop under `--no-verify` at runtime (the panic message on Rust, the check on LLVM).
  - LG4-D1: README does not document the `line` reject field.
- Audit (B: Claude opus high, codegen and LLVM): no required finding (length store, cap kept, verified pop unguarded,
  user `vec_pop` dispatch checked by reading code and IR). Advisory rows:
  - LG4-B1: the LLVM pop check follows `vec_get_checked` (also on with foreign functions or `--foreign-checks all`),
    Rust's follows `--no-verify` only; no wrong result (a verified pop never sees an empty list), but SPEC's "the
    same check unverified" is narrower than LLVM.
  - LG4-B2: an LLVM `--no-verify` pop failure calls `pruv_panic_requires` without freeing the list buffer (the
    `vec_get` failure path frees first; `vec_reserve` behaves like pop).
  - LG4-B3: mono.rs keeps targs for any call with a view targ; only `vec_new` in a `[u8]` Let reaches it, so it
    could test `x.name == "vec_new"`.
  - LG4-B4: tests not yet written: `[u8]` list allocation balance on loop early return / `break` / `?`; an
    `inout [u8]` parameter pushed, and `arr_push(inout src)` while the list is live; `vec_pop` in a contract;
    `Vec<ref str>` from `vec_new::<ref [u8]>()`; a `[u8]` list in a program where G3 mono runs.

## Step 4.4j: OpenCode rows 4.4b-CV, 4.3g-T2 (and 4.3g-F2)

- `pkgs/time`: new `pub struct Civil { year, month, day, rem }` and `pub fn civil(ms) -> Civil` (same requires as
  `format_iso`, ensures `0 <= rem < 86400000`), the hand's `civil` walk. `format_iso` calls it; its contract and
  digit writes are unchanged. `opencode/rows::format_ext` calls `crate::time::civil` instead of its own copy of the
  walk (closes 4.4b-CV and 4.3g-F2).
- `opencode/entry`: `Attach` holds `model: Model` with `enum Model { Absent, Pair(p: str, m: str) }` (rs/rust
  `Pair(String, String)`), set once per assistant message from `util::str_at` (moved, no second copy);
  `convert::model_bytes(model)` builds `p/m` with one reserve. Replaces the `pid`/`mid` tape indexes and the
  re-decode per user message (closes 4.3g-T2). The bound on the two lengths moves into `is_variant` /
  `variant_field` requires, ensures and the loop invariant.
- New 4.4j-TU (language): `civil` returns a named struct because tuple types are single-file only (P6-2), and
  `Option<(String, String)>` is a dedicated enum because an `Option` payload cannot be a struct holding `str`.
  Same layout and allocations as the hand. New 4.4j-ME (bench): `model_bytes` keeps an empty-side check the hand
  does not have. Details and the per-function table: `opencode/RESHAPE.md` §8.
- Consumers of `pkgs/time` (codex, grok, opencode, qwen) rerun on `rs/rust` and `rs/llvm`; results in
  `opencode/RESHAPE.md` §8. `qwen/q.py` hardwires the frozen py pairs and was not run (as in earlier steps).

## Step 4.4k: OpenCode row 4.4b-NV (attempted; blocked by a proof gap)

- `opencode/RESHAPE.md` §9. No `.pruv` file changed (`opencode/RESHAPE.md` and this file only); no `lib/` or `pkgs/`
  module changed, so no consumer was rerun.
- The hand-shaped `normalize_text(p: str) -> str` (local `Vec<ref str>`, `vec_pop` for `..`, a `String` builder,
  `string_into_str(move s)`) was written and fails verification only on the builder's length bounds
  (`string_push_char` / `string_push_str`, `len(s) + w <= 2^40`). The bound needs the sum of the list's element
  lengths, which no contract can state. A run-time guard or a narrower `requires` would verify it; both are
  approximations the hand does not have, so `path::normalize` / `join` and the `export` caller stay as they are.
  Draft and verifier output: `work/reset4_4k/` (not committed).
- Closed: none.

### Rows open after 4.4k

- New (language / proof): 4.4k-VS, no length sum over a view list (`vec_get` in a contract is
  `type/foreign_contract`; a list cannot be a parameter). Either `len(vec_get::<ref T>(l, k))` plus a sum in
  contracts, or a catalog join over a view list (hand `out.join(sep)`) with the total in its `ensures`.
- New (language): 4.4k-SR, no `String` reserve / with-capacity and no `String` insert at 0 (hand `join` allocates
  once and inserts the root `/`).
- Independent review (Codex `gpt-6.2-astra`): draft and hand agree on nine edge inputs; no SPEC feature proves the
  bounds, so 4.4k-VS stands. Option on 4.4k-VS (unverified): keep the offsets but build a `String` with
  `string_push_str(subview(..))` and `string_into_str`; drops `join`'s `utf8_valid` scan but still differs from the
  hand's piece list. See `opencode/RESHAPE.md` §9.
- Still open in OpenCode: 4.4b-NV (now blocked by 4.4k-VS), 4.4b-P208, 4.3g-A2, 4.3g-K3, 4.3g-M07, 4.3g-BS,
  4.3g-SQ, 4.4f-CW, 4.4j-TU, 4.4j-ME.
- Still open from 4.4j and before: as listed there.
- Next:
  1. Language: 4.4k-VS (view-list join or element lengths in contracts) and 4.4k-SR (`String` reserve), then 4.4b-NV
     with the draft in `opencode/RESHAPE.md` §9 and 4.3g-K3.
  2. Then the Next list after 4.4g (items 2-5).


### Bundle L-gaps 5 (2026-11-27, capped at 1 hour)

- P10 `replace`/`swap`: done (R-6g9-12). `swap(inout P, inout Q)` and `replace(inout P, e) -> T` on owned values
  and scalars; places are locals, field paths and `inout` parameters. Rust `::std::mem::swap` /
  `::std::mem::replace`, LLVM loads and stores (slots for locals and parameters, the storage form through the place
  pointer for fields). Proof: after `replace`, the result is `old(P)` and `P == e`; after `swap` the two values are
  exchanged. Overlapping places are `type/inout_alias`. The names are contextual: a user function named `swap` or
  `replace` is called as before (tests/spec/lgaps5_user_mem.pruv). An `inout` array parameter passed to either is a
  resizable parameter (`&mut Vec`). Tests tests/spec/lgaps5_*.pruv, allocation balance on both backends.
- Still open (not reached within the cap): 4.4h-FA (in-place append to a `[u8]` field of a local, `inout` or Vec
  element struct), LG4-B1 (vec_pop check wording vs. LLVM), LG4-B2 (LLVM `--no-verify` pop failure does not free the
  list buffer). P10-M1: a Vec element (`v[i]`) is not accepted as a `swap`/`replace` place.
- Audit (A: Codex gpt-6.2-astra high, spec and tests). Fixed: the verifier's `swap`/`replace` branch skipped the
  post-call range check of a refined local place (tests `lgaps5_swap_refine`, `lgaps5_replace_refine`); SPEC named
  `swap(inout s.f, inout s)` as an alias case (it is `type/inout_argument`, the types differ) and linked the
  same-field test as "a field and its struct"; added the `replace` value-reads-place reject (`lgaps5_replace_alias`).
  Advisory rows:
  - LG5-T1: reject tests for contract use, a type argument, a wrong arity, a non-place, a missing `inout`, a
    read-only parameter, `inout` on the `replace` value, a `replace` value type mismatch, `borrow_signature`, an owned
    local without `move` / reused after it.
  - LG5-T2: accept tests for a field of an `inout` struct parameter, nested fields, two fields of one struct, `[i64]`
    and `Vec<T>` places, and the bytes (not only lengths) after a swap.
- Audit (B: Claude opus high, codegen and LLVM; it could not build or run, findings from code reading). Reproduced
  and fixed on LLVM: the old value of `replace` used in place (`len(replace(..))`, a borrowed argument) was never
  freed, and an old struct still pointed at the overwritten place, so a callee read the new value (Rust `3242`,
  LLVM `3292`, 5 allocations / 2 frees; test `lgaps5_mem_use`). The old value is now detached from the place and
  dropped with the statement's temporaries. Now rejected by the checker (LLVM emitted wrong IR or failed inside):
  a fixed-length array place (`lgaps5_replace_fixed`), an `inout` `Vec` / `HashMap` parameter or a map binder as a
  place (`lgaps5_swap_vec_param`). Advisory rows:
  - LG5-B1: a fixed-length array place (Rust could take it; LLVM needs the storage form for `[T; N]` slots).
  - LG5-B2: handle types (`Process`, `Socket`, `File`, `Loop`) and `Option<Struct>` as places are not tested.
  - LG5-B3: tests for a boxed field path, cost mode, and a user `fn swap(inout a: [i64], i: i64, j: i64)`.
  - LG5-B4: `mem_builtin` reads `prog.functions` while the backends read `fmap`/`funcs`; not shown to differ.

## Step 4.4l: 4.4i-UF (UTF-8 fact for string spans), 4.4i-DP (one-pass decode)

- Changed packages: `pkgs/lex`, `pkgs/parse`, `pkgs/json`, `pkgs/lines` (contracts and two new preds) and
  `pkgs/jstr` (code). Details, the per-function structure table and the consumer tests: `pkgs/RESHAPE.md` §Step 4.4l.
- 4.4i-UF closed: `lex::scan_string` ensures its content span is valid UTF-8, `parse` carries it as
  `parse::strings_ok`, and `parse::parse` / `lines::parse_owned` / `parse_copy` / `parse_range` / `build_span` ensure
  `json::doc_strings(document)` for `Parsed`. A `JsonDocument` invariant was tried first and dropped: re-proving the
  quantified fact at each call in a loop times out in `access::find_ascii` (access.pruv:104).
- 4.4i-DP closed: `jstr::decode_append` decodes in one pass with no count pass and no reserve.
- Independent structure comparison (Codex `gpt-6.2-astra`, review role): every changed function still differs from
  the hand; new shape rows 4.4l-RE, 4.4l-VU, 4.4l-SS, 4.4l-RS, 4.4l-CT (`pkgs/RESHAPE.md`).
- Not done (cap): 4.3h-O and the title part of 4.4c-MB. Both are unblocked: a consumer that requires
  `doc_strings(doc)` can take a `str` view of a plain span.

### Rows open after 4.4l

- New (shape, `pkgs`): 4.4l-RE, 4.4l-VU, 4.4l-SS, 4.4l-RS, 4.4l-CT.
- Still open in Claude: 4.3h-O, 4.4c-MB (title), and the rest as listed after 4.4i.
- Still open from 4.4k and before: as listed there.
- Next:
  1. Claude 4.3h-O and the 4.4c-MB title on `doc_strings` (`pkgs/access` / `pkgs/jstr` borrow a plain span; rerun
     every consumer of the changed packages).
  2. 4.4l-SS (drop the per-scalar recheck in `scan_string` where the line was validated first), then 4.4l-RE / -RS /
     -CT / -VU.
  3. Then the Next list after 4.4k.


### Bundle L-gaps 6

- Closed 4.4k-VS (R-6g9-13): `vec_views_len::<ref T>(parts, k)`, the total byte length of the first `k` views of a
  view list, contract-only. The verifier keeps a running sum per list (`vec_push` adds, `vec_pop` subtracts,
  `vec_clear` resets), so a builder that copies every view proves its length bound without a run-time guard. Tests
  `lgaps6_views_len` (path normalizer), `lgaps6_views_len_bytes`; rejects `lgaps6_views_len_code`,
  `lgaps6_views_len_wrong`, `lgaps6_views_len_targ`.
- Closed 4.4k-SR (R-6g9-14): catalog `string_with_capacity(n)`, `string_reserve(s, n)`, `string_prepend_char(s, c)`
  on both backends (Rust `String::with_capacity` / `reserve` / `insert(0, c)`, G6 shim adapters on LLVM). The
  verifier proves `0 <= n <= MAX_LEN` and keeps the bytes (prepend: encoding at 0..w, old byte k at w+k). Tests
  `lgaps6_str_reserve`; rejects `lgaps6_str_prepend_wrong`, `lgaps6_str_cap_range`.
- Closed 4.4h-FA (R-6g9-15): `arr_append` / `arr_push` on the `inout d.f` field of a `vec_at_mut` binder already
  worked through R-6g-3; the row documents it with frame ensures. Tests `lgaps6_field_append`; reject
  `lgaps6_field_append_frame`.
- Closed LG4-B1: R-6g9-11 now says the LLVM `vec_pop` check runs whenever the `vec_get` bounds check is on.
- Closed LG4-B2: the LLVM view-list `vec_pop` and `vec_reserve` failure blocks now free live temporaries and scope
  owners before `pruv_panic_requires` (`Fail::RequiresFreed`). Test `lgaps6_pop_free` (unverified empty pop, exit
  101, balanced allocations).
- New rows:
  - LG6-VJ: no catalog join of a view list (`out.join("/")`); the normalizer still builds the result byte by byte.
  - LG6-FR: a run-time `requires` over a Vec element field is `codegen/unsupported_type` under `--no-verify`, so
    `lgaps6_field_append` runs verified only.
  - LG6-SB: the bytes after a call whose `ensures` names only the length are unknown (as for any function); the
    SR test inlines the reserve/prepend steps to check bytes.
- Still open: P10-M1 (a Vec element `v[i]` as a `swap`/`replace` place) was not reached within the cap.
- Next: P10-M1, LG6-VJ.
- Audit (A: Codex gpt-6.2-astra high, spec and tests; B: Claude opus high, codegen and LLVM). Fixed: the
  `catalog_shapes` unit test count (108 to 111); R-6g9-13's recurrence now says `0 <= k < len(parts)`, and a `k`
  outside `0..=len(parts)` is unspecified (after `vec_pop` the verifier still relates it to the popped views, so
  "unconstrained" overstated it); the `lgaps6_pop_free` comment no longer claims a stack array is freed.
  No wrong code or proof gap was found in the new string ops, `Fail::RequiresFreed` or the view sums. Advisory rows:
  - LG6-T1: `lgaps6_field_append` reads only lengths and the last pushed byte; check the appended bytes and the
    untouched elements' bytes.
  - LG6-T2: R-6g9-14 rejects: capacity above `2^40`, both `string_reserve` bounds, an invalid scalar and
    a too-long result for `string_prepend_char`, a wrong tail-shift claim; runs of 3/4-byte prepends and
    `--no-verify` run-time requires failures (exit 101) on both backends.
  - LG6-T3: `lgaps6_pop_free` exercises only the list buffer; add a heap temporary / scope owner and the
    view-list `vec_reserve` failure path, and the verified `--foreign-checks all` pop check.
  - LG6-T4: `vec_views_len` prefix sums at a middle `k` across `vec_push` are not related (only the total is).
  - LG6-B1: `--foreign-checks all` run-time ensures of `string_prepend_char` (and `string_push_char`) check the
    length and the tail, not the encoded prefix bytes.
  - LG6-B2 (unverified guess): `string_with_capacity(2^40)` passes the proof but the allocation would abort rather
    than fail with exit 101 (same as `vec_reserve`).

## Step 4.4m: borrowed plain strings for Claude replies (4.3h-O, part), 4.4l-SS restated

- Changed: `pkgs/access` (new `plain_str`, a borrowed `Option<ref str>` of a plain string node under
  `json::doc_strings`), `claude/entry` (`parse_line` ensures `doc_strings`), `claude/import` (`answer_blocks` and
  `consume_entry` pass `plain_str`'s view to `turns::answer`, owned decode otherwise). No compiler change. Details,
  the per-function structure table and the consumer tests: `pkgs/RESHAPE.md` §Step 4.4m.
- Independent structure comparison (Codex `gpt-6.2-astra`, review role, Codex 2 account) found that the change does
  not take effect: `parse::record` writes the plain bit 0 for every string, so `plain_str` is always `None` on the
  Claude path. Confirmed in parse.pruv:263. Row 4.4m-PB.
- 4.4l-SS restated as 4.4m-SS: the hand also runs `str::from_utf8` on every value chunk (handrust lib.rs:507-508);
  the difference is per-scalar checks (values and keys) vs per-value checks, not an extra pass.
- Not measured (structure not identical).

### Rows after 4.4m

- Closed: 4.4l-SS (replaced by 4.4m-SS). 4.3h-O stays open: its consumer side is in place, its producer is 4.4m-PB.
- New: 4.4m-PB (producer), 4.4m-TB (title), 4.4m-FB (`ref str` enum payload, R-6g-11), 4.4m-SS.
- Still open: 4.4l-RE, 4.4l-VU, 4.4l-RS, 4.4l-CT, 4.4c-MB (title), and the rows listed after 4.4l and before.
- Next:
  1. 4.4m-PB: `record` sets the plain bit (via `string_token`), widen the slot-5 facts in `parse` to the form
     `check_tape` accepts, rerun every consumer of `pkgs/parse`; then check that the Claude reply path is `Some`.
  2. 4.4m-TB part one (blank check on the borrowed view, copy only the winner).
  3. 4.4m-SS, then 4.4l-RE / -RS / -CT / -VU, then the Next list after 4.4k.

### Bundle L-gaps 7

- Closed LG6-VJ (R-6g9-16): catalog `vec_join::<ref T>(parts, sep)` on a `str` / `[u8]` view list (hand
  `parts.join(sep)`), result `String` / `[u8]` with `! alloc`, on both backends. The verifier gives the result
  length from the R-6g9-13 view sum plus `(n - 1) * len(sep)` and every byte (piece `k` starts at
  `S(k) + k * len(sep)`, the separator follows it when `k < n - 1`); it requires the total to be at most `2^40`.
  A `String` result has no run-time UTF-8 scan on Rust (LLVM still checks each piece, LG7-B2). Rust emits `parts.join(&sep[..])`; LLVM sums the lengths, allocates
  once (or `string_with_capacity` through the G6 shim) and copies views and separators. The path normalizer now
  builds with `vec_join::<ref str>(out, "/")` (`lgaps7_join`, emits `out.join(&("/")[..])`). Tests
  `lgaps7_join_bytes`, `lgaps7_join`; rejects `lgaps7_join_len_wrong`, `lgaps7_join_byte_wrong`.
- Fixed: `lgaps6_pop_free` failed `spec_trace` (an `accept` test that only compiles unverified); its JSON now has
  top-level `"flags": ["--no-verify"]`.
- New rows:
  - LG7-SR: a wrong length or byte claim about a join of `subview` pieces is refuted only after the verifier
    timeout (about 120 s, `verify_unknown`) instead of a quick `assert` counterexample; the reject tests use string
    literal pieces. The recursive view sum with symbolic piece lengths is the likely cause.
  - LG7-AB: the LLVM `alloc_balance` counter does not see buffers that the G6 shim allocates and the module frees
    (`string_into_str`), so `lgaps7_join_bytes` checks `g6_drop_balance` instead of `alloc_balance`.
- Closed LG6-T3 (most of it): `lgaps7_pop_free_heap` (unverified empty `vec_pop` with a heap array and an owned
  `[u8]` of an enclosing block live) and `lgaps7_reserve_free` (unverified `vec_reserve` with `n < 0` on a `[u8]`
  list) exit 101 with balanced allocations on both backends. Left as LG7-T3: the verified `--foreign-checks all`
  pop check.
- Still open: P10-M1 (a Vec element `v[i]` as a `swap`/`replace` place): it needs index places in the checker
  (with an alias rule or a proven `i != j`), element stores in the verifier, and `v.swap(i, j)` on Rust; not
  reached within the cap.
- Audit (split: A = Codex gpt-6.2-astra on SPEC/encoding/tests, B = Claude opus high on codegen). Fixed the two
  required findings: the separator was checked as a plain value, so a `subview` or `[]` separator was rejected (now
  a view argument; `lgaps7_join_bytes` joins with both), and a program's own `vec_join` function let a view list
  escape (now `type/borrow_escape`, reject `lgaps7_join_user`). Advisory rows:
  - LG7-A1: no test catches a missing or off-by-one `2^40` bound (a `2^39` view joined twice with `[]` should
    pass; a 1-byte separator should fail `call_requires` on the call line).
  - LG7-A2: no test shows a join's bytes survive a later list change (`vec_clear`, then a push).
  - LG7-B1: with `--no-verify` the LLVM total `S + (n - 1) * L` can wrap in `i64` before the bound check; use
    separate checks or overflow intrinsics.
  - LG7-B2: the LLVM `string_push_str` shim checks each `str` piece with `from_utf8`; an unchecked adapter would
    match the Rust backend.
  - LG7-B3: the LLVM `[u8]` result is zero-filled before the copies overwrite every byte.
  - LG7-B4: the bound check runs on `!verified`, unlike `vec_pop` which keys on `vec_get_checked`; align them.
  - LG7-B5: no test uses a join result without a `let` (as a call argument or in an expression).
- Next: P10-M1, LG7-SR, LG7-T3, LG7-A1, LG7-B1.

## Step 4.4n: plain bit on the parse path (4.4m-PB), title blank check on the borrow (4.4m-TB part one)

- Changed: `pkgs/parse` (`record` takes the plain bit from `string_token`; `string_token` ensures
  `utf8_valid_span` of the token body; the slot-5 facts widened to the form `check_tape` accepts) and
  `claude/entry` (`title_key` blank-checks `plain_str`'s borrowed view and copies only a nonblank candidate;
  `pick_title` / `ranked` / `title_of` / `title_for_type` require `json::doc_strings`), `claude/import`
  (`read_window`'s per-line body split into `window_object` / `window_model`, both `#[inline(always)]`, because the
  solver gave up on the `doc_strings` precondition inside the loop). Test changes: the
  `json/run_a2.py` oracle expects the plain bit; `claude/test_a1.py` adds 13 `plain` cases through
  `a1_root.pruv::plain_w`. No compiler change. Details, the structure table and the consumer tests:
  `pkgs/RESHAPE.md` §Step 4.4n.
- `plain_str` is now `Some` for every unescaped string on `parse_line` documents, on both backends (`test_a1`
  `plain` cases). So the 4.4m consumer side (`answer_blocks`, `consume_entry`) takes the borrowed branch.
- Not measured (structure not identical: 4.4m-TB part two, 4.4m-SS, 4.4m-FB).
- Independent structure comparison (Codex `gpt-6.2-astra`, review role, Codex 2 account; the default account was
  at its limit). It agrees with the table: one pass per string token, and a borrowed blank check with one copy
  per winning title on both sides. It found two more per-call differences: rows 4.4n-TT and 4.4n-KL.

### Rows after 4.4n

- Closed: 4.4m-PB, 4.4m-TB part one.
- New: 4.4n-TT (`title_for_type` reads `type` up to three times), 4.4n-KL (each key lookup walks all members).
- Still open: 4.4m-TB part two (reused title buffer), 4.4m-SS, 4.4m-FB, 4.3h-T, 4.4l-RE, 4.4l-VU, 4.4l-RS,
  4.4l-CT, 4.4c-MB, and the rows listed after 4.4l and before.
- Next:
  1. 4.4m-SS: a "line validated" fact through `record` / `container` / `scan_records` / `scan`, so that
     `string_token` skips `validate_utf8` under `parse_owned`.
  2. 4.4n-TT: read `type` once through `plain_str`.
  3. 4.4m-TB part two (a `str` buffer whose `clear` + append keeps valid UTF-8), then 4.4n-KL (design first),
     then 4.4l-RE / -RS / -CT / -VU.

## Step 4.4o

- Closed 4.4b-NV (with 4.4k-VS and 4.4k-SR): OpenCode `path::normalize` is the hand shape on `str` (`Vec<ref str>`
  pieces, `pop` on `..`, `vec_join(out, "/")`, `string_prepend_char` for the root) and proves the old contracts with
  no run-time guard; `join` follows the hand arms (`with_capacity` + pushes for the two-sided case, no UTF-8 scan).
  No compiler change. Details, the per-function structure table and the tests: `opencode/RESHAPE.md` §10.
- 4.3g-K3 partly closed: `env_value`, `data_dir` / `db_path` home and the `p100` thread borrow the configuration text
  through `util::cfg_view` (`ref str from(cfg_text)`); `cfg_str` copies once instead of twice.
- Behaviour: `normalize("./")` is now `"./"` (hand and Node); the old byte form gave `"."`.
- Independent structure comparison (Codex `gpt-6.2-astra`, review role, Codex 2 account): `normalize` / `join` equal
  on 17 traced inputs; the borrowed readers differ by the per-read UTF-8 scan; `p100` path bound differs (pre-existing).
- Tests: `test_impl.py` 199/199 and `--verify-only` VERIFIED on rs/rust and rs/llvm; `q.py --cases 300` exact 406,
  0 unexplained on both; `q_mutants.py --check` 16/16 markers.
- Not measured (structure not identical).

### Rows after 4.4o

- Closed: 4.4b-NV, 4.4k-VS, 4.4k-SR. Partly: 4.3g-K3.
- New: 4.4o-SB (no `str` → `[u8]` move; `utility` mode 3 and `p100` copy to call `dirname_bytes`), 4.4o-EB
  (`export` root stays bytes), 4.4o-CV (per-read UTF-8 scan of config spans), 4.4o-PL (`p100` path bound),
  4.4o-VP (verified `vec_push` of a view keeps the `2^40` check).
- Next:
  1. 4.4o-CV: validate the configuration text once and keep a `ref str` of it, then drop the per-read scans.
  2. 4.4o-SB with 4.3g-T (a `str` `dirname` returning a view, no pool copy in `p100`).
  3. 4.4o-EB, 4.4o-PL, 4.4o-VP, then the Next list after 4.4m.

## Step 4.4p: one line check on the record path (4.4m-SS), `type` read once in the title path (4.4n-TT)

- Changed: `pkgs/parse` (`string_token` takes `checked` and skips bytes `>= 128` without a per-scalar check when
  the whole input is known valid; `record` / `container` / `scan_records` / `scan` / `scan_document` require
  `utf8_valid_span(input, 0, len(input))`; `parse` validates the line first, like the hand `parse_line`; `scan`
  needed an `assert` before its `match` because the solver gave up on its ensures otherwise) and `claude/entry`
  (`title_for_type` reads `type` once through `plain_str` and compares the view; `string_is` stays only for an
  escaped or non-plain `type`). No compiler change, no test change. Details, the structure table and the consumer
  tests: `pkgs/RESHAPE.md` §Step 4.4p.
- Not measured (structure not identical: 4.4m-TB part two, 4.4p-CF, 4.4n-KL).
- Independent structure comparison (Codex `gpt-6.2-astra`, review role, Codex 2 account; the default account was
  at its limit). It agrees with the table: one line check and zero per-scalar checks on the record path, and an
  unescaped `type` borrowed once and compared at most once. The `checked` constant branch is row 4.4p-CF; the other
  differences it listed (owned source copy, `check_subtrees`, A2 limits) are earlier rows.

### Rows after 4.4p

- Closed: 4.4m-SS, 4.4n-TT.
- New: 4.4p-CF (`string_token`'s `checked` is a run-time flag, not two specialized scanners).
- Still open: 4.4m-TB part two (reused title buffer), 4.4n-KL, 4.4m-FB, 4.3h-T, 4.4l-RE, 4.4l-VU, 4.4l-RS,
  4.4l-CT, 4.4c-MB, and the rows listed after 4.4n and before.
- Next:
  1. 4.4m-TB part two: a `str` buffer whose `clear` + append keeps valid UTF-8, reused like the hand's one buffer.
  2. 4.4n-KL (design first), 4.4p-CF.
  3. 4.4l-RE / -RS / -CT / -VU.

## Step 4.4q: config text checked once (4.4o-CV), borrowed `str` dirname in `p100` (4.4o-SB part, 4.3g-T part), `p100` path bound (4.4o-PL)

- Changed: `opencode/util` (`cfg_view` requires `utf8_valid(t)` and keeps two char-boundary byte tests instead of
  the span scan; the old body is the private `span_text` behind `cfg_str`), `opencode/path` (`dirname(p: str) ->
  ref str from(p)` replaces `dirname_bytes`; `env_value` / `data_dir` / `db_path` require `utf8_valid(cfg_text)`;
  `utility` mode 3 checks the text once) and `opencode/admit` (`p100` checks the text once, borrows the parent of the
  owned DB path, builds the pool with one reserve and two appends, and bounds the path by `> 1e9` first). No
  compiler change. Details, the structure table and the tests: `opencode/RESHAPE.md` §11.
- `Attach` already holds what the hand holds (`open: bool` + `Model::Absent | Pair(str, str)`); not changed.
- Tests: `test_impl.py --jobs 2` 199/199 and `--verify-only` VERIFIED on rs/rust and rs/llvm; `q.py --cases 300`
  exact 406, 0 unexplained on both; `q_mutants.py --check` 16/16 markers.
- Not measured (structure not identical: `dirname` loop vs `trim` / `rfind`, the step-wire pool, `utility` mode 3
  `to_bytes`).
- Independent structure comparison (Codex `gpt-6.2-astra`, review role, Codex 2 account; the default account was
  at its limit). It agrees on zero scans after the one check and the borrowed `dirname`; it found the value
  differences recorded as 4.4q-CU and 4.4q-DN.

### Rows after 4.4q

- Closed: 4.4o-CV, 4.4o-PL. Partly: 4.4o-SB (`p100` closed; `utility` mode 3 `to_bytes` stays, language), 4.3g-T
  (`dirname` borrowed, `Attach` matches; the request pool is the step wire), 4.3g-K3 (the `p100` copy is gone).
- New: 4.4q-CU (a config text not valid UTF-8 as a whole is `(3,6)` where the hand decodes lossily; a span cut inside
  a character reads `""`; unreachable from the host), 4.4q-DN (`dirname` on repeated slashes: Pruv and Node give
  `"//"` / `"a/"` for `"//a"` / `"a//b"`, the hand gives `""` / `"a"`).
- Still open: 4.4o-EB (blocked by 4.4o-SB), 4.4o-VP, 4.4p-CF, 4.4m-TB part two, 4.4n-KL, and the rows listed before.
- Next:
  1. 4.4o-SB language side: a `str` → `[u8]` move (or a byte view of a `str` argument), then 4.4o-EB and `utility`
     mode 3.
  2. 4.4q-DN: decide which side is the reference for the hand `dirname` (the pinned TypeScript says Node).
  3. 4.4o-VP, then the Next list after 4.4p.

### Bundle L-gaps 8

- Closed P10-M1 (R-6g9-12): an element of a resizable array is a `swap` / `replace` place:
  `swap(inout v[i], inout v[j])` and `replace(inout v[i], e)` with `v` a `let mut` local or an `inout` parameter.
  The checker takes plain indexes that do not mention `v`, both `swap` elements from one array, no other mention of
  `v` in the call (`type/inout_context`), and reports a live view of `v` as `type/borrow_conflict`. The verifier
  stores into the array: each index carries the range obligation (`verify/index_oob`), the two `swap` indexes must
  be proven different (`verify/inout_alias`), and a narrow `replace` value carries the element range
  (`verify/elem_range`); loops havoc the array. Rust emits `v.swap(i, j)` and
  `::std::mem::replace(unsafe { v.get_unchecked_mut(i) }, x)` when verified, index-checked helpers with
  `--no-verify`; LLVM loads and stores the elements in their storage form. Test `lgaps8_mem_index` (`[i64]`, `[u8]`,
  `[bool]`, an `inout` parameter in a loop) runs on both backends with and without `--no-verify`; rejects
  `lgaps8_swap_index_alias`, `lgaps8_replace_index_oob`, `lgaps8_swap_index_oob`, `lgaps8_replace_index_elem`,
  `lgaps8_replace_index_view`, `lgaps8_swap_index_arrays`.
- Closed LG7-B1: with `--no-verify`, the LLVM `vec_join` running sum saturates just above `2^40` and
  `(n - 1) * len(sep)` and the total use `llvm.smul` / `llvm.sadd.with.overflow`, so a wrapped total cannot pass the
  bound check (verified IR unchanged).
- Closed LG7-A1: `lgaps8_join_bound` (a `2^39` view joined twice with an empty separator is accepted) and
  `lgaps8_join_bound_sep` (a 1-byte separator fails `verify/call_requires` on the call line).
- LG7-SR (open, diagnosed): the slow refutation is not in `vec_join`. Two `subview`s of a parameter array pushed
  onto a view list, then a false `assert len(out) == 3`, already end in `verify_unknown` (`incomplete (theory
  array)` or the timeout). The view sum `S(a, k)` takes the list array, whose elements hold `subview` lambdas over
  the parameter array, and z3 cannot complete a model; a local `alloc` base works because its lambda folds to a
  constant array. Tried and reverted: a per-join `Int -> Int` sum, and a fresh array name per pushed view with a
  quantified byte fact (faster, still incomplete). Next: key the view sum on something other than the list array
  term (a per-list-version `Int` function, or a length-only ghost list). The must_reject test with `subview`
  pieces waits for that fix.
- New rows:
  - LG8-B1T: no run-time test drives the `--no-verify` LLVM wrap path (it needs views whose lengths sum past
    `2^63`); only the IR shape was checked.
- Audit (one round, A: Codex gpt-6.2-astra on SPEC/encoding/tests; B: Claude opus high on codegen). Fixed:
  `replace(inout v[i], x)` on a refined element array (`[i64 | lo..=hi]`) was accepted out of range; it now
  proves the stored element's range (`verify/refinement_assign`, test `lgaps8_replace_index_refine`). SPEC now
  names the concrete element array types (generic `[T]` is rejected), `type/immutable`, and the
  `inout_alias` / `inout_context` split; `verify/inout_alias` was added to the code table. B found nothing required.
- Advisory rows (from the audit):
  - LG8-A1: no `--no-verify` run-time failure tests for element places (index out of range, `[u8]` value 256),
    so `pruv_oob` vs `Fail::Oob` and `pruv_elem_oor` vs `Fail::Elem` output parity is unchecked.
  - LG8-A2: no `[u16]` element place test (65535 accepted / 65536 rejected), and no `[u8]` / `[bool]` `inout`
    parameter element place.
  - LG8-A3: missing rejects: a wrong fact after a swap loop, equal literal indexes, an index that reads the
    array, an index with a call, the first `swap` index out of range.
  - LG8-A4: Rust `--no-verify` `vec_join` checks the 2^40 total after joining (pre-existing); LLVM stops before
    allocating. Check a saturating `u64` total first in Rust too.
  - LG8-A5: Rust verified narrow element replace relies on the storage pin, not `self.layout`; LLVM swap and
    replace choose the element shape in different orders (same result today).
- Not reached: LG7-T3 (verified `--foreign-checks all` pop check).
- Next: LG7-SR, LG7-T3, LG7-A2, LG7-B2..B5.

## Step 4.4r: one reused title buffer (4.4m-TB part two), `string_token`'s constant flag folds (4.4p-CF), key lookup design (4.4n-KL)

- Changed: `claude/entry` (`RankedTitle { rank: i64, text: [u8] }` and `new_title()`; `title_key` / `pick_title` /
  `ranked` / `title_of` / `title_for_type` write the winning title into the caller's buffer by `truncate(0)` + a `str`
  append, return an `i64` code, and check the rank before any lookup; the `Title` enum is gone), `claude/import`
  (`tail_info` and `read_window` hold one `RankedTitle` across records and borrow the final title as a `str` with no
  re-check; the string tail of `consume_entry` moves to an `inline(always)` `answer_string`, split with three asserts
  because the solver gave up on the whole body) and `claude/a1_root` (the a1 wrappers). No compiler change, no `pkgs`
  change. Details, the structure table and the tests: `pkgs/RESHAPE.md` §"Step 4.4r".
- 4.4p-CF: no code change. The literal `checked` argument folds after the forced inline on rs/rust and rs/llvm (IR
  evidence in `pkgs/RESHAPE.md`).
- 4.4n-KL: design note only (`pkgs/RESHAPE.md` §"4.4n-KL"); recommended a one-walk fixed-key-set read in
  `pkgs/access`.
- Tests (claude only, nothing shared changed): `test_a0`-`test_a4` all pass on rs/rust and rs/llvm (272, 2897, 2443,
  134, 270); `q.py` 20270/20270 on rs/rust and rs/llvm (exit 1 from the frozen py pairs only); `q_mutants.py` 18 mutants: 8
  detected, 10 proof-rejected, none survive (mutant 4's snippet updated to the new `ranked` line).
- Not measured (structure not identical: 4.4n-KL member walk per key, the owned user prompt and the model written
  after the content (4.4c-MB), the escaped candidate decoded at the lookup (4.3h-T)).
- Independent structure comparison (Codex `gpt-6.2-astra`, review role, Codex 2 account; the default account was
  at its limit). It agrees on one title buffer reused in both readers, the rank check before the lookup and no
  UTF-8 re-check of the final title; it found no wrong-result path and no new row.

### Rows after 4.4r

- Closed: 4.4m-TB (part two), 4.4p-CF.
- Designed, code open: 4.4n-KL.
- New: none.
- Still open: 4.4o-SB language side, 4.4o-EB, 4.4o-VP, 4.4q-CU, 4.4q-DN, and the rows listed before.
- Next:
  1. 4.4n-KL option 3: a one-walk `access::fields` for a fixed key set, then `claude/entry` / `claude/import` on it.
  2. 4.4o-SB language side, then 4.4o-EB.
  3. 4.4q-DN reference decision, 4.4o-VP.

### Bundle L-gaps 9

- Closed 4.4o-SB (R-6b-6): `str_into_bytes(move t) -> [u8]` hands the buffer of an owned `str` local to a byte
  array without a copy (hand `String::into_bytes`), and `str_bytes(s) -> ref [u8] from(s)` views the bytes of a
  `str` place, `subview`, `ref str` value or literal (hand `as_bytes()`). The verifier keeps the same array and
  length with byte elements. A missing `move` or a use after the move is `type/foreign_move`, moving a borrowed
  `str` parameter is `type/array_alias`, and a returned view of a local is `type/borrow_escape`. Rust emits
  `.into_bytes()` / `.as_bytes()`; LLVM hands over pointer, length and capacity and clears the moved slot, and a
  view keeps pointer and length only. Test `lgaps9_str_bytes` runs on both backends with and without
  `--no-verify` (alloc balance); rejects `lgaps9_into_bytes_use_after`, `lgaps9_into_bytes_borrowed`,
  `lgaps9_into_bytes_len`, `lgaps9_str_bytes_len`, `lgaps9_str_bytes_escape`. A `ref [u8]` parameter stays
  `type/borrow_signature` (`r6g_11_ref_param`); pass the view to a plain `[u8]` parameter.
- New rows:
  - LG7-SR stays open (reverted in this bundle). A per-list-term `Int -> Int` view sum (keyed by the z3 term, a
    term with a quantified variable kept the array-keyed `S(a, k)`) made a false claim about a view list of
    `subview`s of a parameter array refute in about a second with an `assert` counterexample (was
    `verify_unknown` after 7 s), but it slowed `tests/spec/lgaps7_join_bytes.pruv` from 3 s to about 100 s (over
    the 300 s spec_trace limit under load). Repros `work/lgaps9/lgaps9_views_subview_len.pruv` and
    `lgaps9_views_subview_sum.pruv`; a fix must keep `lgaps7_join_bytes` near 3 s.
  - LG9-SR2: the same two `subview` pieces joined with `vec_join` (separator `subview(b, 0, 1)`) and a false
    `assert len(s) == len(b)` still end in `verify_unknown` (`incomplete (theory array)`, fast). Dropping either
    the renamed list equality (`vjoin!aN == list`) or the per-position byte forall of the join makes it refute;
    naming the pushed views or the separator did not help. Repro `work/lgaps9/sr_join_unknown.pruv`. The
    must_reject test with `subview` pieces joined by `vec_join` waits for this row.
  - LG9-SB1 (audit, advisory): moving a borrowed `str` parameter into `str_into_bytes` is `type/array_alias`
    with the hint `take s: str`, but that edit then gives `type/foreign_move`. The hint should say
    `let t: str = copy(s);` then `move t`.
  - LG9-SB2 (audit, advisory): pin more R-6b-6 rejects: moving a `ref str` local (`type/array_alias`), moving
    `t` before the last use of `str_bytes(t)` (`type/borrow_conflict`), a missing `move` (`type/foreign_move`),
    and `assert utf8_valid(a)` after `a[0] = 255` on bytes from `"a"` (counterexample `[255]`).
  - LG9-SB3 (audit, advisory): runtime paths not in `lgaps9_str_bytes` (each checked by hand on both backends):
    `str_into_bytes` in a value `if`, from a match payload binder, in a struct literal field, in a loop with
    reassignment, on an empty `str` then `arr_push`, and on a `str_from_utf8` result with capacity above length;
    `return str_bytes(s)`, `str_bytes(subview(..))`, `str_bytes` pushed to a `Vec<ref [u8]>`, from `vec_get` and
    from an `inout` struct field, and `str_bytes` in contracts under `--no-verify`.
  - LG9-SB4 (audit, advisory, older than this bundle): the Rust bindings written next to `--emit ll`
    (`bindings.rs`) wrap a function that returns `ref [u8]` or `ref str` as an owned `Vec`/`String` and declare a
    `pruv_abi_*__typed` symbol the `.ll` does not define.
- Audit (A: SPEC, verifier and tests; B: Rust and LLVM codegen). Fixed: the R-6b-6 rule had no accept or reject
  test under `tests/spec` (added `lgaps9_str_bytes` and `lgaps9_into_bytes_use_after` there), and the two LG7-SR
  spec tests had no JSON (moot after the revert).
- Not reached: LG8-A4, LG7-T3.
- Next: LG7-SR (keep `lgaps7_join_bytes` fast), LG9-SR2, LG8-A4, LG7-T3, LG9-SB1, LG9-SB2, LG9-SB3, LG9-SB4.

## Step 4.4s: one walk per root object for the claude key set (4.4n-KL option 3)

- `pkgs/access`: a member cursor, three new functions (no change to `field` / `find_ascii`):
  `members_end(doc, id)` (the object's skip link, or -3 / -2 as `field`), `member_next(doc, id, j, stop)` (the
  next member after key record `j`, with `find_ascii`'s per-member checks; -4 = corrupt tape) and `key_is(doc, j, key)`
  (`may_match` then `ascii_equal`).
- `claude/entry`: `RootKeys` (13 `i64` slots, returned by value) and `root_keys(doc)`: one walk over the root's
  members, a fixed `key_is` chain per member, slot = value id, later duplicates overwrite (last wins, as `field`).
  Missing = -1; a non-object or bad root fills every slot with `field`'s code; a corrupt tape fills -4. The readers
  (`human_text_of`, `user_text`, `title_for_type`, `ranked`, `string_content`, `array_content`, `is_human_text`) take
  slots; the a1/a2 wrapper functions (`human_text`, `pick_title`, `title_of`, `task_origin`, `is_true`, ...) keep
  their signatures and resolve through `root_keys` or `field`.
- `claude/import`: `read_head`, `tail_info` and `window_object` call `root_keys` once per line; `consume_entry`
  takes the slots. `read_window`'s per-line parse moved to `window_line` (the solver gave up on the parsed document's
  facts in the loop body once `RootKeys` joined the module: `verify_unknown` on `doc_strings(d)`).
- Why the 4.3h-R reason does not come back: `root_of` allocated an 8-word array and resolved 7 keys for every
  object, title-only and empty ones included. `root_keys` allocates nothing (a struct of `i64` in registers / on the
  stack), walks the members once, and an empty object costs the one `members_end` read.
- Tests: claude `test_a0`-`test_a4` (272, 2897, 2443, 134, 270), `q.py` 20270/20270 and every `pkgs/access`
  consumer pass on rs/rust and rs/llvm (table in `pkgs/RESHAPE.md` §"Consumer tests (step 4.4s)").
  `q_mutants.py` 18: 9 detected, 9 proof-rejected, none survive (mutant 1 retargeted to `user_text`'s `true_of` line).
- Not measured: structure not identical (the hand builds a `BTreeMap` per object at parse; nested objects still use
  one `field` walk per key; 4.4c-MB, 4.3h-T rows unchanged).
- No audit: no new result meaning (same slots and codes as the per-key `field` calls; the new access functions are
  `find_ascii`'s walk split into steps).

### Rows after 4.4s

- Closed: 4.4n-KL (root reader). Nested reads (`message.content`, `origin.kind`, block `type` / `text`) still call
  `field` per key, as the hand calls `get` per key on those maps.
- New: 4.4s-FK (language gap): a generic `access::fields(doc, id, [&str; N]) -> [i64; N]` needs `[str; N]` or const
  generics; neither exists, so the fixed key set lives in the caller (`entry::root_keys`) on the access cursor.
- Still open: 4.4o-SB language side, 4.4o-EB, 4.4o-VP, 4.4q-CU, 4.4q-DN, and the rows listed before.
- Next:
  1. 4.4o-SB language side, then 4.4o-EB.
  2. 4.4q-DN reference decision, 4.4o-VP.
  3. 4.4s-FK if the language gets `[str; N]` / const generics (then `root_keys` can become `access::fields`).

### Bundle L-gaps 10

- Closed LG7-SR (R-6g9-13, R-6g9-16). Cause of the L-gaps 9 slowdown: the quantified join step
  `S(k+1) = S(k) + len(parts[k])` fires only on a `parts[k]` term already in the goal, so finding the view block
  of a result position depended on the z3 seed. The old encoding had the same sensitivity (the dumped
  `lgaps7_join_bytes` goal timed out under some seeds); the per-term sum only moved which seeds were lucky. Fix:
  a closed list term (no quantified variable) gets its own `Int -> Int` view sum `vsum!N`, kept per term id, and
  `vec_join` of a list with a constant length up to 64 also asserts the step at every index as a ground fact
  (`len(parts[j]) >= 0 && S(j+1) = S(j) + len(parts[j])`); a term under a quantifier keeps the array-keyed
  `S(a, k)`. `vec_join` reads the sum of the list term before it renames the list, so the push facts reach it.
  Timing on the same machine, run one after the other: `tests/spec/lgaps7_join_bytes.pruv` 0.33 s on this branch,
  3.67 s on main (`lgaps7_join` 0.20 s / 0.19 s). The false length and sum claims about two `subview`s of a
  parameter now refute in about 0.01 s with counterexample `b = [2], m = 0` (main: `verify_unknown`, canceled).
  New rejects `lgaps9_views_subview_len` and `lgaps9_views_subview_sum` (tests/spec and tests/must_reject).
- Closed LG9-SB1: moving a borrowed `str` parameter is still `type/array_alias`, now with the message
  "parameter s is a borrowed str" and the hint `let t: str = copy(s);` then `move t` (was `take s: str`, which
  ends in `type/foreign_move`). Accept test `lgaps10_into_bytes_param_fixed` runs the hinted form on both backends
  with and without `--no-verify`. Golden digests of `lg17_str_move_param`, `lg36_str_move_under_outer_arm` and
  `lgaps9_into_bytes_borrowed` changed with the message.
- Closed LG9-SB2: rejects `lgaps10_into_bytes_ref_local` (`type/array_alias`), `lgaps10_into_bytes_borrowed`
  (`type/borrow_conflict`), `lgaps10_into_bytes_no_move` (`type/foreign_move`) and `lgaps10_into_bytes_utf8`
  (`assert`, counterexample `a = [255]`), in tests/spec and tests/must_reject. The borrowed-parameter case was
  already `lgaps9_into_bytes_borrowed`.
- Closed LG8-A4: Rust `--no-verify` `vec_join` sums a saturating u64 total (pieces plus separators) and stops with
  `vec_join: total length <= 2^40` before joining, as LLVM does. Verified code is unchanged.
- Still open, with a new diagnosis:
  - LG9-SR2: `vec_join` of `subview` pieces with a false `assert len(s) == len(b)` still ends in
    `verify_unknown` (`incomplete (theory array)`, fast). The cause is not the view sum: the goal without the
    per-position byte forall is `sat`, and it stays `unknown` with the separator lambda replaced by a plain array
    select. The forall reads `arr(parts[q(i)])` where the elements hold `subview` lambdas. By hand, one byte forall
    per piece (constant length, offset `S(k) + k * len(sep)`) instead of the `q(i)` forall is `sat`, but its
    pattern is an offset term, which may lose the positive byte proofs (`lgaps7_join_bytes`); not tried in code.
    Repro `work/lgaps9/sr_join_unknown.pruv`.
- Not reached: LG7-T3 (verified `--foreign-checks all` pop check), the audit.
- Checks: `cargo test --release`; `diff_rust --fuzz-count 20`; `run_all --only must_reject` and `--only
  g8_frontend` on both backends; `m3_all --quick` (goldens updated with `--update-golden`: the three message
  digests, new files and the mutant sample shifted by them); `--list-rows` diff (10 new rows).
- Next: LG9-SR2, LG7-T3, LG9-SB3, LG9-SB4.

## Step 4.4t: OpenCode moves strings into byte pools (4.4o-SB writing side, 4.4o-EB), hand `dirname` follows Node (4.4q-DN)

Only `bench/l3/opencode` changes (no compiler, no `pkgs/`, so no consumer runs). Detail and tables in
`bench/l3/opencode/RESHAPE.md` §12.

- `path::utility` mode 3: `str_into_bytes(move d)` (hand `data_dir(&conf).into_bytes()`) instead of `to_bytes(d)`.
  `p100` was already moved in 4.4q.
- `export::start_export`: one `utf8_valid` on the configuration text, the root borrowed with `cfg_view` (hand
  `conf.temp_root`), `join(root, "opencode-")` and `str_into_bytes` as the request pool (hand `make_temp`). The byte
  pool built by hand and `path::normalize_bytes` are gone.
- `path::dirname`: hand shape (`Option` end, `break`, `match`). The hand `dirname` was a different algorithm on repeated
  slashes; it now equals Node `path.posix.dirname` (pinned TypeScript = reference) on 17 inputs.
- Tests (rs/rust and rs/llvm): `--verify-only` VERIFIED, fixtures 199/199, `q.py --cases 300` exact 406 of 499 with 0
  unexplained, `q_mutants.py --check` 16/16 markers; hand `opencode.rs` builds without warnings.
- Not measured (structure not identical). No audit: no new result meaning on reachable inputs.

### Rows after 4.4t

- Closed: 4.4o-SB (writing side), 4.4o-EB, 4.4q-DN.
- New: 4.4t-RV (language gap): `for` has no `rev` (R-6q-3), so OpenCode `dirname` keeps a counting-down `while` where
  the hand writes `for i in (1..n).rev()`.
- Still open: 4.4o-VP (compiler: `vec_push` of a view keeps the `2^40` length check; not touched here because
  pruvc-rs/ was out of scope), 4.4q-CU, 4.4s-FK, and the rows listed before.
- Next:
  1. 4.4o-VP in an L-gaps bundle (compiler).
  2. 4.4t-RV if `for` gets a `rev` adapter (then `dirname` takes the hand's loop).
  3. 4.4s-FK if the language gets `[str; N]` / const generics.

### Bundle L-gaps 11

- Closed LG9-SR2 (R-6g9-16): a `vec_join` of a list with a constant length up to 64 (`JOIN_GROUND_MAX`) now gets
  one byte forall per view block instead of the `q(i)` forall: for view `j` at offset `P(j) = S(j) + j * len(sep)`,
  `P(j) <= i < P(j) + len(parts[j]) => res[i] == parts[j][i - P(j)]`, and for `j < n - 1` the separator block after
  it, each keyed on `res[i]` (so positive byte proofs still fire on the read `s[i]`). The `q(i)` forall read
  `parts[q(i)]` whose elements were `subview` lambdas and z3 answered `unknown` (incomplete array theory). A list of
  unknown or larger length keeps the `q(i)` forall. Timing (same machine): `work/lgaps9/sr_join_unknown.pruv`
  refutes in about 1 s with counterexample `b = [2], m = 1` (was `verify_unknown`); `tests/spec/lgaps7_join_bytes.pruv`
  0.10 s (0.33 s in L-gaps 10), `lgaps7_join` 0.28 s. Every `lgaps6_views*`, `lgaps7_join*`, `lgaps8_join*`,
  `lgaps9_*` and `lgaps10_*` spec file keeps its result. New reject `lgaps11_join_subview_len` (tests/spec and
  tests/must_reject).
- Closed L-gaps 10 A1: the ground steps and the per-view byte facts apply only when the constant length is in
  `0..=64` (a length of 65 used to unroll 64 steps); the comment says so.
- Closed L-gaps 10 A2: accept `tests/spec/lgaps11_views_sum_quant` reads the prefix sums of one list outside a
  quantifier (per-term sum) and inside one (array-keyed sum) and proves them equal and equal to the totals.
- Closed L-gaps 10 A3: moving a `ref str` local is still `type/array_alias`, now "r is a ref str view and cannot be
  moved" with the hint `let t: str = copy(r);` then `move t` (golden digest of `lgaps10_into_bytes_ref_local`
  changed).
- Closed LG7-T3: runtime `lgaps11_pop_checks_all` pops a two-view list with `--foreign-checks all` (verified; LLVM
  keeps the `len(v) >= 1` run-time check, Rust emits the proven `pop()`) and without, on both backends with alloc
  balance.
- Closed LG9-SB3: runtime `lgaps11_str_bytes_paths` (both backends, verified and `--no-verify`, alloc balance):
  `str_into_bytes` in an `if` branch return, from a consuming match payload binder, in a struct literal field, in a
  loop with reassignment, on an empty `str` then `arr_push`, on a `str_from_utf8` result then `arr_push`; `return
  str_bytes(s)`, `str_bytes(subview(..))`, `str_bytes` pushed to a `Vec<ref [u8]>` and read back with `vec_get`,
  from an `inout` struct field, and `str_bytes` in an `ensures` under `--no-verify`. A value `if` expression is not
  in the language, so the `if` path is a branch `return`.
- Closed LG9-SB4: the `--emit ll` Rust bindings skip a function with a borrowed result (`ref [u8]` / `ref str`), as
  `is_host_abi_fn` already did for the `.ll`; before, `bindings.rs` wrapped it as an owned `Vec`/`String` and
  declared an undefined `pruv_abi_*` / `pruv_abi_*__typed` symbol.
- New row LG11-QS: a false claim about a prefix sum inside a quantifier (`assert forall k in 2..3:
  vec_views_len(out, k) == 4` on the A2 list) does not refute quickly (killed after 2 minutes); the array-keyed sum
  has only the definition instances around `k`.
- Still open: P10-M1, LG7-A1, LG7-B1, LG7-B4 (Rust `vec_pop` keys on `verified`, LLVM on `vec_get_checked`).
- No audit in this bundle (cap); the SR2 encoding change is the one new verifier meaning.
- Checks: `cargo test --release` (163 passed); `run_all --only must_reject` 1371/1371; `--only g8_frontend
  --backend both` 39/39; `--only runtime --sample-prefix tests/runtime/lgaps11 --backend both` 8/8; `diff_rust
  --fuzz-count 20` (golden mismatches only from the new files, the A3 message and the shifted mutant sample);
  `m3_all --quick --update-golden` (all stages ok; llvm digests of `lgaps9_str_bytes` and `p9_rt_alias_pos` changed from LG9-SB4, their borrowed-result functions left the bindings); `--list-rows` diff (9 new rows, `tests/run_all_rows.txt` updated).
- Next: audit of the LG9-SR2 encoding, LG11-QS, LG7-B4, P10-M1.

## Step 4.4u: Qwen chain walk marks and follows separately (4.4d-QC), arena and title without owned copies (4.3e-F, 4.3e-J)

Only `bench/l3/qwen` changes (no compiler, no `pkgs/`, so no consumer runs). Detail in `bench/l3/qwen/RESHAPE.md` §3c.

- `chain::resolve` follows the hand `current_chain`: `select` marks every entry with the current uuid (hand
  `selected.insert`) and only the reached entry's parent is read; `newest` returns the newest entry of the parent uuid
  (hand `latest.get`). A `-2` parent defect on an unfollowed duplicate no longer fails the chain. The `Leaf` struct and
  `lookup` / `Hit` are gone. New fixture `duplicate-bad-parent` (two `u` entries, the older with a lone-surrogate
  parent). It is a regression guard only: the JSON decoder maps a lone surrogate to U+FFFD, so `-2` cannot come from
  input (the old code also passed it).
- `chain::put_field` decodes straight onto the arena with `access::string_append` (4.3e-F; no owned `str` first).
- `entry::array_content` pushes each part from its `plain_str` view, falling back to the owned decode only for an
  escaped node; `title_key` checks blankness on the view and copies only a nonblank title (4.3e-J). This needs
  `doc_strings` from `parse_line` down the call chain; `window_wire`'s per-object title and replay records moved into
  `import::window_object` and `tail_info` reads the title before the timestamp, so the solver proves it near
  `parse_line` (both defects fail the line alike).
- Tests (rs/rust and rs/llvm): `test_impl.py` PASS 27/27, `q.py --count 300` (rs pairs) same 317, P1 1, P4 4, P5 1,
  unexplained 0, list same 21, P4 1; `q_mutants.py` 11 differential-killed, turn-cap verifier-rejected.
- Not measured. No audit: no new result meaning on reachable inputs.

### Rows after 4.4u

- Closed: 4.4d-QC, 4.3e-F, 4.3e-J.
- New: 4.4u-TO (order): `tail_info` reads the custom title before the timestamp (hand: timestamp first) so
  `doc_strings` is proved in a small context; output is the same because either defect fails the line.
- Still open: 4.3e-D / 4.4d-QS (the scan still records prompt, stamp and parts of every entry before the chain is
  chosen; needs raw string ranges in the log decoded in `replay`, or a parsed-document vector, gap VL), 4.4d-QA, and
  the rows listed before.
- Next:
  1. 4.3e-D / 4.4d-QS: log raw ranges of `data` for prompt, stamp and parts and decode them in `replay` for selected
     and attached entries only.
  2. 4.4d-QA with gap 4.4c-SV (a `[u8]` span lent as `str`).
  3. 4.4u-TO back to hand order once the solver keeps `doc_strings` through the loop body.

### Bundle L-gaps 12

- Closed LG11-QS (R-6g9-13): inside a quantifier, a list term that mentions no bound variable now uses the same
  per-term sum function `vsum!N` as outside (so the push facts reach it), and a closed list of constant length in
  `0..=64` gets the ground sum steps with its view lengths as an `ite` over the ground lengths, so no `a[k]` select
  of `subview` lambdas sits under the forall. The A2 false claim `assert forall k in 2..3: vec_views_len(out, k) ==
  4` refutes in about 1.2 s with counterexample `p = [11, 11, 29]` (killed after 2 minutes before; the first step
  alone gave `verify_unknown`); the accept `lgaps11_views_sum_quant` stays at 0.04 s. New reject
  `lgaps12_views_sum_quant_false` (tests/spec and tests/must_reject) with `max_seconds: 30`; `run_all.py`
  must_reject jobs now honor `max_seconds` (spec_trace already did).
- Closed LG7-B4 (R-6g9-10, R-6g9-11, R-6d-2): view-list (`Vec<ref str>` / `Vec<ref [u8]>`) run-time checks for
  `vec_push`, `vec_get`, `vec_pop` and `vec_join` are emitted exactly when the build is unverified, on both backends;
  `--foreign-checks` and `trusted rust fn` no longer change them, since a view list never crosses a function or
  foreign boundary. Rust view push/get keyed on `unchecked_collections()`, LLVM view get/pop on `vec_get_checked`;
  both now key on `verified`. Runtime `lgaps12_view_checks` (verified, default and `--foreign-checks all`, both
  backends: the check strings are absent) and `lgaps12_view_checks_nv` (`--no-verify`, both backends: the
  `vec_push`/`vec_pop`/`vec_get`/`vec_join` messages, the `len(v) >= 1` pop check and the push panic are present),
  with alloc balance. `lgaps11_pop_checks_all` now runs with no pop check on LLVM under `all` (comment updated).
- Closed 4.4o-VP: the LLVM view push shim drops its `2^40` guard in a verified build (`Mod.verified`), and the Rust
  view `vec_push` is `parts.push(v)` even when the program has a `trusted rust fn` (OpenCode imports `jstr`, whose
  `utf8_to_string` is trusted, so every view push kept the check). Runtime `lgaps12_view_checks_trusted` (both
  backends, with a trusted `::std::cmp::min`; the view checks are absent).
- Closed LG7-A1 and LG7-B1 in L-gaps 8 already (5e898eff: `tests/spec/lgaps8_join_bound*.pruv`; the LLVM
  no-verify join total uses a saturating running sum with `llvm.smul/sadd.with.overflow`); the L-gaps 11 "Still
  open" line was stale. Both spec files still pass.
- New row LG12-RS: `vec_reserve` on a view list is outside the LG7-B4 rule: LLVM always checks `0 <= n && n <=
  2^40`, Rust keys it on `unchecked_collections()` (checked with a trusted fn or `--foreign-checks all`).
  SPEC R-6d-2 and R-6g9-11 name the four operations and point `vec_reserve` here (audit A).
- New row LG12-QK: no quantified reject reads a prefix sum outside `0..=n` (`k == -1` or `k == n + 1`); audit A
  found the encoding sound there (the ground steps are guarded by `0 <= k < n` / `1 <= k <= n`), only the test is
  missing.
- Audit (one round, at once): A Codex gpt-6.2-astra high (Codex 2 account; the default account was at its limit),
  spec only: no wrong accepted program in LG11-QS, no wrongly closed list term; findings were the `vec_reserve`
  wording (fixed in SPEC and the test comment), the trusted test being Rust-only (now both backends) and LG12-QK.
  B Claude opus high, codegen (static, its runs were blocked): no elided check in a build where the obligation is not
  proven; the vpush shim is only used by view lists. Its one finding, LLVM dropping the view push guard with
  `--no-verify --foreign-checks off`, cannot happen: pruvc rejects that flag pair ("--foreign-checks off requires
  verification"), so unverified always has `foreign_requires`. Its note on the unreachable `list[index]` branch in
  the Rust view `vec_get` (codegen.rs) is left as is.
- Still open: P10-M1, LG12-RS, LG12-QK, 4.4t-RV, 4.4s-FK.
- Checks: `cargo test --release` (163 passed); `run_all --only must_reject` 1372/1372 (the new reject about 1.2 s);
  `--only spec_trace` ok (`lgaps8_join_bound*` still pass); `--only g8_frontend --backend both` 39/39; `--only
  runtime --sample-prefix tests/runtime/lgaps12 --backend both` 10/10; `--only runtime --backend both` 3511/3511 (before the two LLVM rows of the trusted test, which pass in the 10/10);
  `diff_rust --fuzz-count 20 --update-golden` (new files and the shifted mutant sample); `m3_all --quick
  --update-golden` (all stages ok; llvm digests of the verified view programs `lgaps9_views_subview_*`,
  `lgaps11_join_subview_len`, `lgaps11_pop_checks_all`, `lgaps11_str_bytes_paths` changed because their view checks
  left); `--list-rows` diff (11 new rows, `tests/run_all_rows.txt` updated).
- Next: LG12-RS (key view `vec_reserve` on `verified` too), LG12-QK, P10-M1.

### Rows after 4.4w

- Closed: 4.4d-QA (Qwen `replay` lends prompt, answer and stamp spans of `log.text` as `str` views, R-6g-10, with
  `utf8_valid(log.text)` kept through the scan), 4.4u-TO (`tail_info`'s body is `import::tail_object`, timestamp
  then title as the hand).
- New: 4.4w-CE (bench: two char-edge byte tests per lent span; closing needs a per-record fact over the log),
  4.4w-VD (language: `Vec<json::JsonDocument>` is `type/foreign_owned_type`, so no parsed-document vector),
  4.4w-TR (package: `pkgs/time` has no `iso` over a raw range of `data`), 4.4w-DU (package: `jstr::decode_append`
  ensures no UTF-8 fact, so a decoded range cannot be lent as `str` without a run-time pass).
- Still open: 4.3e-D / 4.4d-QS (scan-time join, iso and parts walk; blocked by 4.4w-VD, or by 4.4w-TR with 4.4w-DU),
  4.4c-SV (`str` lent as `[u8]`, the other direction; it did not block 4.4d-QA), and the rows listed before.
- Tests (rs/rust and rs/llvm): `test_impl.py` PASS 27/27, `q.py --count 300` (rs pairs) same 317, P1 1, P4 4, P5 1,
  unexplained 0, list same 21, P4 1; `q_mutants.py` 11 differential-killed, turn-cap verifier-rejected. Not measured (structure still differs at 4.3e-D).
- Next:
  1. 4.4w-TR and 4.4w-DU in `pkgs/time` / `pkgs/jstr` (all consumers rerun), then 4.3e-D / 4.4d-QS with raw ranges.
  2. Or 4.4w-VD in the language, then the hand's parsed-entry vector.
  3. 4.4w-CE once a quantified record fact over `log.records` is affordable.

## Step 4.4v: buffered bytes before polling (4.4d-PO), line cleared after use (4.4d-LC), cursor kept on -3 (4.4d-RC), empty growable buffers (4.3f-B), dead turns helpers removed (4.4d-TD)

Changes `bench/l3/pkgs/transport`, `pkgs/turns`, `mcp`, `grok`, `codex` (no compiler change).

- `transport::receive_grow` is split into `take` (scan the buffered bytes for LF), `wake` (poll 0 and liveness),
  `fill` (read stdout into the chunk, `take`, then wait 10 ms) and `receive` / `receive_tail` (MCP keeps the stderr
  tail). Buffered bytes are scanned before any poll (hand `Rpc::step`); `scan` runs only the scan block, and the grok
  drain uses it (hand `step(false)`: a partial line ends the drain without a read or a poll). `Reader.filled` is gone: the partial line is
  `len(line)`. Over the cap, `-3` leaves `at` at the piece start (hand: cursor unchanged).
- Consumers clear the line with `truncate(line, 0)` after the line is used (hand `line.clear()` after `on_line`):
  grok pump and drain, mcp after `parse_copy`, codex `open` and the three root loops.
- Codex: the private `rpc::receive` is gone, `step` calls `transport::receive`. The line and the `hb` arena start as
  an empty `[u8]` with a reserve and grow by `arr_append`; `ByteBuffer`, `byte_buffer`, `grow_bytes` and the `tail`
  dependency are removed. `read::replay` grows `hb` to the header + base when no turn is held.
- mcp: the line starts empty with a 4096-byte reserve.
- `turns`: `empty_buffer`, `finish2` and `finish_ref` are removed with the comments that cite them.

### Rows after 4.4v

- Closed: 4.4d-PO, 4.4d-LC, 4.4d-RC, 4.3f-B, 4.4d-TD.
- New: 4.4v-SN (gap): stderr is still read and dropped (`ev_proc_read` on fd 2) because Pruv has no spawn with a
  null stderr and `ev_eof` is true only when both streams are drained; the hand uses `Stdio::null()`.
- New: 4.4v-LC (contract): the line clear is a consumer convention; `receive` does not require an empty line on a
  fresh read (the requirement puts a proof burden on every pump invariant).
- New: 4.4v-RT (shape): `receive` and `receive_tail` repeat the same 8 lines (no function parameters to pass the
  stderr action).
- New: 4.4v-PB (shape, from the review): the pump and codex loops count every `step` against the 1,000,000 bound,
  partial chunks included; the hand `step(true)` waits inside one call. Output differs only past a million steps.
- New: 4.4v-TR (order, from the review): when a chunk and the overall timer are ready together, `receive` checks the
  timer first; the hand `recv_timeout` can take a chunk already queued. Only the race at the deadline differs.
- Still open: the rows listed before 4.4v.
- Review: one structure review (Codex gpt-6.2-astra, generated vs hand Rust only). Its one in-scope output difference,
  the grok drain reading a next chunk after a partial line, is fixed here; the rest are the rows above or cost-only.
- Tests (rs/rust and rs/llvm): claude a0-a4, json a0-a5, supervisor a0-a3, qwen 27/27, mcp 22/22, grok 254/254,
  codex 136/136, opencode 199/199. Not measured.
- Next:
  1. 4.4v-SN once the language has a spawn option with a null stderr.
  2. 4.4v-RT with function parameters or a stderr sink type.

### Rows after L-gaps 13

- Closed 4.4w-VD (R-6e-3): the cause was the field set, not the module owner: an `[i64]` buffer field did not make
  an owned aggregate (a local struct was rejected too). `[i64]` now counts like `[u8]`/`[u16]` (Rust `Vec<i64>`,
  LLVM element drop of R-6e-22), so `Vec<json::JsonDocument>` compiles on both backends; a struct with an `[i64]`
  field also moves under R-6e-12 and is a host-entry parameter under R-6f-6. `vec_get::<S>(v, i)` of an owned
  aggregate is also a read-only call argument (Rust `&(*v.get_unchecked(i))`); a let copy, an owned argument and an
  inout alias of the same Vec stay rejected. There is no move out: `vec_pop` is a view-list operation and there is
  no `vec_remove`. `[bool]` and `str` fields stay out (no LLVM element-drop or field-borrow path); the four reject
  tests that used `[i64]` as the "not owned" example now use `[bool]`, and the `[i64]` forms are accept tests.
- Closed LG12-RS (R-6d-2, R-6g9-11): view-list `vec_reserve` checks `0 <= n && n <= 2^40` exactly when unverified
  on both backends, like the other LG7-B4 operations; verified code proves it (`verify/call_requires`).
- Closed LG12-QK (R-6g9-13): quantified rejects at `k == -1` and `k == n + 1`, and the in-range accept.
- Still open: P10-M1, narrowed: `[T]` element places were closed in L-gaps 8 (the later "still open" lines were
  stale); a `Vec<T>` collection element (`swap(inout v[i], ...)` with `v: Vec<i64>`) is still
  `type/inout_argument` and needs the checker, the Vec verifier model and both backends. Not reached in the cap.
  Also 4.4t-RV, 4.4s-FK.
- Checks: `cargo test --release` ok; `run_all --only must_reject` 1378/1378; `spec_trace --only-rs` 2353/2353;
  `--only g8_frontend --backend both` 39/39; `--only runtime --backend both` 3521/3521; `diff_rust --fuzz-count 20`
  success after `m3_all --quick --update-golden` (all stages ok; digests of `g9_ext_in_vec`, the
  `lgaps12_view_checks*` programs and mutants shifted by the new must_reject files changed); `--list-rows` diff
  (14 new rows, `tests/run_all_rows.txt` updated); `bench/l3/qwen/test_impl.py` 27/27 rs/rust and rs/llvm.
- Audit: one review round (Codex 2 account; the default account was at its limit). Fixed in a follow-up commit:
  a nested holder-record field of a Vec element as a read-only argument moved out of the vector in Rust (now a
  place borrow; test `lgaps13_vec_nested_field`); on LLVM a range-check failure
  in a later argument leaked an owned value already moved into an earlier `take` argument (now dropped on that
  path, 3/3 allocations against 3/2 before; test `lgaps13_take_then_fail`);
  R-6e-3, R-6e-6 and R-6e-22 text (read-only argument context, `vec_clear`, the `[bool]`/`str` reason, buffer
  field lists). Not taken: an upper-bound-only `vec_reserve` reject and a same-Vec `inout` alias test (optional).
  Confirm review (same account): all three fixed, no new findings.
- Next: P10-M1 for `Vec<T>` elements, then the hand's parsed-entry vector on top of 4.4w-VD (bench rows 4.3e-D /
  4.4d-QS).

## Step 4.4x: UTF-8 fact of `jstr::decode_append` (4.4w-DU), `time::iso_raw` over a raw range (4.4w-TR)

Changes `bench/l3/pkgs/jstr` and `pkgs/time` (no compiler change, no consumer change).

- `jstr::scalar` ensures the value is not a surrogate (`< 55296 || > 57343`; lone surrogates were already 65533).
  `decode_append` ensures `(utf8_valid(old(out)) && !scanned) ==> utf8_valid(out)`: in the non-scanned mode a raw
  run is ASCII only (new inner-loop invariant), and an escape or a multi-byte char pushes the encoding of a valid
  scalar. The four pushes of one encoding are now one `arr_push(inout out, v1, .., vk)` (R-6g5-5, hand
  `char::encode_utf8` then one append); the outer loop keeps the fact as an invariant with `old(out)`. No run-time
  pass is added. The scanned mode (raw non-ASCII bytes copied as they are) ensures nothing new (row 4.4x-DS).
- `time::iso_raw(b, lo, hi)` is the hand `iso(&Value)` over the raw token bytes `b[lo..hi]` of one JSON value: a
  token starting with `"` is a string (the 26-byte canonical copy, else `jstr::string_text_owned` then
  `from_string`, hand `iso_str`), anything else a number token parsed as `access::number` does (non-integer or
  above 2^53-1 is absent, hand `iso_number`). The caller keeps only a byte range, no document. `iso_of` is left as
  it is (a delegation would need `sp.end <= 16777216` on every document, which `iso_of` does not require).

### Rows after 4.4x

- Closed: 4.4w-DU (non-scanned mode), 4.4w-TR.
- New: 4.4x-DS (package): `decode_append` in the scanned mode ensures no UTF-8 fact. Proving it needs every raw run
  to start on a char boundary of a `utf8_valid_span` source, and the byte after `\` (or after `\uXXXX`) may be a
  continuation byte in input the parser rejects; either an extra requires on the source (escapes followed by ASCII)
  or a literal-backslash rule for that input. Non-scanned decoding gives the same bytes on valid input.
- Still open: 4.3e-D / 4.4d-QS (Qwen scan-time join, iso and parts walk). Now unblocked by 4.4x (raw ranges decoded
  in `replay` with `decode_append(.., false)` and `iso_raw`), not done in this bundle (time cap); the design is in
  `qwen/RESHAPE.md` §3e. Also 4.4w-VD, 4.4w-CE, 4.4c-SV and the rows listed before.
- Tests (rs/rust and rs/llvm): claude a0-a4, json a0-a5, supervisor a0-a3 pass; qwen 27/27, mcp 22/22, grok
  254/254, codex 136/136, opencode 199/199 (`--jobs 2`); qwen `q.py --count 300` (rs pairs) same 317, P1 1, P4 4,
  P5 1, unexplained 0, list same 21, P4 1; `q_mutants.py` 11 differential-killed, turn-cap verifier-rejected.
  Not measured (Qwen structure still differs at 4.3e-D).
- Next:
  1. 4.3e-D / 4.4d-QS with raw ranges (qwen §3e).
  2. 4.4x-DS if the Qwen replay needs the scanned fast path.

## Step 4.4y: Qwen scan records raw ranges, replay decodes selected entries (4.3e-D / 4.4d-QS)

Changes `bench/l3/qwen/import.pruv` only (no compiler or package change). Details in `qwen/RESHAPE.md` §3f.

### Rows after 4.4y

- Closed: 4.3e-D / 4.4d-QS (the scan-time join, decode and iso): the scan records raw token ranges of `data`;
  `replay` decodes the selected entries with `jstr::decode_append(.., false)`, joins the prompt, and formats the
  stamp with `time::iso_raw`, an assistant's only when a turn is attached. 4.4w-CE closed (no log spans, no
  `char_edge`).
- New 4.4y-PW: the parts visibility walk (`thought` / `text` / `functionCall`) still runs in the scan for every
  object; the hand walks `visible_parts` in replay for a selected user or an attached assistant. Moving it needs
  the parsed documents in replay (`Vec<json::JsonDocument>` compiles after 4.4w-VD, but replay would also need
  `doc_strings` for each element, which `vec_get` does not carry); not tried in this bundle.
- New 4.4y-SB: `prompt` / `piece` scratch buffers reused across entries and the model decoded from its raw token;
  the hand allocates a `String` per part (serde) and per `join`, and clones the model.
- Still open: 4.4x-DS, 4.4c-SV, P10-M1 and the rows listed before.
- Tests (rs/rust and rs/llvm): qwen `test_impl.py` 27/27; `q.py --count 300` (rs pairs) same 317, P1 1, P4 4,
  P5 1, unexplained 0, list same 21, P4 1; `q_mutants.py` 11 differential-killed, turn-cap verifier-rejected (the first mutant ran on the file before the model/stamp reorder). Not measured (structure differs at
  4.4y-PW, 4.4y-SB).
- Next: 4.4y-PW (per-element facts for a Vec of parsed documents), then 4.4y-SB.

### Rows after L-gaps 14

- Closed P10-M1 (R-6g9-12): an element `v[i]` of a `Vec<i64>` / `Vec<bool>` is a `swap` / `replace` place (hand
  Rust `v.swap(i, j)` / `std::mem::replace(&mut v[i], x)`). Each index carries the range obligation, the two places
  of a swap are proven distinct, and an immutable Vec, an out-of-range index and a same-Vec alias stay rejected.
  LLVM calls the shim adapters `vswap` / `vreplace`; a narrow unsigned index is widened as for arrays.
- Closed 4.4t-RV (R-6q-1): `for i in (a..b).rev()` visits `b - 1` down to `a` (Rust `for i in (a..b).rev()`). In an
  invariant `i` is the low end of the visited part (`b` before the first pass, `min(a, b)` after the loop) and the
  measure is `i - a`; quantifier triggers are renamed like the rest of the invariant. `(P).rev()` over a slice and
  `(a..=b).rev()` stay `parse/for_iterable`. OpenCode `dirname` can now take the hand's loop.
- Closed 4.4v-SN (R-6l-3): `proc_spawn_quiet(program, args, cwd)` spawns with the child's stderr on the null device
  (Rust `Stdio::null()`); stream 2 is empty and at end of file from the start, also inside an event loop
  (`ev_add_process`), and a large stderr write cannot block the child. Same contract and must-consume handle as
  `proc_spawn`. Consumers to switch (left to the bench owner; bench/l3/pkgs is not touched here): the fd-2 readers
  that read and drop stderr with `ev_proc_read(l, slot, 2, ...)` in mcp, grok, codex and qwen; after the switch
  their stderr branch and the 4.4v-RT duplicate in `receive` / `receive_tail` go away.
- Still open: 4.4s-FK, 4.4v-RT (the stderr half goes away with 4.4v-SN). New LG14-ST: the slice-for rewrite
  (`for x in P` with `index(x)`) also skips quantifier triggers (`for_index_rewrite`); not reached in the cap.
- Checks: `cargo test --release` ok; `run_all --only must_reject` 1380/1380; `--only g8_frontend --backend both`
  39/39; `spec_trace --only-rs` 2381/2381; runtime `--backend both`: tests/spec 374/374, g10p_r23 6/6, g10a_r23_quiet_ev 4/4,
  p4_rt_for 10/10; `diff_rust --fuzz-count 20` and `m3_all --quick` success after `--update-golden` (the proc
  prelude's `pruv_proc_spawn_in` gained a `quiet` flag, so every process and event-loop program's Rust output
  changed; mutant digests after the new test files shifted with their numbering); `--list-rows` diff (28 new rows,
  `tests/run_all_rows.txt` updated).
- Audit: one review round (Codex 2 account; the default account was at its limit). Fixed: an LLVM Vec element
  index of type `u8`/`u16`/`u32` was passed unwidened to the i64 shim argument (test `lgaps14_vec_mem_narrow`);
  a trigger in a reverse-range invariant kept the loop variable and failed `type/unknown_name` (test
  `lgaps14_for_rev_trigger`). No findings on `proc_spawn_quiet`. The confirm round
  marked both fixes FIXED with no new required findings.
- Next: switch the mcp/grok/codex/qwen stderr readers to `proc_spawn_quiet`, then 4.4v-RT and LG14-ST.

### Rows after 4.4z

- Closed 4.4d-CL (Grok): `line::entry` ensures `len(e.content) <= len(s)` (carried as `len(e.content) <= i` through
  `content` / `fields` / `part`, the join paid by the key bytes in `part_members`); `feed`'s 100000000 test and the
  `part` 2^40 guard are gone.
- Closed 4.4e-DL (Codex): `str_at`'s 50331648 test is replaced by an erased `assert` linking the `JsonDocument`
  source bound (16777216) to the decoder's 3 x span bound. The 4.4e text saying there is no source bound was wrong.
- No run-time branch added (one Codex structure comparison, default account). Older structure differences remain
  (4.4d-T0, 4.4d-CB, 4.3f-D owned decode, the members loops), so performance was not measured.
- Results on `rs/rust` and `rs/llvm`: grok `test_impl.py` 254/254, `q.py --cases 300` 554 cases 0 unexplained,
  `q_mutants.py --check` 16/16; codex `test_impl.py` 136/136, `q.py 300` 323 cases 0 unexplained, mutant markers
  16/16 (full mutant run not done, 4.4e-VT).
- Still open: 4.4d-T0, 4.4d-CB, 4.3f-D (owned decode), 4.4e-VT.

### Rows after 4.4zb

- Closed 4.3h-D (Claude, hand side): the hand `dropped_user` is the Pruv one-pass byte scan over the raw prefix.
- 4.4h-AE (Claude), shape part closed: Pruv `answer` has the hand's three append branches on `d.reply` in place.
  The early return stays an `if`: new language row LG-4zb-RA, `return` inside a `vec_at_mut` `Some` arm is
  `type/foreign_contract`.
- Closed 4.4h-PL (Claude, hand side): hand `Collector::plan` returns `Plan { kept, bytes }` and `window_packet`
  reserves `title + model + plan.bytes`, like the Pruv `arr_reserve`.
- Closed T1 (Qwen): the hand keeps the raw last custom title and calls `one_line` once; Pruv calls it only when a
  title was found.
- One Codex structure comparison (default account). Remaining differences are listed in the claude and qwen
  sections; structure is not zero, so performance was not measured.
- Results on `rs/rust` and `rs/llvm`: claude test_a0..a4 272/2897/2443/134/270 all passed, `q.py --count 2000`
  2270/2270, `q_measure.py --quick` hand check 36 hand/oracle and 36 Pruv/hand/oracle exact matches; qwen
  `test_impl.py` 27/27, `q.py 300` 0 unexplained (317 same, P1 1, P4 4, P5 1), `q_grid.py --check` 34/34. Both hand
  crates build.
- Open: LG-4zb-RA; claude 4.4h-VD, 4.4c-IS, 4.4h-ST.

## Step 4.4za: codex spawns with a null stderr (4.4v-SN consumer), one shared receive start (4.4v-RT part)

Changes `bench/l3/pkgs/transport`, `codex/rpc.pruv` and a comment in `grok/rpc.pruv` (no compiler change).

- Codex `rpc::start` spawns with `proc_spawn_quiet(binary, args, cwd)` (hand `codex.rs` / `reader.rs`
  `.stderr(Stdio::null())`): the child's stderr no longer reaches a pipe, so a large stderr write cannot block it and
  nothing is buffered in the parent.
- `transport`: the start shared by `receive` and `receive_tail` (scan the buffered bytes with `take`, then `wake`) is
  one private `begin`; the two public functions now differ only in the stderr line (drop or keep in `tl`).
- `receive` still reads stream 2 once per need-more step. Grok spawns with `proc_spawn_env` (GROK_HOME), which has no
  quiet form, so its stderr must be drained (4.4za-EQ). For the quiet codex child the read adds no bytes, but the
  process slot's `ev_eof` is `ends[1] && ends[0]` and `ends[1]` is set only by an explicit read of stream 2
  (`codegen.rs` event-loop read path), so without it `fill` never sees EOF and an early child exit turns from
  `read-failed` into `timeout` (found by the structure review and reproduced before the fix: codex rs/rust 132/136, `list_init_exit`,
  `list_exit_early`, `read_meta_exit`, `read_turns_exit` gave `timeout` instead of `read-failed`; 4.4za-QE).
- MCP keeps `receive_tail` and `proc_spawn`: the TS reports the child's last stderr line, and there is no hand spawn.

### Rows after 4.4za

- Closed: 4.4v-SN for codex (quiet spawn). 4.4v-RT narrowed: the repeated start is one `begin`; `receive` and
  `receive_tail` stay two functions because MCP keeps the stderr tail and the other two still read stream 2.
- New 4.4za-EQ (gap): no spawn with both an environment and a null stderr (`proc_spawn_env` has no quiet form); the
  grok hand `load_check` uses `.env("GROK_HOME", ..)` with `Stdio::null()`. Grok still reads and drops stderr.
- New 4.4za-QE (gap): in an event loop a quiet child's `ev_eof` needs one `ev_proc_read` on stream 2, although
  R-6l-3 says stream 2 is at end of file from the start. If `ev_add_process` marked a quiet slot's stream 2 as
  observed at end of file, codex `receive` would drop the stream-2 read (hand reads stdout only).
- Still open: 4.4v-PB, 4.4v-TR, LG14-ST and the rows listed before.
- Tests (rs/rust and rs/llvm) on the final source: codex 136/136, grok 254/254, mcp 22/22 (the transport consumers;
  opencode, supervisor, claude, json and qwen do not import `transport`). Not measured.
- Review: one structure review (Codex gpt-6.2-astra) of the first form, where `receive` read stdout only. Its one
  required finding (no EOF for the quiet codex child) is fixed above; grok and mcp paths unchanged, `begin` contract
  sufficient. Confirm review (same engine): FIXED, no new required findings.
- Next: 4.4za-QE and 4.4za-EQ in a language bundle, then drop the stream-2 read for quiet children.

### Rows after L-gaps 15

- Closed LG-4zb-RA (R-6e-24): a `return` inside a `map_get_mut` / `vec_at_mut` `Some` arm is legal. The verifier
  writes every enclosing arm binder back into its collection, innermost first, before it proves the `ensures`
  (the Rust `&mut` borrow ends at the `return`). `return move d.f` stays `type/foreign_move`, `break` / `continue`
  out of the arm stay `type/loop_jump_map` and a `return` inside a map iteration stays `type/foreign_contract`.
  Rust and LLVM code is unchanged (`get_mut` match, in-place element slot). The old reject test
  `r6e_4_return_in_arm` is now an accept test. Claude `answer` takes the hand's early return
  (`if len(text) == 0 || d.used >= 32768 { return; }`), which closes 4.4h-AE.
- Closed LG14-ST (R-6q-2): `index(x)` in a quantifier `trigger` of a slice-for invariant names the hidden counter,
  like the rest of the invariant (`for_index_rewrite` now walks triggers; before, it was `type/unknown_fn index`).
- 4.4y-PW, language half closed (R-6e-3): no compiler change was needed. In a contract `docs[k]` of a `Vec<S>` is
  the element, so `forall k in 0..len(docs): p(docs[k])` survives `vec_push` and is instantiated at a
  `vec_get::<S>(docs, k)` call argument (tests `lgaps15_vec_elem_pred`, `_lost`). The bench half (qwen parts walk
  in replay) is still open. New row LG15-MB: a matched `JsonParse::Parsed(doc)` binder cannot go into the Vec,
  because `move d` of an owned-aggregate struct binder is `type/foreign_move` ("move of a non-collection value";
  R-6e-12a moves only array and `str` binders) and a plain binder payload is "an aggregate collection payload needs
  a constructor, call result or move". The qwen move needs LG15-MB first.
- Review (one round, Codex) found two required problems, both fixed with tests. (1) A mutable arm opened through
  a place alias (`let a = inout c; match vec_at_mut(inout a.docs, i)`) wrote its binder back onto the alias's
  stale value, which reset a sibling field written through `c` inside the arm, so a wrong `ensures` was accepted.
  The bug was older than the return change and was there at the arm's end too. Now `store_collection_place`
  rebuilds the path on the alias target's current value (`lgaps15_arm_return_alias`, `_alias_lost`). (2) A
  quantifier that binds the loop's own name had its `index(x)` renamed too; its body and triggers are now left
  alone (`lgaps15_for_slice_trigger_shadow`).
- Checks after the fixes: `cargo test --release` 163 passed; `run_all --only must_reject` 1380/1380; `--only g8_frontend
  --backend both` 39/39; `spec_trace --only-rs` 2407/2407; runtime `--backend both` tests/spec 384/384;
  `diff_rust --fuzz-count 20` and `m3_all --quick` match golden (no golden update); `--list-rows` diff (10 new rows,
  `tests/run_all_rows.txt` updated). Fix-check review: both findings closed, no new required problem. L3 on the final
  binary: claude a0..a4, json a0..a5, supervisor a0..a3, grok, qwen, mcp, codex and opencode passed.
- Open: LG15-MB, then the 4.4y-PW bench half; 4.4y-SB; claude 4.4h-VD, 4.4c-IS, 4.4h-ST; 4.4s-FK, 4.4v-RT.

### Rows after L-gaps 16

- Closed 4.4za-QE (SPEC R-6l-3): a quiet child (`proc_spawn_quiet`, `proc_spawn_env_quiet`) has its stream 2 marked
  at end of file when it joins an event loop, in both backends, so `ev_eof` needs no stream-2 read.
  `transport::receive` no longer reads stream 2 (its callers, codex and grok, spawn quiet children); `receive_tail`
  still reads both streams for MCP, whose child keeps a piped stderr.
- Closed 4.4za-EQ (SPEC R-6l-4): `proc_spawn_env_quiet(program, args, cwd, names, values)` is `proc_spawn_env` with
  stderr set to the null device (hand `Command::env(..).stderr(Stdio::null())`); it has the same proof rules as
  `proc_spawn_env`. Grok `rpc::start` spawns with it, so its stderr drain is gone.
- Closed LG15-MB (SPEC R-6e-12b): in a consuming match, `move d` of a struct binder such as `Parse::Parsed(doc)`
  hands the whole payload over (for example into `vec_push`). A second use, a whole move after a field move and a
  move in a loop of the arm stay `type/foreign_move` / `type/foreign_loop_move`; a struct binder of a non-consuming
  match and an enum binder are `type/array_alias`. Rust moves out of the by-value scrutinee; LLVM takes the payload
  and stores zero into that field of the scrutinee's slot, so the scrutinee's drop frees nothing twice. R-6g8-4 now
  points at R-6e-12b for a whole move, and `r6e_16_reject_dead_enum_binder_move` (a borrowed scrutinee) now expects
  `type/array_alias`.
- 4.4y-PW bench half (qwen keeps the parsed documents) is unblocked by LG15-MB but not done in this bundle.
- Review (one Codex pass) found three required problems, all fixed in this bundle: a whole move of a struct binder
  with no owned part gave bad LLVM IR (now a plain copy; a binder with a boxed field is refused whole, `array_alias`);
  a nested `match` on an enclosing arm's scrutinee of a plain enum accepted a move (`array_alias` now); a quiet
  slot's end event waited for the stream-2 reader thread (the queue's stream-2 end is now preset as well). Tests
  `lgaps16_rt_binder_scalar`, `lgaps16_binder_record_nested_scalar`, `g10a_r23_quiet_exit`.
- `g10a_r23_quiet_ev` and `g10a_r23_quiet_eof` wait 20 s per `ev_wait` instead of 2 s: under the parallel harness the
  first exec of the freshly built fixture child stalled past 2 s (no bytes at all), while the whole run still ends
  in about 6 s, so it is start-up latency and not a lost wakeup.
- Open rows from the review (optional, not done): LGAPS16-DC dead code after `return` in the new arm move path,
  LGAPS16-HN a test helper reuses a handle name, LGAPS16-WB `r6g8_4_binder_whole` could pin the exact error code,
  LGAPS16-SW R-6e-12b wording for a struct holding a `str`.
- Checks on the final compiler: must_reject 1390/1390, g8_frontend both 39/39, runtime both 3581/3581,
  g10_proc_frontend 36/36, g10_async_frontend 44/44, g10_proc_proof 20/20, spec_trace 1/1, cargo test 163 passed,
  diff_rust fuzz 20 and m3_all --quick ok after the intended golden update (runtime text and new samples).
