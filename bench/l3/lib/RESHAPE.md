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

- New (bench): 4.4e-LP, 4.4e-EC, 4.4e-NU, 4.4e-FB (the `InputTooLarge` folder bound stays for the `req_list` size proof), 4.4e-DL (`doc_local`
  gives no source length bound, so `str_at` keeps its test), 4.4e-VT (codex verify time).
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

