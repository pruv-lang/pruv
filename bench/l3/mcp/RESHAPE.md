# MCP stdio check: Rust-shape reset (RUST_SHAPE_RESET §6 step 4.1)

The product path `root.pruv::mcp_check` → `packet.pruv` / `util.pruv` is rewritten in the shape of hand Rust. Pruv adds
only proofs. The result packet (`[i64]`, 10 slots per tool) stays: hand Rust uses the same representation
(`perf/hand_packet.rs` `core_hand`, §3 "keep" list). What went is every per-line, per-field and per-tool copy that hand
Rust does not make.

- Product modules (`root.pruv`, `packet.pruv`, `util.pruv`) no longer build a descriptor packet (`plan`) or any step
  wire. The only word packet left is the result packet itself.
- Harness adapters stay under `perf/` (`perf_root.pruv`, `perf_driver.rs`, `old_packet.pruv`) and `harness.rs`.
- `bench/l3/lib` and `bench/l3/pkgs` are unchanged (no consumer runs needed).

## 1. Structure table (§1.5 form)

The call path runs from `mcp_check` (product entry) to the owner of the result (`p`, returned to the host).
"hand" is the shape a Rust programmer writes for the same job. For the packet core this is `perf/hand_packet.rs`
`core_hand`. For the request, line and stderr steps it is the obvious `Vec<u8>`/`&[u8]` code (there is no separate hand
file for the I/O loop; the host side `host/` is shared).

| step | Pruv before | Pruv now | hand Rust | bytes copied / allocations (now vs hand) |
|---|---|---|---|---|
| request line (`request`) | `String` push ×N, `bytes(str)` copy, `join` ×4 for the cursor (each a fresh buffer) | one `alloc(256+cursor)`, `put` of literals (no alloc, R-6g-12), `copy_range` of the cursor token from `d.source`, `truncate` | `Vec::with_capacity`, `extend_from_slice` ×N | 1 alloc each; same |
| `notifications/initialized` | `bytes(lit)` | `lit(lit)`: one exact-size alloc | `b"...".to_vec()` (send takes an owned buffer) | 1 alloc each; same |
| per line: trim + parse | copy `line[..n]` to `b`, `trim(b)` copies again, `parse_owned` | `util::trim_span` (bounds only), `lines::parse_copy(line,lo,hi)`: one exact-size copy moved into the document | `parse(&line[lo..hi])`, the document borrows the line | 1 copy of the trimmed line vs 0 (gap B, below) |
| reply to a server request (`reply`) | `join(bytes(lit), raw(d,mid))`, `join` again | `raw_span` (bounds only), one `alloc(256+id)`, `put` + `copy_range`, `truncate` | `Vec::with_capacity` + `extend_from_slice` | 1 alloc; same |
| id check | `id_matches(raw(d,mid), id)`: copy of the id token | `id_matches(d.source, start, end, id)` on the span | compare `&src[s..e]` | 0; same |
| tools/list page | `has_work`, then `plan` (descriptor buffer `4+10*n`), then `write(move p, d, plan)` (resize) | `packet::page(inout p, d, tools, count, objects)`: one pass, `p = resize(p, len+2*array_bytes)` once, name decoded straight into `p` and checked in place (`put_name`), title/description via `put_field`, hints, raw bytes widened in place; returns `end`, the caller does `p = packet::shorten(move p, end)` (in place) | `core_hand`: `Vec::with_capacity(len+2*array_bytes)`, `extend_from_slice(p0)`, decode/push, `truncate` | Pruv: 1 grow in place (Rust backend `&mut Vec` + `resize`, LLVM `pruv_regrow_v2`), no prefix copy; hand: 1 alloc + prefix memcpy. Pruv zero-fills the reserved tail (gap R below) |
| field lookups per tool | `fields4` (1 walk for 4 keys) + 1 walk for 2 hints | same | 6 walks (one per key) | fewer walks than hand |
| nextCursor | `string_units(d,cur)` (decoded copy) for an emptiness test, `raw(d,cur)` copy for the next request | `str_span` (bounds only); empty ⇔ `end-start <= 2`; the next request copies `src[cs..ce)` once | `&src[s..e]` | 1 copy (into the request) vs 1; same |
| error message | `string_units` (owned units) → `append(p, units)` (fresh `len(p)+n` buffer) | `put_message(inout p, d, id)`: `resize` by the inner bytes, decode in place, `truncate` (fallback to `string_units` only above 16 MB) | decode into `out` | 1 grow in place vs 1 push run; same |
| stderr last line | `last_line` copy → widen to a new `[i64]` → `append` (fresh buffer) | `last_line` returns `Bounds`; `append_bytes(inout p, err, lo, hi)` widens straight into `p` | `out.extend(err[lo..hi].iter().map(..))` | 0 intermediate copies; same |
| result owner | `p` returned to the host | same | same | — |

