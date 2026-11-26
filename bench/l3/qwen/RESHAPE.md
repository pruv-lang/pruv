# Qwen head, tail and window import: Rust-shape reset (RUST_SHAPE_RESET §6 step 4.3d)

Scope: every function of `root.pruv`, `import.pruv`, `chain.pruv`, `framing.pruv`, `entry.pruv` and `boundary.pruv`
against the hand baseline `handrust/` (serde_json, the shape cited in RUST_SHAPE_RESET §4 ABI). The hand had only
`window_import`; this step adds `handrust/src/edge.rs` (`head_import`, `tail_import`, TS `readQwenHead` / `tailInfo`
over `readEdgeLines`), so every Pruv entry has a hand counterpart. pruvc/, pruvc-rs/, SPEC.md, docs/RUST_PARITY_SPEC.md
and tests/ were not touched; `bench/l3/lib` and `bench/l3/pkgs` are unchanged (no module moved, so no shared-module
consumer runs were needed; `pkgs/turns` is called, not changed).

The JSON reader is the `pkgs/json` tape, not a serde `Value` (row J); field reads go through `pkgs/access`. That
difference is accepted here as it was for Grok.

What went:

- Byte loops the hand does not write. `framing::skip_first` and `framing::window_next` walked byte by byte (window_next
  with a 16-byte min-distance block pre-pass); both are one `find_byte` (hand `split_inclusive`, memchr).
  `chain::capacity` counts LFs with one `find_byte` per line. `chain::same_span` (a byte loop) is gone: `lookup`
  compares `ul == klen && eq_range(...)` (hand `entry.uuid == key`, one slice compare). `import::put_str` and
  `chain::put` (byte loops) are gone: `copy_range` with a `str` source.
- Allocations the hand does not make. `head_result` / `tail_result` built the text in a zero-filled exact-size array
  filled by index; they now `arr_reserve` once on an empty array and `arr_append` (hand `String::with_capacity` +
  `push_str`). `root::title_bytes` filled a zero array by a byte loop; now `arr_reserve` + `arr_append` (hand
  `as_bytes().to_vec()`).
- Copies the hand does not make. `entry` returned `Field::Value(copy(v))` / `Title::Ranked(.., copy(v))` /
  `Block::Piece(copy(x))` on an owned match binder in eight places (a second copy of every decoded field); each is now
  `move`. `read_head` (`cwd`, `title`), `tail_info` (`best`) and `stamp_of` move their owned strings instead of
  copying.
- Dead state. `read_head` / `tail_info` kept an `idle` flag nobody read; `read_head` carried an always-empty `ep` /
  `has_ep` pair into `head_result` (meta slots 8/9 stay -1 from `meta_new`, as before). `entry::array_content`
  counted ignored blocks in `seen` (never read) and matched a `Block::Veto` no function creates; both removed.
- `replay` dispatched on `event.kind` with an else-if chain; it is now `match event.kind { 0 => .., 1 => .., 3 => ..,
  9 => .., 8 => .., _ => .. }` with an inner `match` for 2 / 4 / 6 / prompt-or-answer (R-6u-8).

## 1. Structure table (§1.5 form)

| step | Pruv before (4.3c tree) | Pruv now | hand Rust | copies / allocations (now vs hand) |
|---|---|---|---|---|
| head lines | `edge_next` (`lines::jsonl_next`, final line only when not cut) | same | `data.split(b'\n')`, `pop()` when cut, skip empty | 0 / 0; same |
| head fields | `string_field(cwd)` then `copy(v)`; `human_text` then `copy(t)`; `idle` flag | `move v`, `move t`; no flag | `entry.get("cwd")` clone; `human_text` → `Option<String>` | 1 per field (decode) vs 1 (clone); same |
| head result | zero-filled `alloc(total)`, three `put_str` byte loops (one always empty) | `arr_reserve(total)` + two `arr_append` | `Head { cwd, first_prompt }` | 1 alloc vs 0 (host wire, row ABI) |
| tail lines | `skip_first` byte loop, then `edge_next` | `skip_first` = `find_byte`, then `edge_next` | `split(b'\n')`, `next()` when cut | 0; same |
| tail fields | `iso_of` then `copy(iso.text)`; `title_of` then `copy(text)` | `copy(iso.text)` (row 4.3d-M); `move text` | `iso(timestamp)` owned; `one_line(text)` owned | +1 copy of the 24-byte stamp (4.3d-M) |
| tail title | ranked best kept raw, `one_line` once at the end | same | `one_line` per custom title line | Pruv fewer (difference T1 below) |
| window lines | 16-byte min-distance pre-pass + byte loop | one `find_byte` | `split_inclusive(b'\n')` | 0; same |
| window entries | `Vec`-free event log (`Events`: 9-byte records + text buffer) | same | `Vec<Slot>` of `Entry { uuid, parent, action }` | row VL |
| uuid index | arena + splitmix hash buckets; `put` byte loop; `same_span` byte loop | `copy_range`; `ul == klen && eq_range` | `HashMap<&str, &Entry>`, `HashSet<&str>` | arena 1 alloc vs map; row 4.3d-H |
| chain walk | `resolve` / `lookup` | same | `current_chain` | same steps |
| replay | else-if chain on `event.kind` | `match event.kind` (outer and inner) | `match &entry.action` | same |
| collector | `pkgs/turns` st/sl/bk flat arrays | same | `Collector { VecDeque<Turn> .. }` | row VL |
| window result | wire header by `put_word`, title/model `put_str` loops | `copy_range` for title/model | `Window { .. }` | host wire, row ABI |
| title bytes (root) | zero `alloc(len)` + byte loop | `arr_reserve` + `arr_append` | `to_vec()` | 1; same |

