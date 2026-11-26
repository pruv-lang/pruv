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
| path normalize | `2n + 4` scratch + push/pop helpers, second result buffer | (start, end) pairs of the pieces in one `[i64]` (push / `truncate` pop), one reserved result (4.4b) | `Vec<&str>` + `join("/")` | 1 + 1 vs 1 + 1; row 4.4b-NV |
| path compare | byte loops in `contained`, `same_key` | `eq_range` on `[u8]` | slice `==`, `starts_with` | 0 vs 0 |
| `key_eq`, `has_prefix`, `lit_eq` | byte loops on `str` | same | `==`, `starts_with` | row 4.3f-S |
| folder key / dirname / spans | `alloc` + `copy_span` | `copy(subview(..))` | `to_owned()` of a slice | 1 vs 1 |
| dirname of a `str` | `to_bytes` / `to_str` round trip | removed (no caller; 4.4b), `dirname_bytes` stays for `p100` | `rsplit_once('/')` | row 4.3g-K3 (`p100`) |
| pool append (`cat_bytes`, `append_str`, `pool_*`) | zero-filled `alloc(a+b)` + two loops | one pool grown `inout` (`arr_reserve` + `arr_append`); helpers removed (4.3g rows step) | `extend_from_slice` on one `Vec` | 0 vs 0 |
| column check (`has_name`) | full scan with `found` | return at first match | `iter().any` | 0 vs 0 |
| rows decode | zero-filled `rows` / `pool`, own `grow`, `fr` / `fp` copies | `arr_push` / `arr_append` into `rows` / `pool`, refs already in text-bank offsets, `p208` moves `d.rows` and pushes the absent counts (4.4b) | `Vec<Row>` push | 0 extra vs 0 |
| select SQL / pool | `pool_str` chain, 4 buffers in mode 0 | one SQL buffer, `match mode`; mode 0 one pool from a borrowed `folder_key` (rows step) | one `String`, `format!` | 1 vs 1 |
| export temp prefix / spawn args | second normalizer, zero-filled `out`, `to_bytes` of literals | `path::normalize(root ++ "/opencode-")`; `spawn_pool` reserve + `arr_append`, literals as `str` (4.4b) | `join(root, "opencode-")`, `&str` args | 1 vs 1 |
| message attach state | `mt: [i64]` 4 slots | `Attach { open, pid, mid }` inout | `Option<(String, String)>` + `bool` | tape ids vs owned strings (row 4.3g-T) |
| message text | `out` + `total` + `first` | `out` + `first`, `continue` exits | `String` + `first` | 0 extra vs 0 |
| turns store | `st` / `sl` / `buf` flat arrays, `feed` returns 1 to grow | same | `Vec<Turn>` | row VL |
| plan candidate | `skip` flag | `continue` | `continue` | same |
| plan fit | 90-word + 24,000-byte zero buffers, prefix copy out | `arr_push` / `arr_append`, moved into `PlanValue` | `Vec<String>` push | 0 extra vs 0 |
| plan `at` | `copy(at)` | same | `Option<String>` clone | row 4.3d-M |
| plan time | `copy(c)` / `copy(d)` | move | owned return | 0 vs 0 |
| convert loop | `bad` flag | `return shape_error()` | `?` | same |
| model text | `put_str` byte loops | reserve + `arr_append` + `arr_push(47)` | `format!("{p}/{m}")` | 1 vs 1 |
| result packet | zero-filled `40+N+K+T`, `put_*` loops, `result_shape` twice | reserve + `arr_append` of each part (rows step); `result_shape` still twice | `Vec::with_capacity` + `extend` | 1 vs 1; row 4.3g-A2 |

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

After the last code change of step 4.4b (§6), `rs/rust` and `rs/llvm`:

| check | rs/rust | rs/llvm |
|---|---|---|
| scope verify A-D (`--verify-only --scope`, 300 s each; scope C now also loads `path`) | A, B, C, D VERIFIED | A, B, C, D VERIFIED |
| `opencode/test_impl.py` (verifier + fixtures) | PASS, 199/199 | PASS, 199/199 |
| `q.py` 300 random (seed 20261117) + 199 fixed | exact 406, P1 6, P2 5, P3 1, P4 6, P5 2, P8 5, P9 29, P2+P9 37, P3+P4+P8 1, P2+P4+P8 1, unexplained 0 | same |
| `q_mutants.py --check` | 16 / 16 present exactly once (no marked text changed) | - |
| hand `opencode.rs` | `cargo build --release`, 0 warnings | - |

No case needed a rerun. No perf timing was run.

## 5. Rows step (4.3g rows closed after 4.4)

Every changed Pruv function was brought to the structure of its hand function; the hand changed only for 4.3g-E.