Field accesses: every tool reads the same tape fields as `core_hand` (`kind`, `next`, `parent`, span start/end), plus
the validation that hand skips (`valid`: kind range, parent, span). Removed by proof: the per-unit capacity tests and the
`src[p]` bounds checks of hand's `decode`/`push` (`put_units_room`, `put_name` store unchecked by contract).

### Differences that remain, and why

| difference | reason | kind |
|---|---|---|
| one copy of each trimmed line into the document | `JsonDocument` cannot borrow `line` on the LLVM backend; `ref` structs (R-6g-11) are Rust-backend only, and the MCP product path is built for both backends | gap B (language) |
| the reserved tail of `p` is zero-filled by `resize` and then overwritten | `resize` must give a fully defined array; hand reserves capacity only | gap R (language) |
| `lit` allocates for the one literal line | `send` takes `[u8]`; there is no borrowed `[u8]` view of a `str` literal | gap L (language) |
| the error path `put_message` ends with `p = truncate(p, ..)` on its inout `p`, which copies the kept words | truncate-assign is in place only as `return truncate(local, m)` (gap T); the error path runs once per check, so it was left as is | gap T (lowering) |
| `put_name` decodes before it knows the name is usable, then leaves the units behind the cursor | same as hand (tentative decode, `truncate`); Pruv does not truncate because the cursor is the logical end | none |

### LB5b: line-by-line table against `perf/hand_packet.rs` (rule of 2026-11-24, RUST_SHAPE_RESET §7)

The LB5 table (git a43e180d) still had shape differences (`put_name` tested each byte twice before the push,
`fields4` ran `key_is` four times per member where hand tests the length first, `put_field` stored `slot+1 = 0`,
`page` stored three hint defaults). LB5b rewrote `packet.pruv` so that every helper of `core_hand` has a Pruv function of
the same name and the same statements, in the same order. `usable`, `cut`, `valid`, `Hits`, `key_is`, `fields4`,
`usable_span`, `put_units`, `put_units_room` and `put_name` are gone. Root and the perf wrapper now find `result` and
`tools` with `packet::field` (hand `field`), not `util::field` (`access::field`).

Kinds: `=` same statements; `E1` a check that hand gets from an index panic or a malformed-tape loop (Pruv has no
panic path, so the test is explicit; it is never taken on a well-formed tape); `E2` ABI of the measured entry;
`P` proof text only (`assert`, `invariant`, contracts), no generated code.

