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

## 2. Language gaps hit

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

## 4. Who did what

All work in this bundle: Claude Opus 5.5 (main conversation). No delegation was used; the sweep was limited to
`bench/l3/mcp` (packet.pruv, util.pruv, root.pruv, perf/), which was read directly.