## 2. Differences that remain

- VL (pkgs, open): the hand's `Vec<Slot>` and `VecDeque<Turn>` hold structs with owned strings; Pruv keeps the event
  log (`import::Events`, records + text) and the `pkgs/turns` flat store.
- 4.3d-H (bench): the hand indexes uuids with `HashMap`/`HashSet` of borrowed `&str`. Pruv has no hash map; `chain`
  keeps its arena + bucket index (DB6). Closest form; the compare itself is now a slice compare like the hand.
- 4.3d-M (language): `move iso.text` on a `let`-bound struct (`crate::time::Iso` returned by `iso_of`) is
  `type/array_alias` ("an array field cannot be moved"). TK / LB9 only covers a consuming match binder. `tail_info`
  keeps `copy(iso.text)` (24 bytes per stamped line). Closest form kept, no workaround.
- T1: the hand (and TS) run `one_line` on every custom title line; Pruv keeps the best raw title and runs `one_line`
  once. Output is the same (the last one wins either way); Pruv does less work. Kept.
- ABI: `boundary.pruv` (limits, `check_shape` / `check_output`, `put_word` / `get_word`, `wire_fail`, `wire_packet`,
  `widen`, `pack_result`) and the `ImportResult` meta words are the host boundary; the hand returns structs.
- U / L: string literals passed where the hand borrows `&'static str` stay `str` values.

## 3. Codex structure review (hand vs Pruv, per function) and what was applied

One review, Codex gpt-6.2-astra, effort high, hand `handrust/src/{lib,edge}.rs` against every Pruv function.

Applied: the dead `seen` counter and `Block::Veto` (`entry::array_content`).

Not applied in this step (bench rows, `qwen/` only; each changes several functions and the q mutants, so they did not
fit the step's time cap without unverified code):

- 4.3d-C: `chain::capacity` sizes the uuid index and `import` sizes `selected` by counting LFs first; the hand grows
  its `Vec` / `HashMap` per object line. A file of 8 MiB of bare LFs reserves far more than the hand. Grow per entry.
- 4.3d-T: `import` records title candidates as tag-2 events and picks the title in `replay`, and calls `stamp_of` on
  lines that do not need it; the hand decides the role once per line. `tail_info` keeps `best_rank` / `replaces`
  although `title_of` returns rank 1 only (the `last-title` mutant targets `replaces`).
- 4.3d-O: `boundary::publish` / `make_output` copy meta, turns and text into the output; the hand moves them.
- 4.3d-P: the hand skips a line whose first non-whitespace byte is not `{` before parsing (`lib.rs` `parse_line`);
  Pruv parses and then checks the root kind. Output is the same; Pruv does more work on non-object lines.
- 4.3d-W: `boundary::put_word` / `get_word` are byte loops; `store_le` / `load_le` are the hand form.
- Small: `human_text` looks up `provenance` twice (`access::field` then `field_is`); `title_of` looks up `subtype` even
  when `type` is not `system`; `Leaf.bad` is always false (kept: a q mutant reads it).

Behaviour notes from the review (no Pruv change):

- The hand (serde_json with `arbitrary_precision`) reads a timestamp object whose key is the escaped
  `$serde_json::private::Number` as a number. A hand-baseline artefact; TS and Pruv do not.
- At the 120-unit title cut TS can leave a lone high surrogate; Rust and Pruv drop the whole character
  (`../pkgs/RESHAPE.md`, known difference).
- 4.3d-M: Codex pointed at `grok/rpc.pruv` (`let r1 = move ck.raw; ck.raw = alloc(0, 0);`). That moves a field of a
  `let mut` struct and refills it, an allocation in place of the copy; not the hand shape, so the copy stays.

## 4. Results

After the last code change, `rs/rust` and `rs/llvm`:

| check | rs/rust | rs/llvm |
|---|---|---|
| `qwen/test_impl.py` (verifier + fixtures) | PASS, 26/26 | PASS, 26/26 |
| `q.py` 300 cases (+21 list) | same 316, P1 1, P4 4, P5 1, unexplained 0; list same 21, P4 1 | same |
| hand `edge.rs` vs fixtures (21 files) | 0 differences | - |

P1 / P4 / P5 are the classes `q.py` already explains. No perf timing was run.