| hand | Pruv | difference | kind |
|---|---|---|---|
| `span` (inline) | the same four tests inline in `field`, and `access::raw_span` in `str_span` / `page` | none | = |
| `hex` | `hex` | none | = |
| `decode` | `decode`: same loop test `p < stop && len(out)-base < limit`, ASCII push, escape table, `\uXXXX`, UTF-8 lead table, `bad` flag, `65533`, surrogate pair | invariants only | = (P) |
| `key_eq`: plain test, `inner == key` | `key_eq`: same plain scan, length test + `eq_range` (one slice compare, R-5 `eq_range`) | none | = |
| `key_eq`: `Vec::with_capacity(key.len()+1)`, `decode`, zip compare | `alloc(0,0)` + `arr_reserve(len(key)+1)` (one exact allocation), `decode`, early-exit loop | none | = |
| `field`: `t[6obj] != 6`, `stop`, member walk, last match wins, `j = t[6(j+1)+3]` | same walk | `obj` range and `stop <= len/6` tests; `v >= stop` and `next <= j` stop the walk | E1 |
| `is_unit_ok` | `unit_ok` | none | = |
| `str_span`: `id?`, `t[6id] != 4`, `span` | `str_span`: `access::kind(d,id) != 4`, `raw_span` | `kind` tests the id range | E1 |
| `put_field` | `put_field` (`Option<Span>` is `Span.status`) | none | = |
| `field(0,"result")`, `field(r,"tools")` | `packet::field` twice, in root.pruv and `perf_root::core_new` | none | = |
| kind 5 test, `stop > a && stop <= len/6`, `reserve = 2*(span bytes)` | same, at the top of `page` | none | = |
| `Vec::with_capacity(p0.len()+reserve)` + `extend_from_slice(p0)` | `out = alloc(0,0)`, `arr_reserve(inout out, len(p)+reserve)`, `arr_append(inout out, p, 0, len(p))`, `p = move out` | none | = (was `arr_reserve(inout p, reserve)`: realloc growth of the input kept a second 16 KB page resident, measured) |
| `while j < stop && seen < 1048576 && objects < 500` | same, plus `0 <= j` | `0 <= j` | E1 |
| loop head: `cur`, `j = t[6j+3]`, `seen`, kind 6, `objects`, `count >= 500` | same | none | = |
| name: `decode(.., 201)`, nonempty, `<= 200`, all `is_unit_ok`, else `truncate(at)` | same (`p = truncate(p, at)`, R-6g4-3) | `assert`s | = (P) |
| name slots, title (16000000, nonempty), description (65536) | same; the cap is `desc_cap()` (`{ return 65536; }`, the `description-cap` mutant's anchor) | none | = |
| hints: `readOnlyHint == true` → `1`, `base+7 = -1`, `destructiveHint` 1/2 → 0/1 | same | none | = |
| raw: invalid span → empty, `len <= 16000000 && out.len() <= MAX - len`, slots, widening `extend` | same, `arr_append` (R-6g4-6) | none | = |
| `count += 1` | same | none | = |
| (error message, not in `core_hand`) | `put_message`: exact `arr_reserve`, `decode`, two slot stores | none | = |
| (stderr tail, not in `core_hand`) | `append_bytes` = `arr_append` | none | = |

Unexplained differences: 0. One contract followed hand rather than the old proof bound: hand appends the name with no
`MAX_WORDS` test, so `page` now ensures `len(p) <= 100000000 + 200*count` (root.pruv's loop invariant and the perf
wrapper's bound changed to match) instead of clamping. No language gap was needed, so no row was added to
RUST_SHAPE_RESET §4.

### LB5c: better algorithms on both sides, then line-by-line again (main correction of 2026-11-24)

Rule: an algorithm that hand Rust can also use is not reverted in Pruv; it goes into `core_hand` too, so the two sides
compare the same algorithm. LB5c put the LB5 algorithms back into `packet.pruv` and added them to
`perf/hand_packet.rs`:

* `fields4`: one walk over the members for four keys; each member key is scanned once and, when not plain, decoded
  once (16 units, one more than the longest key) and compared with each key (`tok_eq`). Was: `field` once per key,
  6 walks per tool (4 tool keys + 2 annotation keys).
* `put_units`: stop at `cap` units, keep the low unit of a split astral scalar (`spill`) or peek the next unit, drop
  the high unit when the cut would split a pair. Was: decode `cap+1` units, then cut.
* `put_name`: each unit is checked as it is pushed (stop at the first bad unit or the 201st). Was: decode 201 units,
  then scan them.
* `step`: the escape / `\uXXXX` / UTF-8 / 65533 logic of one scalar, shared by `decode`, `put_units` and `put_name`
  (as `jstr::step`).
* Contract stores: on both sides an absent field writes `slot = -1` and leaves `slot+1` untouched; no hint defaults
  beyond `base+7 = -1`.

Output is unchanged (same oracle counts, `run_perf.py check` 0 mismatches). Kinds as in the LB5b table.

| hand (`perf/hand_packet.rs`) | Pruv (`packet.pruv`) | difference | kind |
|---|---|---|---|
| `span` (inline) | the same four tests inline in `field` / `fields4`, `access::raw_span` in `str_span` / `page` | none | = |
| `hex` | `hex` | none | = |
| `step`: `\\` with `stop-p >= 2`; `\u` with `stop-p >= 6` → hex value, `p+6`; escape table → `p+2` | same statements (`Step { v, next }` is hand's `(i64, usize)`) | none | = |
| `step`: lead table `(n, v)`, `stop-p < n` → `(65533, p+1)`, `for k in 1..n { v = v*64+b%64; if v > 16777215 { bad; break } }`, `bad \|\| v > 1114111` → 65533, `n == 1` → `c` | same; `k = n` in tail position lowers to `break` (LB5b codegen), `else { k = k+1 }` is the `for` step | invariants (`bad ==> k == n`, `!bad ==> v <= 16777215`) | = (P) |
| `decode`: `while p < stop && len-base < limit`, ASCII push, else `step` + one or two pushes | same | invariants | = (P) |
| `put_units`: `while p < stop && k < cap`, ASCII push `k+1`, else `step`; astral: high, `k+1`, low if `k < cap` else `spill` | same | invariants | = (P) |
| `put_units`: `k == cap` → `nx = spill`, else peek `step(..).0` (high unit if astral), `last = out[at+k-1]`, low after high → `truncate(at+k-1)`, `k-1` | same (`out = truncate(out, at+k-1)`, in place, R-6g4-3) | none | = |
| `key_eq` (root lookups only) | `key_eq` | none | = |
| `field` (root `result` / `tools` only) | `field` | `obj` range, `stop <= len/6`, `v >= stop`, `next <= j` | E1 |
| `tok_eq`: plain → `&src[p..q] == key`; else length + zip compare | plain → length test + `eq_range`; else length + early-exit loop | none | = |
| `fields4`: `t[6obj] != 6` → none; `stop`; `j = obj+1`; `u = Vec::new()` | same; `u = alloc(0,0)` | `obj` range and `stop <= len/6` tests | E1 |
| `fields4` per member: `span` valid → `(p,q)`, plain scan (early exit), `!plain` → `u.truncate(0)` + `decode(.., 16)`, four `tok_eq` → `h[i] = j+1` | same order, same statements; plain scan stops with `i = q` (lowers to `break`) | `v >= stop` stop | E1 |
| `fields4`: `j = t[6(j+1)+3]` | same | `next <= j` stop | E1 |
| `is_unit_ok` | `unit_ok` | none | = |
| `str_span`: `id?`, `t[6id] != 4`, `span` | `access::kind(d,id) != 4`, `raw_span` | `kind` tests the id range | E1 |
| `put_name`: per unit: ASCII → `is_unit_ok` (else `truncate(at)`, None), push; else `step`, BMP → `is_unit_ok`, push; astral → two pushes; `len-at > 200` → `truncate(at)`, None | same statements, -1 for None | invariants | = (P) |
| `put_name`: empty → None, else `Some(len-at)` | same | none | = |
| `put_field`: `slot = -1`, `let Some(sp)` else return, `at`, `n = put_units(..)`, `(!nonempty \|\| n > 0) && at <= MAX-n` → `slot, slot+1`, else `truncate(at)` | same (`Option<Span>` is `Span.status`) | none (`slot+1` untouched when absent, both sides) | = |
| reserve, fresh buffer, loop head (`cur`, `j`, `seen`, kind 6, `objects`, `count >= 500`) | as in LB5b | `0 <= j` | E1 |
| `h = fields4(cur, "name", "title", "description", "annotations")`, `at`, `n = None`, `str_span(h[0])` → `put_name`, `let Some(n) else continue` | same; `if n >= 0 { .. }` is the `continue` | `assert`s | = (P) |
| name slots, title (16000000, nonempty), description (65536), `base+7 = -1` | same; the cap is `desc_cap()` | none | = |
| `if let Some(a) = h[3]`: `ah = fields4(a, "readOnlyHint", "destructiveHint", "", "")`; `ah[0]` true → `base+6 = 1`; `ah[1]` false/true → `base+7` 0/1 | same | none | = |
| raw JSON, `count += 1` | as in LB5b | none | = |

Pruv-only differences left: E1 (an explicit range / malformed-tape test where hand would panic or loop) and P (proof
text). Unexplained differences: 0. No language gap was needed (no RUST_SHAPE_RESET §4 row).

### 4.1 recheck (2026-11-26, RUST_SHAPE_RESET §6 step 4.1)

On today's main (after 3.8 and P9a-5) the LB5c line table still holds; no code changed in this recheck.

| check | result |
| --- | --- |
| package sources | `root.pruv` imports `json`, `access`, `lines`, `tail`, `jstr`, `args`, `transport` through `pruv.json` (all `../pkgs/*`); no `lib/` JSON copy is used. `packet.pruv` and `util.pruv` are local |
| gap R (fill before write) | closed by `arr_reserve`/`arr_append`: `page` (LB5b) and `put_message` reserve exactly and push; no `resize` left on the product path |
| gap T (truncate-assign copies) | closed: `out = truncate(out, ..)` on a local or inout is in place (R-6g4-3); `put_name`, `put_field`, `put_units` use it. `shorten` stays only for the perf harness (`perf_root`) |
| gap L (literal as `[u8]`) | closed: `transport::send(.., "{...initialized...}\n", ..)` passes the literal; there is no `lit` copy left |
| gap B (one copy per trimmed line) | still open in this bench. The language side exists now (borrowing structs on LLVM, R-6g9-2), but closing it needs a borrowed document in `pkgs/json`/`pkgs/lines` (`parse_view`), a shared-package change with all its consumers; left for its own step (lib/RESHAPE.md Next) |
| `0 <= j`, `obj` range, `stop <= len/6`, `v >= stop`, `next <= j`, `kind` range | E1, unchanged: these are the proof-only bounds the LB5c table lists; P9a value `if`/`match` does not remove them |
| `test_impl.py`, rs/rust and rs/llvm | verified, 22/22 each |

## 2. Language gaps hit

LB5 closed gaps R, T, G and L (SPEC §6g4: R-6g4-1 `arr_reserve`, R-6g4-2 `arr_push`, R-6g4-3 in-place truncate,
R-6g4-4 exact resize growth, R-6g4-5 literal as `[u8]`) and added R-6g4-6 `arr_append` for hand's
`extend_from_slice`/`extend`. Gap B stays open (D-MCP-3). The table below and the §1 step table above are the record from before LB5 (they describe the old `resize` form).

| id | gap | where | minimal language change (Rust can do it) |
|---|---|---|---|
| B | A borrowed document over the line buffer (`Doc<'a> { src: &'a [u8], tape }`) on the LLVM backend | `root.pruv` per line, `parse_copy` | Extend R-6g-11 `ref` structs to the LLVM backend (a borrowed `[u8]` field is a (ptr, len) pair with the same lifetime check), then `lines::parse_view(line, lo, hi) -> JsonView` |
| R | `resize` fills the tail; no "reserve then write" | `packet::page`, `put_message` | Lower `resize` without the fill when every tail index is proven written before the first read, or an append form for owned arrays (`push`/`extend` on `[i64]` with a capacity hint) |
| T | `x = truncate(x, m)` copies the kept prefix (`.to_vec()`) on a `let mut` local and on an inout parameter; only `return truncate(local, m)` shortens in place (SPEC §truncate says "a local at its last use") | `packet::page` (was the last line; moved out), `put_message` | Treat `x = truncate(x, m)` like `x = resize(x, n, v)` (R-6e: in place on a `let mut` local or a resizable inout). Workaround used: `packet::shorten(take p0, m) { let mut p = move p0; return truncate(p, m); }` |
| G | `x = resize(x, n, v)` lowers to `Vec::resize`, whose reserve is amortized (`max(2*cap, n)`); hand's `with_capacity(n)` is exact | `packet::page` at small pages (1 tool: 80160 vs 48384 bytes) | Lowering only: `reserve_exact(n - len)` before the resize (no language change) |
| L | No borrowed `[u8]` view of a `str` literal | `root.pruv::lit` for `send` | Allow a `str` argument (literal or value) where a read-only `[u8]` parameter is expected (UTF-8 bytes, no copy), or a `bytes_of(s) -> [u8] view` |

## 3. Results

### Oracle (behaviour equals TS)

| check | result |
|---|---|
| `test_impl.py` (py/rust, py/llvm, rs/rust, rs/llvm) | all 4 pairs verified, 22/22 each (final code) |
| `q.py` vs the Node reference (2040 cases, 4 pairs) | 0 unexplained on every pair: 2027 exact + known TS differences: exact-decimal-id 3, invalid-utf8 3, byte-line-cap 3, numeric-signal 3, surrogate-safe-cut 1 |
| `q_mutants.py` (12 mutants) | 12/12 killed: 2 verifier-rejected (page-contract, tool-contract), 10 differential-killed. Run before the `shorten` move (the mutated lines are unchanged by it) |
| LB5b re-run | `q.py` on the LB5b code: the same counts on all 4 pairs. `q_mutants.py`: the `destructive` anchor moved to `== 2 { p[base+7]=1; }`; result in the LB5b report |
| LB5c re-run | `q.py` on the LB5c code: the same counts on all 4 pairs (2027 exact + the same known TS differences). The LB5b `destructive` anchor (`== 2 { p[base+7]=1; }` → `0`) survived: none of the 22 fixed fixtures gives a distinguishing output for the `true` branch alone. LB5c restores the original mutant (both values → -1): `{ p[base+7]=0; } else if .. == 2 { p[base+7]=1; }` → `-1`, `-1`. `q_mutants.py` on the LB5c code: 12/12 killed (2 verifier-rejected, 10 differential-killed, `destructive` included) |
| `run_perf.py check` (new vs old vs hand, 15 pages + 3 chains) | 0 mismatches |

Known edges that no oracle case reaches: a nextCursor over 20 MB (the old `join` quirk) and the room clamp at
100000000 words; both are handled by the proofs' bounds, not by tests.

### Counters (`run_perf.py counters`, `perf/work/counters.json`)

Visits and output stores are linear (residual < 0.07%). No descriptor stores, no prefix copy, no rewrite beyond the
10 header slots per tool (19.5 per tool). `new_alloc_bytes` fails the linear gate only through the 1-tool cell
(80160 vs hand 48384, gap G); from 10 tools up the bytes equal hand exactly.

### Perf (§7; `run_perf.py perf`, 5 interleaved rounds, RSS from fresh processes)

perf_cells: `bench/l3/mcp/perf/work/perf_cells.json` (tools/perf_gate.py format, not committed: work/ is generated).

| cell | size | time new/hand | allocs new/hand (count) | alloc bytes new/hand | peak RSS new/hand |
|---|---|---:|---:|---:|---:|
| 50 tools | M | 0.96x | 1 / 1 | 1.00x | 1.064x |
| 100 tools | M | 0.94x | 1 / 1 | 1.00x | 1.092x |
| 250 tools | L | 0.93x | 1 / 1 | 1.00x | 1.133x |
| 500 tools | L | 0.93x | 1 / 1 | 1.00x | 1.160x |
| 5x100 (chain) | L | 0.69x | 3 / 5 | 0.58x | 1.052x |

(S cells: time 0.44x to 0.94x, RSS 1.009x to 1.026x; the old per-tool path is 16x to 917x hand.)
`tools/perf_gate.py`: time and allocs pass at every size; FAIL on the 5 M/L RSS cells.

One fix pass was made: the first run had 2 allocations per page and 1.73x alloc bytes because `p = truncate(p,
cursor)` on the inout `p` copied the whole output (`.to_vec()`, gap T). `page` now returns `end` and the caller
shortens in place with `packet::shorten`.

**Line-level cause of the RSS cells over 1.0x** (still over after the fix pass): `packet.pruv` `page`,
`p = resize(p,want,0);` with `want = len(p) + 2*array_bytes`. The Rust backend lowers it to `Vec::resize`, which writes
0 into every reserved word, so the whole reservation becomes resident. `core_hand` reserves the same capacity with
`Vec::with_capacity` and touches only the words it writes. The excess equals the reserved but unwritten tail:

| cell | (want - out words) x 8 | RSS new - hand |
|---|---:|---:|
| 100 tools | 227168 | 262144 |
| 500 tools | 1136504 | 1179648 |

(RSS moves in 16 KB pages.) This is gap R; it needs the language change in §2 (reserve without fill / append form).
A tighter `want` would shrink but not remove it and would leave hand's shape.

### LB5c measurement (final; one `run_perf.py perf` run after the LB5c table reached 0 unexplained, lock `/tmp/pruv-perf.lock`)

perf_cells: `bench/l3/mcp/perf/work/perf_cells.json` (not committed). check 18 cases, 0 mismatches. Hand now carries
the same `fields4`/`put_units`/`put_name`/`step` algorithms, so these ratios compare one algorithm in two languages.
Load average was about 7.5-8 during the run (another worktree's tests were running); medians are interleaved, so
both rows saw the same load.

| cell | time new/hand (median) | instr/call new/hand | allocs | peak RSS new/hand |
|---|---:|---:|---:|---:|
| 0 tools | 1.00x | 19508 / 19609 (0.995x) | 1 / 1 | 1.009x (+16 KB) |
| 1 tool | 1.20x | 26899 / 28548 (0.942x) | 1 / 1 | 1.010x (+16 KB) |
| 10 tools | 0.93x | 93382 / 109090 (0.856x) | 1 / 1 | 1.000x |
| 50 tools | 0.93x | 394147 / 472850 (0.834x) | 1 / 1 | 1.014x (+32 KB) |
| 100 tools | 0.92x | 769647 / 927477 (0.830x) | 1 / 1 | 1.006x (+16 KB) |
| 250 tools | 0.91x | 1911002 / 2308755 (0.828x) | 1 / 1 | 1.004x (+16 KB) |
| 500 tools | 0.92x | 3815239 / 4612697 (0.827x) | 1 / 1 | 1.002x (+16 KB) |
| 5x100 (chain) | 0.96x | 5013166 / 5811000 (0.863x) | 5 / 5 | 1.000x |

`tools/perf_gate.py`: FAIL (24 cells; 4 fail = RSS at 50/100/250/500 tools; 3 debt = 0-tool RSS, 1-tool time,
1-tool RSS). Time and allocs pass at every M/L size. Gates on this code: `cargo test --release` ok, `diff_rust.py
--fuzz-count 20` all same, `g8_frontend` 27/27 (py) and 39/39 (rs), `q.py` 2027 exact on all 4 pairs, `q_mutants.py`
12/12 killed. Still open, unchanged from LB5b below: item 4 (inline-policy code size, the +16 KB code page) and item
5 (1-tool time 1.20x with 0.94x instructions, 3 timer ticks).

### LB5b measurement (superseded by LB5c; `run_perf.py perf` at commit time, lock `/tmp/pruv-perf.lock`)

perf_cells: `bench/l3/mcp/perf/work/perf_cells.json` (not committed). check 18 cases, 0 mismatches.

| cell | time new/hand (median) | instr/call new/hand | allocs | peak RSS new - hand |
|---|---:|---:|---:|---:|
| 0 tools | 1.00x | 0.994x | 1 / 1 | +16 KB (1.009x) |
| 1 tool | 1.19x | 0.962x | 1 / 1 | +16 KB (1.010x) |
| 10 tools | 0.91x | 0.911x | 1 / 1 | +16 KB |
| 50 tools | 0.90x | 0.898x | 1 / 1 | +16 KB (1.007x) |
| 100 tools | 0.90x | 0.896x | 1 / 1 | +16 KB (1.006x) |
| 250 tools | 0.90x | 0.894x | 1 / 1 | +16 KB (1.004x) |
| 500 tools | 0.90x | 0.894x | 1 / 1 | +16 KB (1.002x) |
| 5x100 (chain) | 0.93x | 0.915x | 5 / 5 | +16 KB (1.001x) |

`tools/perf_gate.py`: FAIL (5 M/L RSS cells), S cells over 1.0: 1-tool time, 0/1/10 RSS. Not done; the causes:

1. **Compiler, found and fixed: loop exit.** LB5 instruction excess (about 1100 per tool) was a `while` whose exit
   assignment (`i = q`, `j = stop`) left the loop running to its head test. Both compilers now emit `break` when the
   assignment is in tail position and sets the loop variable to the bound of the loop test (`codegen.rs`
   `exit_bound`/`same_bound`/`loop_exits`, `pruvc/codegen.py` `_exit_bound`/`_same_bound`/`loop_exits`). Instructions
   went from 1.11x to 0.89x hand.
2. **Harness, fixed.** The driver kept a dead borrowed start packet alive for the new row (+40 KB in the RSS probe) and
   timed the new row freeing its input while hand borrowed it. Now new and hand both own and free their input.
3. **Source shape, fixed.** `page` grew the input with `arr_reserve` (realloc) where hand builds a fresh buffer; the
   wrapper looked up `result`/`tools` with `util::field`. Both now follow hand (table above).
4. **Compiler, found, not fixed: inline policy code size (the +16 KB).** `pruvc/inline_policy.py` /
   `inline_policy.rs` promote `decode` (98 lines, 3 sites), `field` (37 lines, 7 sites), `key_eq` and `put_field` to
   `#[inline(always)]` because `incl * (sites - 1) <= INLINE_BUDGET` (1500 lines). `run_new` becomes 18104 bytes of
   text against hand's `core_hand`+`decode`+`field`+`put_field` of about 5500, one more 16 KB code page resident.
   Evidence: the same `gen.rs` with `#[inline(always)]` removed from `core_new` and the `packet` functions only
   (`/tmp/lb5rss/v2`): RSS equal to hand at 0, 1, 50, 100, 250 tools and 16 KB under at 500; time 0.92x at 100 tools.
   The fix is a global policy change (budget in code bytes rather than lines, or no promotion of a fn with a loop
   and more than one site) that needs a re-measure of every bench; it did not fit the time box.
   **LI1 tried (2026-11-24, branch `lair/compiler-bundle-li1-inline-pol-aad0`, 8e5ad72a, not merged):** "no forced
   promotion for a fn over 64 lines with a loop" in both compilers. RSS became equal to hand in every cell, but
   instructions per call rose 10-12% at 100 and 500 tools (3.82M -> 4.29M) and 500-tool time went 0.91x -> 1.03x.
   Putting `packet::field` alone back to `always` gives 3.57M (6% under the old compiler) with hand RSS; putting the
   hot `step` back gives 4.77M. So the cost is not in the demoted bodies: the caller size changes how rustc inlines
   `step` inside `page`. A rule on callee size and loops cannot separate these three fns; the next attempt needs the
   `page` assembly and a caller-side rule. The +16 KB is a fixed share (code page), so it stays a debt row, not a gate.
5. **Open: 1-tool time 1.19x** with 0.96x instructions (1000 vs 834-875 ns, 3 timer ticks); not located.

### LB5 measurement (superseded: taken on the LB5 shape, before LB5b; kept for the record)

check 18/18 cases 0 mismatches; `test_impl.py` 22/22 verified on py/rust, py/llvm, rs/rust, rs/llvm. Counters: no
prefix copy, no zero-fill words, alloc count and bytes equal hand in every cell (1-tool 48384 = hand, gap G closed),
19.5 header rewrites per tool.

| cell | time new/hand (median) | instr/call new/hand | allocs | alloc bytes | peak RSS new/hand |
|---|---:|---:|---:|---:|---:|
| 1 tool | 0.62x | 1.07x | 1 / 1 | 1.00x | 1.019x |
| 10 tools | 1.05x | 1.10x | 1 / 1 | 1.00x | 1.009x |
| 50 tools | 1.15x | 1.13x | 1 / 1 | 1.00x | 1.014x |
| 100 tools | 1.14x | 1.12x | 1 / 1 | 1.00x | 1.011x |
| 250 tools | 1.16x | 1.11x | 1 / 1 | 1.00x | 1.007x |
| 500 tools | 1.20x | 1.11x | 1 / 1 | 1.00x | 1.004x |
| 5x100 (chain) | 0.86x | 0.98x | 5 / 5 | 1.00x | 0.791x |

The RSS gap of gap R is gone (500 tools: 1.160x before, 1.004x now; the remainder is 2 pages of 16 KB). Time is
still 1.05x-1.20x on the single pages with an instruction excess of about 1100 per tool (500 tools: 5.47M vs 4.92M),
so it is real work, not noise. The allocation and copy structure is the same as hand. Per the bundle rule this is a
codegen difference to locate line by line, not a debt row; it was not located inside the LB5 time box. Candidates
in the generated `gen.rs`: the per-unit name test in `put_name` (two compares per byte before the push), the four
`key_is` calls per member in `fields4` against hand's early length test, and the non-inlined `jstr::step` call on
escaped bytes. D-MCP-1/2 stay open until this is found.

## 4. Who did what

LB5b: Claude Opus 5.5 (this conversation): line table, packet.pruv rewrite, loop-exit `break` codegen in both
compilers, small-array budget fix, RSS diagnosis, driver symmetry. Delegations: 0b64496f (Claude Sonnet, write; the
candidate was applied by hand after review) and 69975b1f (command, gate run).

LB5 (buffer growth): main conversation Claude Opus 5.5 (spec rules, packet.pruv/root.pruv, structure table).
Compiler work on both implementations: Codex gpt-6.2-sol (high), two write delegations (R-6g4-1..5, then R-6g4-6
`arr_append` and the audit fixes). Audit and confirm review: Codex gpt-6.2-astra (ultra). Gate runs: command
delegations.

## 4.4za: stderr

MCP keeps `proc_spawn` and `transport::receive_tail`: the child's last stderr line is part of the result (TS), and there is
no hand spawn to follow. `receive_tail` now starts with the shared `transport::begin` (no behaviour change).
