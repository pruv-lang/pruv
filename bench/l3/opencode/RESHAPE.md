# OpenCode import: Rust-shape reset (RUST_SHAPE_RESET §6 step 4.3g)

Scope: the OpenCode slice (`util`, `packet`, `path`, `admit`, `sql`, `rows`, `list`, `export`, `entry`, `convert`,
`plan`, `root`) against a new hand baseline `hand/opencode.rs` (std + serde_json + rusqlite; one function per Pruv
function or step group, each marked `// Pruv: module::fn`). The hand opens the database with rusqlite, spawns the
exporter with `std::process`, and parses the export with serde_json; it builds with `cargo build --release` (serde_json,
rusqlite 0.40 bundled) with no warnings. pruvc/, pruvc-rs/, SPEC.md, docs/RUST_PARITY_SPEC.md and tests/ were not
touched; no module moved to `lib` / `pkgs` and no shared module changed, so no consumer runs were needed.

Pruv has no SQLite or file-system host API, so every database and file step is a host request: the Pruv side runs the
step machine (`ctl: [i64]` of 64 words, phases, step and result packets) where the hand makes direct calls (row
4.3g-SQ). The hand header lists the step wire and the `util` pool helpers as having no hand counterpart.

What went:

- Zero-filled buffers that are then overwritten. `plan::fit` allocated a 90-word item array and a 24,000-byte text
  buffer, filled them by index and copied the used prefix out; it now grows `items` / `pb` with `arr_push` /
  `arr_append` and moves them into `PlanValue`. `packet::cat_bytes` / `append_str` / `cat_words`,
  `export::with_span`, `util::to_bytes` and `convert::model_bytes` / `assemble` filled `alloc(n, 0)` by byte loops or
  `copy_span`; each now does one `arr_reserve` and `arr_append` (a `str` source where the input is text).
- Byte loops the hand does not write. `path::join` copied both halves by two loops into an `n + 1` buffer and then
  `truncate`d; it reserves and appends with an optional `/`. `path::contained` and `same_key` compare by `eq_range` on
  `[u8]` (hand slice `==` / `starts_with`). `normalize` builds its result with reserve + push + append, not an
  `alloc` + `copy_span`. `dirname_bytes`, `folder_key`, `util::span_bytes` and `util::raw` return `copy(subview(..))`.
  `convert::put_str` (a byte loop) is gone.
- Flags the hand writes as early exits. `plan::select` kept a `skip` flag for the role, summary and todo checks; each
  is a `continue`. `convert::convert_doc` kept a `bad` flag for an invalid message; it returns `shape_error()` at once.
  `sql::has_name` scanned every column with a `found` flag; it returns at the first match (hand `iter().any`).
  `entry::text_of` kept a `total` counter beside `out`; the loop tests `len(out)` and leaves by `continue`.
- Untyped state. The message attachment `mt: [i64]` (4 slots, `len(mt) == 4` in every contract) is
  `entry::Attach { open: bool, pid, mid }`, passed `inout` through `feed_user` / `feed_assistant` / `feed`.
- Copies of owned locals. `plan::export_at` returned `copy(c)` / `copy(d)` of its own arrays; it moves them.
  `convert::clean_title` returns `one_line(t, 120)` directly. The export title is owned by the call
  (`let mut title = export_title(..)`), not copied once more before the fallback.

## 1. Structure table (§1.5 form)

