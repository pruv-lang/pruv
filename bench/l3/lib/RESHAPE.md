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
| `ent_packet` | `hand/ent_packet.rs` (new in 3.2) | K, E, O, S, V, L, P-hi, P-start |

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