| row | status | what changed |
|---|---|---|
| 4.3g-P | closed | `packet::cat_bytes` / `append_str` / `put_bytes` and `util::pool_bytes` / `pool_str` / `pool_dec` removed; each pool is one buffer grown `inout` with `arr_reserve` + `arr_append` (`list::list_result` appends ids / titles / ISO straight from the text, `plan::plan_result`, `export::start` / `on_mkdtemp`) |
| 4.3g-Q | closed | `sql::select_sql` builds one buffer from the SQL literals with `match mode`; `select_pool` mode 0 borrows `folder_key` and appends it twice into one pool, mode 1 returns the thread without a copy |
| 4.3g-D | closed | `path::db_none` removed; `db_path` returns `enum DbPath { Memory, File(path: str) }` and reads `OPENCODE_DB` once; `admit::p100` matches on it |
| 4.3g-A | closed except A2 | `assemble_result`, `build_error`, `assemble_step`: reserve the total, append the header and each part (no zero fill, no fill loops) |
| 4.3g-E | closed | hand: `Fail.name`, `engine_error` (spawn / wait failure carries the native errno, raw code and `EngineImportError`), `encode_error` writes raw / name / detail with words 25 / 26 like Pruv `build_error` |
| 4.3g-K | partly closed | new `util::span_view -> ref [u8] from(t)`; `read_handoff`, `path::utility` mode 4, `select_pool`, `admit::p101..p103`, `sql::p200` borrow instead of copying; `folder_key` returns a view |
| 4.3g-F | partly closed | `util::format_i64` / `pool_dec` removed (no callers); `plan::items_at` and `list::rows_result` reserve + `arr_push` instead of zero fill |

Left as rows:

| row | what stays | why |
|---|---|---|
| 4.3g-A2 | `result_shape` runs twice (`ok_result`, then `step_args_ok`) | wire check on both sides of the packet (4.3g-SQ) |
| 4.3g-K2 | `rows::clean_title` / `updated_number` still copy through `span_bytes` | they need a `str` borrowed from bytes (`owned_text` takes an owned array); with 4.3g-R |
| 4.3g-F2 | `rows::format_ext` repeats the civil-date arithmetic | with 4.3g-R |
| 4.3g-R | `rows::decode` zero fill, own doubling, `fr` / `fp`, `relocate` / `p208` copies | not started (time cap) |
| 4.3g-N | `normalize` scratch `2n + 4` and a second buffer | needs a segment list (`Vec<&str>`) of views; not started |
| 4.3g-X | `export::join_prefix` second normalizer, `spawn_pool` zero fill | not started; depends on 4.3g-N |
| 4.3g-T | `Attach` tape ids; `dirname` / `iso_str` `str` / `[u8]` round trips | `admit::p100` appends the parent into the pool it was taken from, so a borrowed `dirname` aliases the `inout` pool; the hand borrows from an owned `String` |
| 4.3g-BS (language, new) | `build_error` / `error` / `done_error` take `name: [u8]`, `detail: [u8]` | `arr_append` of a `str` into `[i64]` is a type error; literals pass as static views, so callers did not change |

Codex structure comparison (gpt-6.2-astra, high; Codex 2 account, the default was at its limit). Applied:

