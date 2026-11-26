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
| `ent_state` | `hand/ent_state.rs` (new in 3.4) | OA, ENT, CK, AD (adapters for 3.6 / 3.7), CX partly closed, DR |
| `ent_dispatch` | `hand/ent_dispatch.rs` (new in 3.4) | OA, AD (`find_item` / `list_item` / `read_item`), O (clip code 0); LR closed in 3.5 |
| `ent_bound` | `hand/ent_bound.rs` (new in 3.5) | SA, HK, PB, U, J, O, AD (`threads_item` / `excluded_item`, 3.7) |
| `ent_canon` | `hand/ent_canon.rs` (new in 3.5) | RA, O, OA, AD (by-value entry points for `ent_sources`, 3.6) |

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
- **MP** (new, language gap): `codex_acct` matches the `is_codex_home` result as `Ok(host)` then `Err(h)` with an
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
| F5 rest: `valid_account_id` decodes before / after the length check | kept: `ent_accounts` has no hand baseline yet (3.7) |
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