| step | Pruv before (4.3f tree) | Pruv now (4.3g) | hand Rust (`hand/opencode.rs`) | copies / allocations (now vs hand) |
|---|---|---|---|---|
| database / exporter / files | host step machine, `ctl: [i64]` 64 words | same | rusqlite, `std::process`, `std::fs` | row 4.3g-SQ |
| path join | two byte loops into `alloc(n+1)`, `truncate` | `arr_reserve` + `arr_append` + `arr_push(47)` | `format!` / `push_str` | 1 vs 1 |
| path compare | byte loops in `contained`, `same_key` | `eq_range` on `[u8]` | slice `==`, `starts_with` | 0 vs 0 |
| `key_eq`, `has_prefix`, `lit_eq` | byte loops on `str` | same | `==`, `starts_with` | row 4.3f-S |
| folder key / dirname / spans | `alloc` + `copy_span` | `copy(subview(..))` | `to_owned()` of a slice | 1 vs 1 |
| dirname of a `str` | `to_bytes` / `to_str` round trip | same | `rsplit_once('/')` | row 4.3g-T |
| pool append (`cat_bytes`, `append_str`, `cat_words`) | zero-filled `alloc(a+b)` + two loops | `arr_reserve` + two `arr_append` | `extend_from_slice` on one `Vec` | 1 new array per append vs 0 (row 4.3g-P) |
| column check (`has_name`) | full scan with `found` | return at first match | `iter().any` | 0 vs 0 |
| rows decode | zero-filled `rows` / `pool`, own `grow`, `fr` / `fp` copies | same | `Vec<Row>` push | row 4.3g-R |
| select SQL / pool | `pool_str` chain, 4 buffers in mode 0 | same | one `String`, `format!` | row 4.3g-Q |
| export temp prefix / spawn args | second normalizer, zero-filled `out`, `to_bytes` of literals | same; `with_span` now reserve + append | `join(root, "opencode-")`, `&str` args | row 4.3g-X |
| message attach state | `mt: [i64]` 4 slots | `Attach { open, pid, mid }` inout | `Option<(String, String)>` + `bool` | tape ids vs owned strings (row 4.3g-T) |
| message text | `out` + `total` + `first` | `out` + `first`, `continue` exits | `String` + `first` | 0 extra vs 0 |
| turns store | `st` / `sl` / `buf` flat arrays, `feed` returns 1 to grow | same | `Vec<Turn>` | row VL |
| plan candidate | `skip` flag | `continue` | `continue` | same |
| plan fit | 90-word + 24,000-byte zero buffers, prefix copy out | `arr_push` / `arr_append`, moved into `PlanValue` | `Vec<String>` push | 0 extra vs 0 |
| plan `at` | `copy(at)` | same | `Option<String>` clone | row 4.3d-M |
| plan time | `copy(c)` / `copy(d)` | move | owned return | 0 vs 0 |
| convert loop | `bad` flag | `return shape_error()` | `?` | same |
| model text | `put_str` byte loops | reserve + `arr_append` + `arr_push(47)` | `format!("{p}/{m}")` | 1 vs 1 |
| result packet | zero-filled `40+N+K+T`, `put_*` loops, `result_shape` twice | same | `Vec::with_capacity` + `extend` | row 4.3g-A |

## 2. Differences that remain

- 4.3g-SQ (language / host, new): no SQLite or file-system host API. Every open, query, mkdtemp, spawn, stat, read and
  remove is a step packet answered by the host; the hand calls rusqlite and `std::fs` directly.
- VL (pkgs, open): the turns store is `st` / `sl` / `buf`; `feed` returns 1 to ask for a larger `buf`.
- 4.3f-S (language): `eq_range` rejects `str`, so `convert::has_prefix`, `path::key_eq` and `sql::lit_eq` keep loops.
- 4.3d-M (language): `copy(iso.text)` in `entry::iso_bytes` and `copy(at)` in `plan::fit` stay.
- 4.3f-E (pkgs): `util::elements` builds the id array before the walk.

## 3. Structure review (hand vs Pruv, per function) and what was applied

The Codex review was replaced by the fallback (Claude Opus, high; a delegated Codex run returns only after the turn
ends, so it could not feed back into this step). One read-only review of every function against `hand/opencode.rs`:
about 60 functions match, 21 structure findings, 4 output disagreements. Applied:

- `packet::cat_bytes`, `append_str`, `cat_words` and `export::with_span`: reserve + append instead of zero fill + loops.
- `sql::has_name`: return at the first match.
- `plan::export_at`: move the local arrays.
- Hand side, three behavior differences where the Pruv slice (checked against the pinned TypeScript) is the reference:
  `cell_text` now uses `String::from_utf8_lossy` (the host has already applied lossy replacement, DESIGN), the
  result bound counts body + pool against the bank (Pruv `ok_result`), and `run` decodes the configuration before the
  text-size check and answers with the configured mode (Pruv `opencode_begin`).