- Hand bug: the raw code of an fs / exec failure was `ErrorKind`'s text; it is now the errno symbol the host reports
  (`errno_code`, e.g. `ENOENT`). A spawn that fails with `ENOENT` / `EACCES` is reported at stage 16 (Node raises it
  on the child's error event, as the host and fixture F08 do), other spawn failures at 15.
- `export::code_bytes -> ref [u8] from(completion)`; `rm_step` / `path_step` pass the path as a `subview`, `rm_step`
  builds its words in one reserved buffer; `on_mkdtemp` checks the bank first and writes `dir ++ "/export.json"` once
  (the directory ref is its prefix); `sql::select_pool` mode 1 checks a view, then copies once.

Not applied (rows):

| row | finding | why |
|---|---|---|
| 4.3g-A3 | `build_error` / `assemble_step` start from a 40 / 12-word buffer and grow it; the hand reserves the total first | header words are set by index after `meta_new`; with A2 |
| 4.3g-K3 | `admit::p100` owns the thread id for `valid_thread` and copies the parent; `db_path` owns `home` before the absolute check; `utility` mode 3 `to_bytes(d)` | p100 with 4.3g-T; the others need `str` byte views (4.3f-S family) |
| 4.3g-M07 | `list_result` reads rows through `sql_order` (identity) | the array is the M07 mutant hook (`q_mutants.py`); changing it needs a new mutant form |
| (contract) | `plan::items_at` drops a trailing partial triple (before: zero words up to `len(items)`) | only `fit` outputs reach it and they are whole triples; the hand has no partial triples |

Observation (hand, not changed): `hand::dirname("//a")` returns `""` (`t[..1]` trimmed), Node `path.posix.dirname`
returns `"//"`; no fixture reaches it (the DB path comes from `join(home, ..)`).

## 6. Step 4.4b (rows R, K2, F2, N, X, part of T)

| row | status | what changed |
|---|---|---|
| 4.3g-R | closed | `rows::decode(query_words, query_bytes, base)` starts `rows` / `pool` empty and grows them with `arr_push` / `arr_append` (no zero fill, no own `grow`, no `fr` / `fp` copies); the refs it pushes are already text-bank offsets (`base + len(pool)`), so `relocate` is gone; `sql::p208` moves `d.rows`, reserves `n` and pushes `-1` for each absent message count (hand `Row.messages: None`) instead of copying into a `lit` array |
| 4.3g-K2 | closed | `rows::clean_title` / `updated_number` check the span with `utf8_valid` on a `ref [u8]` view and pass `subview(qb, a, a + b)` to the `str` parameter (R-6g-10, no copy); `span_bytes` / `owned_text` are gone from both |
| 4.3g-F2 | partly closed | `format_ext` reserves 27 bytes and pushes (no zero fill + index writes); the civil-date arithmetic is still repeated because `pkgs/time` has no public `civil` (row 4.4b-CV) |
| 4.3g-N | closed (language row) | `path::normalize` follows the hand: empty input gives `"."`, the pieces of `p` between `/` (empty and `.` dropped, `..` pops a non-`..` piece, is dropped when rooted, else kept) go into one list, then one result buffer joins them with `/`, adds `.` when empty and relative, `/` for a trailing slash. The list holds (start, end) pairs in an `[i64]` (pop is an in-place `truncate`) and the root `/` is pushed before the join instead of inserted at 0 (row 4.4b-NV). `push_seg` / `append_seg` / `pop_pos` / `last_dotdot` are gone. `join` returns `str_from_utf8` of the moved buffer (no `to_str` copy) after a `utf8_valid` check the byte buffer forces (row 4.4b-NV) |
| 4.3g-X | closed | `export::join_prefix` (second normalizer, two zero-filled `[i64]` arrays, `out` / `res` copies, internal error 28 over 2 MB) is gone: `start_export` builds `root ++ "/opencode-"` once (hand `format!` inside `join`) and calls `path::normalize`. `spawn_pool` reserves the summed total and appends (no zero fill, `--pure` / `export` appended as `str` literals, no `to_bytes`). `on_mkdtemp` keeps one pool; its two patches are the step wire (4.3g-SQ) |
| 4.3g-T | partly closed | `path::dirname(str)` and `to_str` had no caller and are removed; `entry::iso_str` moves the checked text out (no final `copy`) |

Left as rows:

| row | what stays | why |
|---|---|---|
| 4.4b-NV (language, new) | `normalize` keeps (start, end) offsets instead of `Vec<&str>`; root `/` pushed first; `join` re-checks UTF-8 on the normalized bytes | `Vec` / `Option` of `ref` types is a type error (borrow signature); a byte buffer built from `str` pieces does not carry the UTF-8 fact |
| 4.4b-CV (pkgs, new) | `rows::format_ext` repeats the civil-date arithmetic | `pkgs/time` exposes no `civil`; adding it touches a shared package (run all consumers) |
| 4.3g-T2 | `entry::Attach` holds the tape ids of providerID / modelID, the hand clones two `String`s per assistant message | needs an `Option` pair of owned `str` in `Attach` and a `convert::model_bytes` rewrite; not done in the cap. `iso_bytes` keeps `copy(iso.text)`: `Iso` is a `pkgs/time` struct and `move iso.text` is `type/foreign_move` (4.3d-M) |
| 4.3g-A2, 4.3g-A3, 4.3g-K3 | as in §5 | not started (time cap); `p100` still copies the DB path and the parent (`dirname_bytes`) |
| 4.3g-SQ, 4.3g-BS, 4.3g-M07 | as in §5 | unchanged |

Codex structure comparison (gpt-6.2-astra, high; Codex 2 account, the default was at its limit), read-only over the
changed functions. Path edge cases traced equal on both sides (`""`, `"/"`, `"//"`, `"a/.."`, `"../.."`, `"/.."`,
`"./"`, `"a/./b/"`, `"/a/../../b/"`). No code change came out of it; the findings went to rows:

| row | finding | why not applied |
|---|---|---|
| 4.4b-NV | `clean_title` writes the same `subview` four times; `join` copies a lone non-empty side into `jb` before `normalize` (hand normalizes it directly) | a `ref [u8]` local cannot be passed as `str`, and a `str` is not a `[u8]` argument; language |
| 4.4b-NC | `normalize` reserves `n + 2`; the hand's `join("/")` sizes the result from the surviving pieces | needs a sizing loop over the pairs and new length invariants; next OpenCode step |
| 4.4b-P208 | `p208` pushes the `-1` counts after the move; the hand sets `messages: None` per row | the counts are a separate bank section after the `7 n` row words (step wire, 4.3g-SQ) |
| 4.4b-SP (check) | Codex reads `spawn_pool`'s cwd as `cfg[6..7]` and the last argv as the requested thread, where the hand uses the selected row's directory / id (differs for `COLLATE NOCASE` ids) | not in this step's change; F07 (`argv-cwd`, fake exporter checks cwd and argv) and `q.py` pass against the TypeScript reference; to be checked against the reference before changing either side |
| 4.3g-K3 / pre-existing | `start_export` re-checks binary / tempRoot already checked by `read_handoff`; `updated_number` REAL checks twice and returns 0 for an out-of-range INTEGER on a direct call (unreachable: the host answers `binding-range` first); `iso_bytes` 1-byte flag array | unchanged code; with K3 / 4.3d-M |

Tests: §4. `test_impl.py` scope C now lists `path` (export calls `path::normalize`).