Not applied (rows). Each needs more than the time cap allowed or an API change across the step modules:

| row | finding | where | what the hand does |
|---|---|---|---|
| 4.3g-P | every pool append returns a new array (`pool = cat_bytes(pool, ..)`, `util::pool_str` / `pool_bytes` / `pool_dec`), so building a pool copies it once per field | `packet`, `util`, `list::list_result`, `sql`, `rows` | one `Vec` grown with `extend_from_slice` |
| 4.3g-R | `rows::decode` zero-fills `rows` (7 n) and `pool` (256), grows by its own doubling copy, copies through `fr` / `fp`; `relocate` and `sql::p208` copy again into a `lit` array | `rows`, `sql` | `Vec<Row>` push, refs from one pool |
| 4.3g-Q | `select_sql` / `select_pool` chain `pool_str`; mode 0 builds four buffers | `sql` | one `String`, `format!("{filter}/")` |
| 4.3g-X | `join_prefix` is a second POSIX normalizer (two zero-filled `[i64]` arrays and an `out` copy) with an internal error 28 over about 2 MB; `spawn_pool` zero-fills and copies `to_bytes` literals; `on_mkdtemp` builds three buffers | `export` | `join(root, "opencode-")`, `&str` args |
| 4.3g-K | `span_bytes` copies made only to compare or re-copy (`read_handoff` dir / folder, `utility` mode 4, `rows::clean_title` / `updated_number`); a `subview` needs the ref guard in each caller's contract | `list`, `path`, `rows` | borrows `&str` |
| 4.3g-D | `db_none` and `db_path` both read `OPENCODE_DB` (two scans, two copies) | `path` | reads `given` once |
| 4.3g-N | `normalize` sizes its scratch `2n + 4` and builds a second buffer | `path` | `Vec<&str>` + `join` |
| 4.3g-A | `assemble_result` / `ok_result` / `done_wrap` / `build_error` zero-fill and fill by loops; `result_shape` runs twice | `packet` | `Vec::with_capacity` + `extend` |
| 4.3g-F | `format_i64` / `pool_dec` use two buffers; `rows::format_ext` repeats the civil-date arithmetic; `items_at` / `rows_result` zero-fill | `util`, `rows`, `plan`, `list` | `format!`, one `civil` |
| 4.3g-T | `Attach` holds tape ids, not owned strings; `dirname` and `iso_str` round-trip `str` / `[u8]` | `entry`, `path` | `Option<(String, String)>`, `&str` slicing |
| 4.3g-E | spawn failure: Pruv `engine_err` reports native `event[5]`, the raw code and `EngineImportError` in pool words 25 / 26; the hand's `fail(15, 1, 3)` carries none | `export` vs hand `export` / `encode_error` | align the hand with Pruv (the fixtures pin Pruv's form) |

## 4. Results

After the last code change (whole root verified by `test_impl.py`), `rs/rust` and `rs/llvm`:

| check | rs/rust | rs/llvm |
|---|---|---|
| scope verify A-D (`--verify-only --scope`, 300 s each) | B, C, D VERIFIED after the last change | - |
| `opencode/test_impl.py` (verifier + fixtures) | PASS, 199/199 | PASS, 199/199 |
| `q.py` 300 random + 199 fixed | exact 406, P1 6, P2 5, P3 1, P4 6, P5 2, P8 5, P9 29, P2+P9 37, P3+P4+P8 1, P2+P4+P8 1, unexplained 0 | same |
| `q_mutants.py --check` | 16 / 16 present exactly once | - |
| hand `opencode.rs` | builds with `cargo build --release` (serde_json, rusqlite 0.40 bundled), 0 warnings | - |

No case needed a rerun. The frozen py pairs were not built (PF); `q.py` ran on the two rs pairs only. No perf timing
was run.
