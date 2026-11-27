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

After the last code change of step 4.4f (§7), `rs/rust` and `rs/llvm`:

| check | rs/rust | rs/llvm |
|---|---|---|
| scope verify A-D (`--verify-only --scope`, 300 s each; scope C also loads `path`) | A, B, C, D VERIFIED | verified by the `test_impl.py` build |
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

## 7. Step 4.4f (rows 4.4b-SP, 4.4b-NC, 4.3g-A3; 4.4b-NV checked against L-gaps 2)

| row | state | change |
|---|---|---|
| 4.4b-SP | closed (hand fixed) | the TypeScript `runExport(binary, tempRoot, thread, source.home, ..)` spawns `--pure export <thread>` with `cwd: home` (the comment says the export deliberately runs in the data folder). Pruv `spawn_pool` already did this (cwd `cfg[6..7]`, thread `cfg[10..11]`). The hand used the selected row's directory as cwd and the row id as the thread; it now uses `conf.home` / `conf.thread`, and `Handoff` keeps only the fallback title |
| 4.4b-NC | closed | `normalize` sums the kept pieces (`width + 1` each) in a loop over the pairs before allocating and reserves `need + 1` (leading `/`, trailing `/` or the lone `.`), the bound the hand's `join("/")` + push / insert reaches; was `n + 2` |
| 4.3g-A3 | closed | `packet::meta_cap(mode, extra)` reserves `40 + extra` on an empty array and resizes to 40 (one allocation); `build_error` uses it for the header plus the three byte strings. `assemble_step` reserves `total` on an empty array and resizes to the 12 header words (was `alloc(12)` then a second reserve) |
| 4.4b-NV | open (language) | L-gaps 2 added a function-local `Vec<ref str>` (R-6g9-10) and the ASCII-adjacent boundary (R-6b-5), but the rewrite still cannot drop the `utf8_valid` scan without a workaround: (a) a local `[ref str]` is `codegen/unsupported_type` on LLVM, and the fragment must build on both backends (row 4.4f-NVL); (b) a `String` builder becomes `str` only through `string_to_str`, which copies, where Pruv `str` is already a Rust `String` (row 4.4f-SS). Either form would add a copy of the result, which costs more than the current scan. `normalize` / `join` are unchanged |
| 4.4f-CW | new (bench, step wire) | `list::read_handoff` still writes the first row's directory and id to `ctl[34..37]`; nothing reads them after 4.4b-SP (only the title, `ctl[32..33]`). They are part of the DESIGN ctl layout (words 34 / 36), so removing them is a wire change; kept with 4.3g-SQ |

Not changed (time cap or reason unchanged): 4.3g-T2 (the hand clones two `String`s per assistant message, Pruv keeps
tape ids and copies once at the end, fewer allocations; matching would add copies), 4.3g-A2 (`result_shape` twice,
step wire), 4.3g-K3 (`valid_thread` takes `str` and the thread is a `cfg_text` byte span: needs a `ref str` view of a
byte span, 4.3f-S; `p100`'s `dirname_bytes` result has no view form for the `.` / `/` cases), 4.4b-P208, 4.3g-SQ,
4.3g-M07, 4.3g-BS, 4.4b-CV.

Tests: §4 (rerun after this step, same counts). Hand build: separate crate outside the repo, 0 warnings.

## 8. Step 4.4j (rows 4.4b-CV, 4.3g-T2)

Structure table of the changed functions (generated `rs/rust` code vs `hand/opencode.rs`):

| function | Pruv now (generated Rust) | hand Rust | types / ownership | loops | allocations / copies (Pruv vs hand) |
|---|---|---|---|---|---|
| `time::civil` (new, `pub`) | days shifted by 5 400-year cycles, truncating `/`, year shifted back by 2000; returns `Civil { year, month, day, rem }` | `div_euclid` / `rem_euclid`, returns `(i64, i64, i64, i64)` | named struct instead of a tuple: tuple types exist only in a single-file program (`parse/tuple_module`, row P6-2) | 0 vs 0 | 0 vs 0 |
| `time::format_iso` | `civil(ms)` + 24 digit writes into `alloc(24)`; empty result when the guarded ranges fail | `civil(ms)` + `format!` | `[u8]` vs `String` (unchanged) | 0 vs 0 | 1 vs 1 |
| `rows::format_ext` | `civil(ms)` + reserve 27 + pushes (was a second copy of the calendar walk) | `civil(ms)` + `format!("+{year:06}..")` | `[u8]` vs `String` (unchanged) | 0 vs 0 | 1 vs 1 |
| `entry::Attach` / `entry::Model` | `Attach { open, model: Model }`, `enum Model { Absent, Pair(String, String) }` | `Collector { open, model: Option<(String, String)> }` | owned pair; a dedicated enum because an `Option` payload cannot hold `str` (`type/type_argument`) and tuples are P6-2 | - | - |
| `entry::feed_assistant` (model part) | both fields nonempty strings ⇒ `mt.model = Pair(str_at(pm), str_at(mi))`, old pair dropped | both `as_str()` nonempty ⇒ `c.model = Some((p.to_owned(), m.to_owned()))` | owned per message | 0 vs 0 | 2 vs 2 per eligible assistant message (`str_at` moves the decoded string out, no second copy) |
| `convert::model_bytes` | `match model`: `Absent` ⇒ empty; `Pair(p, m)` ⇒ reserve `p + 1 + m`, append, push `/`, append | `map_or_else(String::new, format!("{p}/{m}"))` | borrows the pair | 0 vs 0 | 1 vs 1 |
| `convert` message loop | `Attach { open: false, model: Absent }`, `model_bytes(mt.model)` after the loop | `Collector { open: false, model: None }`, `model(&c)` | same | same | 0 vs 0 |

| row | state | change |
|---|---|---|
| 4.4b-CV | closed | `pkgs/time` exposes `pub struct Civil` and `pub fn civil(ms)` (requires `-62167219200000 <= ms <= 8640000000000000`, ensures `0 <= rem < 86400000`). `format_iso` and `rows::format_ext` both call it; the calendar arithmetic exists once, as in the hand. `format_iso`'s contract is unchanged; both callers keep their range guards (the calendar arithmetic itself is tested, not proved) |
| 4.3g-F2 | closed | with 4.4b-CV `format_ext` is `civil` + one reserved buffer, the hand's shape |
| 4.3g-T2 | closed | `Attach` holds the owned pair like the hand (`Model::Pair(str, str)`), set per eligible assistant message; `model_bytes` joins it at the end. Tape ids are gone. The pair lengths are carried as `is_variant` / `variant_field` facts through `feed`, `feed_user`, `feed_assistant` and the convert loop invariant |

New / remaining rows:

| row | what stays | why |
|---|---|---|
| 4.4j-TU (language, new) | `Civil` is a named struct, `Model` a dedicated enum, where the hand uses a 4-tuple and `Option<(String, String)>` | tuples are single-file only (`parse/tuple_module`, P6-2); an `Option` payload cannot hold `str` (`type/type_argument`). Same layout and allocations; the names differ |
| 4.4j-ME (bench) | `model_bytes` returns empty when a decoded side is empty; the hand has no such check | `str_at` gives `""` for a corrupt tape (`InternalInvariant`); kept so that the corrupt-tape result stays the old one. Unreachable on a valid tape (both sides passed `nonempty_string`) |
| 4.3g-T (iso part) | `entry::iso_bytes` keeps `copy(iso.text)` | `move iso.text` of the `pkgs/time` struct is `type/foreign_move` (4.3d-M); unchanged |

Tests (`rs/rust` and `rs/llvm`, `PRUVC_RS` release binary; all consumers of `pkgs/time`):

| slice | test_impl | q |
|---|---|---|
| opencode | 199/199 on both; `q_mutants.py --check` 16/16 markers | `--cases 300`: exact 406 on both, unexplained 0 (classes as 4.4f) |
| codex | ts/oracle 136/136; 136/136 on both | `q.py 300`: exact 262 + policy P1/P2/P3/P4/P6/P8 on both, no differing case (exit 1 only from the absent frozen py runners) |
| grok (`GROK_TS_TRANSPILER`) | 254/254 on both | `--cases 300` (554 cases): exact 450 + policy classes, unexplained 0 |
| qwen | verifier PASS; 26/26 on both | not run: `qwen/q.py` hardwires the frozen py pairs |

Other-model comparison (Codex gpt-6.2-astra, read-only, generated `rs/rust` vs `hand/opencode.rs`, no conclusions
given). Equal: `civil` for every ms in the contract range (the 5-cycle shift cancels against `- 2000`, no `i64`
overflow in range), the 24 / 27 output bytes on the plain / extended ranges, `db_iso`, and the `feed_assistant`
order (summary skipped first, model replaced before the `open` check). Differences it reported and how they stand:
- `Civil` struct vs 4-tuple, month / year set by a conditional overwrite vs `if` expressions: 4.4j-TU; no loop or
  heap allocation on either side.
- `format_iso` / `format_ext` keep their range guard returning empty and zero-fill `alloc(24)` / reserve 27 instead
  of `format!`: unchanged (contracts allow an empty result; the bytes are tested, see 4.4b-CV).
- `feed_assistant` filters on the raw token span (`> 2` with quotes) before `str_at`; the hand decodes and checks
  `is_empty()`. Same set on a valid tape (a raw span longer than `""` decodes to at least one character).
- `model_bytes(Pair("", "m"))` is empty where the hand would give `/m`: 4.4j-ME, unreachable through `feed`.
- Convert loop: message id vector + index loop, the flat turns store zero-filled up front and the grow-and-retry
  return `1`: row VL and the existing loop shape, outside this step. No `Attach` / model copy per message.
No new row from the comparison.

## 9. Step 4.4k (row 4.4b-NV, attempted; blocked by a proof gap)

The hand-shaped rewrite was written and run through the verifier (`pruvc-rs` main release binary), but it does not
verify, and the only ways to make it verify are a run-time length guard or a narrower `requires`, neither of which
the hand has. Following the reset rule (no approximations), `path.pruv` is unchanged and the gap is a row. The draft,
its verifier output and its `--no-verify` `rs/rust` output are kept in `work/reset4_4k/` (not committed).

Per-function structure (committed Pruv, the draft, hand `hand/opencode.rs`):

| function | committed Pruv (generated Rust) | draft `normalize_text` (generated Rust, `--no-verify`) | hand Rust | difference that stays |
|---|---|---|---|---|
| types | `normalize(p: [u8]) -> [u8]` | `normalize_text(p: &str) -> String` | `normalize(p: &str) -> String` | draft equal; committed is bytes |
| piece list | `[i64]` of (start, end) pairs, `truncate(out, m - 2)` pops | `Vec<&str>`, `out.push(&p[st..i])`, `out.pop()` | `Vec<&str>`, `push(seg)`, `pop()` | draft equal |
| split loop | one byte loop over `0..=n`, cut at `/` | same | `p.split('/').filter(..)` | iterator vs index loop (same pieces, same order) |
| `..` rule | last pair is not `..` ⇒ pop; else rooted ⇒ skip; else push | `vec_get(out, m - 1)` not `..` ⇒ pop; else rooted ⇒ skip; else push | `match out.last()` with the same three arms | draft equal |
| join of pieces | sizing loop over the pairs, one `reserve(size)`, `extend_from_slice` per piece | `String::new()` + `push('/')` / `push_str(piece)`, no reserve (grows) | `out.join("/")` (std sizes the total, one allocation) | draft: no `String` reserve (4.4k-SR); both: an explicit loop for the std `join` |
| root `/` | pushed first, `base = 1` | pushed first, `base = 1` | `s.insert(0, '/')` at the end | no insert at 0 for a `String` (4.4k-SR); pushing first moves no bytes |
| `.` / trailing `/` | `len(r) == base && !rooted` ⇒ `.`; `dir && len > base && last != '/'` ⇒ `/` | same over `s` | same over `s` | equal |
| result | the `[u8]` buffer | `string_into_str(move s)` (no copy, R-6e-23) | `s` | draft equal |
| verification | verified | fails: 3 `verify/call_requires` on `string_push_char` / `string_push_str` (`len(s) + w <= 2^40`) | - | 4.4k-VS |

| function | committed Pruv | hand | difference |
|---|---|---|---|
| `join(a, b)` | both empty ⇒ `"."`; else one `[u8]` buffer `a ++ "/" ++ b` (reserve `na + nb + 1`), `normalize`, `utf8_valid` scan, `str_from_utf8` of the moved buffer | `(true, true)` ⇒ `"."`; one side empty ⇒ `normalize(other)` directly; else `normalize(&format!("{a}/{b}"))` | a lone side is copied into the buffer; the UTF-8 scan. Both need `normalize_text` (`str` in, `str` out), blocked by 4.4k-VS |
| `export::start_export` (caller) | `normalize(root ++ "/opencode-")` on config bytes | `join(root, "opencode-")` | the byte root stays (L3-VB allows a `Vec<ref [u8]>` port, but the output buffer has the same unprovable size; not tried) |

Why the draft cannot verify. The obligations are the length bounds of the builder pushes inside and after the join
loop (`string_push_char` requires `len(s) + 4 <= 2^40`, R-6g8-2; `string_push_str` the same kind of bound). The true
bound is `len(s) <= 1 + sum of the piece lengths + pieces <= len(p) + 1`, which needs the sum of the list's element
lengths. No contract can state it: `vec_get` of a view list in an invariant is `type/foreign_contract`
(`work/reset4_4k/contract_vec_get.log`), lists cannot be parameters (R-6g9-10, so no helper function over the list),
and the per-piece bound alone (`len(piece) <= n`, `len(out) <= n`) gives `n * n`, above `2^40` for the callers'
`1e9`-byte arguments. The committed offset form avoids this because the pairs are positions in `p`. The ways out
that the reset rule forbids: a per-push `if len(s) + len(piece) + 1 > LIMIT` guard (the hand has none), or
`requires len(p) <= 2^20` (changes `join` / `data_dir` / `db_path` callers, which accept `1e9`-byte values).

Rows:

| row | state | note |
|---|---|---|
| 4.4b-NV | open (blocked by 4.4k-VS) | the shape is available (draft above); `normalize`, `join` and the `export` caller unchanged |
| 4.4k-VS (language / proof, new) | open | a list of views has no length sum in proofs: `vec_get` cannot appear in a contract and a list cannot be a parameter. Needs either `len(vec_get::<ref T>(l, k))` and a sum in contracts, or a catalog join over a view list (hand `out.join(sep)`) whose `ensures` gives the total |
| 4.4k-SR (language, new) | open | no `String` reserve / with-capacity (hand `join` allocates once) and no insert at 0 (hand `s.insert(0, '/')`); the draft grows the `String` and pushes the root first |

Tests: no `.pruv` file of `opencode/` changed, so `test_impl.py`, the scope verifies, `q.py` and `q_mutants.py` were
not rerun; the 4.4j results (§8) stand for the unchanged code. Only the comment above `normalize` in `path.pruv`
changed (it now names 4.4k-VS and 4.4k-SR); `q_mutants.py --check` after it: all 16 markers present exactly once.
No measurement.

Other-model comparison (Codex `gpt-6.2-astra`, review role, given the generated Rust of the draft and the hand Rust
without this section): the draft and the hand agree on the nine edge inputs it traced (`""`, `"/"`, `"//"`, `"a/.."`,
`"../.."`, `"/.."`, `"./"`, `"a/./b/"`, `"/a/../../b/"`; traced by hand, not run); it found no SPEC feature that proves the three push bounds
with the view-list representation, so the gap stands. Its other findings:
- The draft's empty-input branch copies twice (`String::from(".")` then `to_owned()`); fix when 4.4b-NV reopens.
- A hybrid that keeps the `(start, end)` offsets but builds a `String` with `string_push_str(subview(p, lo, hi))` and
  ends with `string_into_str(move s)` could prove the bound and drop `join`'s `utf8_valid` scan. Not done: the piece
  list would still differ from the hand's `Vec<&str>`, so it does not close 4.4b-NV; recorded as an option on 4.4k-VS.
- No `String` reserve / with-capacity / insert builtin exists (agrees with 4.4k-SR).
- The comment above `normalize` was stale (said Pruv has no view lists); fixed in this step.

## 10. Step 4.4o (rows 4.4b-NV, part of 4.3g-K3)

`normalize` is now the hand shape on `str` (a `Vec<ref str>` of pieces of `p`, `pop` on `..`, `vec_join(out, "/")`,
then `.`, the trailing `/` and `string_prepend_char`). The 4.4k-VS gap is closed by the view sum (R-6g9-13) in the
split-loop invariant and the catalog join (R-6g9-16), whose `ensures` gives the total; the old `ensures`
(`1 <= len(result) <= len(p) + 1`) and `requires` (`len(p) <= 2000000002`) are proved unchanged, with no run-time
guard. 4.4k-SR is closed by `string_with_capacity` and `string_prepend_char` (R-6g9-14). The configuration reads that
the hand borrows (`conf.home`, `conf.os_home`, `conf.thread`, `env_value`) now return `ref str from(cfg_text)`
through the new `util::cfg_view`. No compiler change. Evidence (not committed): `work/reset4_4o/gen_changed.rs`
(`rs/rust` output of the changed functions), `work/reset4_4o/hand_paths.rs`.

Per-function structure (generated Rust vs hand `hand/opencode.rs`):

| function | before | Pruv now (generated Rust) | hand Rust | difference that stays |
|---|---|---|---|---|
| `normalize` | `[u8] -> [u8]`, `(start, end)` offset pairs, sizing loop + `extend_from_slice`, root pushed first | `&str -> String`; one index loop over `0..=n` cutting at `/`; `Vec<&str>` `push(&p[st..i])` / `pop()`; `out.join("/")`; `push('.')`, `push('/')`, `insert(0, '/')` | `&str -> String`; `split('/').filter(..)`; `Vec<&str>` `push` / `pop`; `out.join("/")`; same three fix-ups | index loop vs iterator (same pieces, same order); last piece read by index (`out[m - 1]`) vs `out.last()`; `vec_push` keeps its `2^40` length check (4.4o-VP) |
| `normalize_bytes` (new name) | was `normalize` | unchanged byte normalizer | - (hand `join(root, "opencode-")`) | kept for `export::start_export` (4.4o-EB) |
| `join` | one `[u8]` buffer for every case, `normalize`, `utf8_valid` scan, `str_from_utf8` | both empty ⇒ `"."`; `b` empty ⇒ `normalize(a)`; `a` empty ⇒ `normalize(b)`; else `String::with_capacity(na + nb + 1)` + `a` + `/` + `b`, `normalize` | same arms; `format!("{a}/{b}")` | `with_capacity` + pushes vs `format!` (one allocation each); the UTF-8 scan is gone |
| `env_value` | owned copy through `cfg_str` (two copies), `""` copied when absent | `&'a [u8] -> &'a str` borrowed span, `""` literal | `&'a str` into `conf.env` | per-read UTF-8 check of the span (4.4o-CV) |
| `data_dir` | owned `xv`, owned `home` | borrowed `xv`, `home`; one allocation (the join) | borrowed | equal apart from 4.4o-CV |
| `db_path` | owned `gv`, owned `home` | borrowed `gv`, `home`; one allocation (the join or `copy(given)`) | borrowed; `g.to_owned()` | equal apart from 4.4o-CV |
| `admit::p100` (mode 1) | owned thread id (`cfg_str`) | borrowed `cfg_view` passed to `valid_thread(&str)` | `valid_thread(conf.thread...)` borrowed | equal apart from 4.4o-CV; the DB path `to_bytes(p)` + `dirname_bytes` pool copy stays (4.3g-T, 4.4o-SB) |
| `util::cfg_view` (new) | - | bounds / `ref_ok` / length guard, `utf8_valid` on the subview, returns the subview as `&str` (`from_utf8_unchecked`) | `Config::decode` decodes once, fields are `Option<String>` | the per-read scan (4.4o-CV) |
| `util::cfg_str` | `span_bytes` copy, `owned_text` move, then `copy` (two copies) | `copy(cfg_view(..))` (one copy) | - | one copy, as the owned callers need |

Behaviour change: the old byte normalizer gave `"."` for `"./"` (its empty check ran before the `.` push); the new
one follows the hand and Node (`"./"`). `join` reaches it when the normalized pieces are empty and the text ends with
`/` (for example `join("./", "")`, `join(".", "/")`); no test case hits it. `normalize_bytes` keeps the old rule, but its only input ends with
`/opencode-`, so it never applies there.

Kept, with the exact obligation:
- `path::utility` mode 3 `to_bytes(d)` and `p100`'s `to_bytes(p)` before `dirname_bytes`: passing the `str` to a
  `[u8]` parameter is `type/type_mismatch` ("expected [u8], found str"), and there is no `str` → `[u8]` move (row
  4.4o-SB); `p100`'s pool copy is also the step-wire aliasing of 4.3g-T.
- `export::start_export` keeps `normalize_bytes`: its root is a byte span of `cfg_text` that is not checked as UTF-8,
  and the request takes a `[u8]` pool (row 4.4o-EB).

Other-model comparison (Codex `gpt-6.2-astra`, review role, Codex 2 account, given `gen_changed.rs` and
`hand_paths.rs` without this section): `normalize` and `join` agree with the hand on all 17 traced inputs (`""`,
`"/"`, `"//"`, `"."`, `"./"`, `"a/.."`, `"../.."`, `"/.."`, `"a/./b/"`, `"/a/../../b/"`, `"a//b"`, `"../x/./y/.."`,
`"/a/b/../"` and four joins; traced by hand, not run) and have the same structure apart from the index loop and the
`vec_push` check (4.4o-VP). It marks `data_dir`, `db_path`, `env_value`, `cfg_view`, `cfg_str` and `p100` as not
the same structure. Its findings, checked against the code:
- The per-read UTF-8 scan in `cfg_view` (all the borrowed readers): row 4.4o-CV. It also notes that `env_value`
  returns `""` for a matching key whose value is not UTF-8, where the hand's lossy decode returns the replaced text;
  this was already the case with `cfg_str` before this step, and is part of 4.4o-CV.
- `cfg_str` copies once; the hand has no owned read. Its remaining callers need an owned `str`; not a gap of this step.
- `p100` copies the parent (`dirname_bytes`) into the pool and duplicates `ctl`; the hand borrows `dirname(&db)`.
  Already rows 4.3g-T / 4.4o-SB.
- `p100` rejects a file path with `!bank_fits(..) || np > 1073741823` where the hand rejects `db.len() > 1e9`. This
  code is not changed by this step (the pool bound of the step wire); recorded as row 4.4o-PL.
- `p100` has more early returns (`ctl[6] != 0`, mode range) than the hand's `admit`; these are the packet protocol
  checks, outside the hand function.

Tests (`bench/l3/opencode`, `pruvc-rs` main release binary, rs/rust and rs/llvm):

| test | rs/rust | rs/llvm | before (4.4m baseline) |
|---|---|---|---|
| `test_impl.py --verify-only` | VERIFIED | VERIFIED | VERIFIED |
| `test_impl.py` fixtures | 199/199 | 199/199 | 199/199 |
| `q.py --cases 300` (499 cases) | exact 406, 0 unexplained | exact 406, 0 unexplained | exact 406, 0 unexplained |
| `q_mutants.py --check` | all 16 markers present exactly once | | 16/16 |

Rows:

| row | state | note |
|---|---|---|
| 4.4b-NV | closed | `normalize` / `join` in the hand shape, old contracts proved |
| 4.4k-VS | closed | by R-6g9-13 + R-6g9-16 |
| 4.4k-SR | closed | by `string_with_capacity` + `string_prepend_char` |
| 4.3g-K3 | partly closed | `db_path` / `data_dir` home, `env_value`, the `p100` thread are borrowed; left: 4.4o-SB |
| 4.4o-SB (language, new) | open | no `str` → `[u8]` move or byte view of a `str` argument: `utility` mode 3 and `p100` copy (`to_bytes`) to call `dirname_bytes` |
| 4.4o-EB (new) | open | `export` root is unvalidated config bytes and `await_req` takes `[u8]`; needs a `cfg_view` root and a `str` request field |
| 4.4o-CV (new) | open | `cfg_view` scans the span's UTF-8 on every read; the hand decodes `Config` once. Needs a validated-once config text (a `ref str` of the whole text) |
| 4.4o-PL (new) | open | `p100` file-path bound is the pool bound (`bank_fits`, `1073741823`) where the hand checks `len > 1e9`; values differ for a path of 1e9+1 ..= 1073741823 bytes if the bank fits (pre-existing) |
| 4.4o-VP (compiler, new) | open | verified `vec_push` of a view still emits the `2^40` length check (`pruv_g6_req`) that the hand does not have; the invariant bounds `len(out) <= n` |

## 11. Step 4.4q (rows 4.4o-CV, 4.4o-PL, part of 4.4o-SB and 4.3g-T)

The configuration text is now checked as UTF-8 once, in the step that reads it (`admit::p100` for modes 0..2 and
`path::utility` mode 3), like the hand's one `Config::decode` per run. `util::cfg_view`, `path::env_value`,
`path::data_dir` and `path::db_path` `require utf8_valid(cfg_text)`, so the verified calls carry no run-time check,
and `cfg_view` keeps only two O(1) char-boundary byte tests on the span ends (R-6g-10), with no UTF-8 scan. The old
scanning body is the private `util::span_text`, used by `cfg_str` (its caller reads the `text` bank, which is not
checked once). `path::dirname_bytes` is replaced by `path::dirname(p: str) -> ref str from(p)`; `p100` borrows the
parent of the owned DB path and builds the request pool with one `reserve` and two appends (no `to_bytes` copy).
`p100`'s file-path bound is now the hand's `len > 1e9` first, then the bank fit. `Attach { open: bool, model: Model }`
with `enum Model { Absent, Pair(p: str, m: str) }` already holds what the hand holds (`Option<(String, String)>` and a
bool); not changed. No compiler change. Evidence (not committed): `work/reset4_4q/gen_changed.rs`,
`work/reset4_4q/hand_paths.rs`.

Per-function structure (generated Rust vs hand `hand/opencode.rs`):

| function | before | Pruv now (generated Rust) | hand Rust | difference that stays |
|---|---|---|---|---|
| `util::cfg_view` | `ref_ok` / length guard, `from_utf8` scan of the span, `from_utf8_unchecked` | same guards, two byte tests on `t[s]` / `t[s+l]` (continuation byte ⇒ `""`), `from_utf8_unchecked`; no scan | `Config::decode` decodes every field once (lossy), reads borrow `conf.<field>` | the two boundary tests (the host gives offsets, not fields); a field cut inside a character reads `""` where the hand gives `U+FFFD` (pre-existing, host never cuts there) |
| `util::span_text` (new, private) | was `cfg_view` | the old scanning body | - | only for `cfg_str` |
| `util::cfg_str` | `copy(cfg_view(..))` | `copy(span_text(..))` | - | unchanged structure |
| `path::env_value` | borrowed, one span scan per match | borrowed, no scan; verified call to the `cfg_view` body | `iter().find()` on the decoded env | record scan on the wire vs decoded `Vec` (protocol) |
| `path::data_dir`, `path::db_path` | borrowed, scans in each read | borrowed, no scans | borrowed | equal apart from the join shape (§10) |
| `path::dirname` (replaces `dirname_bytes`) | `[u8] -> [u8]`, one allocation per result | `&str -> &str`, one backward index loop, `"."` / `"/"` / `"//"` literals, `get_unchecked(0..endp)` (the invariant has `p[endp] == '/'`, so the cut is a char boundary) | `&str -> &str`, `trim_end_matches('/')`, `rfind('/')`, `trim_end_matches('/')` | loop vs `trim` / `rfind`; values differ on repeated slashes (row 4.4q-DN) |
| `path::utility` mode 3 | per-read scans | one `utf8_valid(cfg_text)` (fail ⇒ `(3,6)`), borrowed reads, `to_bytes(d)` | `decode` then `data_dir(..).into_bytes()` | the `to_bytes` copy (4.4o-SB, language); whole-text check (row 4.4q-CU) |
| `admit::p100` | per-read scans; `to_bytes(p)` + `dirname_bytes(pool)` pool; bound `bank_fits || np > 1073741823` | one `utf8_valid(cfg_text)` (fail ⇒ `(0,3,6)`), borrowed thread / DB reads, `dirname(&p)` borrowed, `vec![]` + `reserve_exact(np + nq)` + two `extend_from_slice`; bound `np > 1e9 || !bank_fits` ⇒ `(1,3,5)` | `decode` once; `admit`: `db.len() > 1e9` ⇒ `(1,3,5)`, `canonicalize(dirname(&db))` | the request pool copy of path + parent is the step wire (the next step gets the bytes back, 4.3g-T); `ctl` copy and protocol checks (`ctl[6]`, mode range) are packet protocol |

Behaviour changes:
- A configuration text that is not valid UTF-8 as a whole is now `CoreError` / `InternalInvariant` stage 0 (`(3,6)`)
  in `p100` and `utility` mode 3, even when the invalid bytes are not inside a field (row 4.4q-CU). Before, each field
  read as `""`; the hand decodes lossily. The host builds `cfg_text` from Rust `String`s, so it cannot send such a
  text; no test case reaches it.
- `p100` with a DB path of 1e9+1 ..= 1073741823 bytes now gives `(1,3,5)` like the hand (closes 4.4o-PL).

Kept, with the exact obligation:
- `utility` mode 3 `to_bytes(d)`: `ok_value` takes `[u8]`; passing the `str` is `type/type_mismatch` ("expected [u8],
  found str") and there is no `str` → `[u8]` move (4.4o-SB, language).
- `export::start_export` keeps `normalize_bytes` on raw config bytes: the request field of `await_req` is a `[u8]`
  pool, so a `str` root would need the same `str` → `[u8]` move (4.4o-EB, blocked by 4.4o-SB).

Other-model comparison (Codex `gpt-6.2-astra`, review role, Codex 2 account; the default account was at its limit;
given `gen_changed.rs` and `hand_paths.rs` without this section): it marks every pair as not the same structure. Its
findings, checked against the code:
- `cfg_view` body: zero UTF-8 scans, two boundary tests; `env_value` / `data_dir` / `db_path`: zero re-checks after the
  one check in the step. Agrees with the table.
- `dirname`: no scan and no allocation on both sides; the `get_unchecked` cut is justified by the invariant. Values
  differ: `"//a"` (Pruv `"//"`, hand `""`) and `"a//b"` (Pruv `"a/"`, hand `"a"`); `"a//"`, `"/"` agree. Pruv follows
  `path.posix.dirname` of the pinned TypeScript reference (checked with Node), as `dirname_bytes` did; the hand does not. Row 4.4q-DN.
- `utility` mode 3 / `p100`: one whole-text check, and the unreferenced invalid byte case above. Row 4.4q-CU.
- A field span cut inside a character (`cfg_text = "é"`, `os_home = (0,1)`): Pruv reads `""`, the hand `"\u{FFFD}"`.
  Pre-existing (the old scanning `cfg_view` also gave `""`); the host never cuts a span there. Part of 4.4q-CU.
- `utility` mode 3 copies `String` → `Vec<u8>` once more than the hand: 4.4o-SB.
- `p100` bank-length case and the `ctl` / `cfg` index contracts: step wire and `requires`, not gaps of this step.

Tests (`bench/l3/opencode`, `pruvc-rs` main release binary):

| test | rs/rust | rs/llvm | before (4.4o) |
|---|---|---|---|
| `test_impl.py --verify-only` | VERIFIED | VERIFIED | VERIFIED |
| `test_impl.py --jobs 2` fixtures | 199/199 | 199/199 | 199/199 |
| `q.py --cases 300` (499 cases) | exact 406, 0 unexplained | exact 406, 0 unexplained | exact 406, 0 unexplained |
| `q_mutants.py --check` | all 16 markers present exactly once | | 16/16 |

Rows:

| row | state | note |
|---|---|---|
| 4.4o-CV | closed | one `utf8_valid(cfg_text)` per reading step, `cfg_view` has two boundary tests and no scan |
| 4.4o-PL | closed | `np > 1e9` first, as the hand |
| 4.4o-SB (language) | partly closed | `p100` borrows the parent; `utility` mode 3 `to_bytes(d)` stays (no `str` → `[u8]` move) |
| 4.3g-T | partly closed | `dirname` borrows; `Attach` already matches the hand; the request pool copy is the step wire |
| 4.4o-EB | open | blocked by 4.4o-SB (`await_req` takes a `[u8]` pool) |
| 4.4q-CU (new) | open | whole config text not valid UTF-8 ⇒ `(3,6)` where the hand decodes lossily; a span cut inside a character reads `""` where the hand gives `U+FFFD`; unreachable from the host |
| 4.4q-DN (new) | open | `dirname` on repeated slashes: Pruv follows `path.posix.dirname` (`"//a"` ⇒ `"//"`, `"a//b"` ⇒ `"a/"`), the hand gives `""` (its `t[..1]` is trimmed to empty), `"a"`; the hand is the side that differs from the reference |

## 12. Step 4.4t (rows 4.4o-SB, 4.4o-EB, 4.4q-DN)

L-gaps 9 added `str_into_bytes(move t) -> [u8]` (hand `String::into_bytes`, R-6b-6). The two places that copied a
`String` into a `[u8]` pool now move it, and the export root is a borrowed `str` like the hand's `conf.temp_root`.
`path::normalize_bytes` (the byte form kept only for the export root) is removed. The hand `dirname` now follows the
pinned TypeScript reference (Node `path.posix.dirname`), and the Pruv `dirname` takes the hand's loop shape. No
compiler, `pkgs/` or test change.

Per-function structure (generated Rust `work/verify/rs_rust/root_gen.rs` vs hand `hand/opencode.rs`):

| function | before | Pruv now (generated Rust) | hand Rust | difference that stays |
|---|---|---|---|---|
| `path::utility` mode 3 | one `utf8_valid`, `data_dir`, `to_bytes(d)` (second buffer + copy) | one `from_utf8(..).is_ok()`, `data_dir(..)`, `(d).into_bytes()` | `decode`, then `data_dir(&conf).into_bytes()` | none in the value path; the `> 1073741823` length test is the result packet protocol |
| `export::start_export` | `Vec` with `reserve(tl + 10)`, two appends of config bytes and `"/opencode-"`, `normalize_bytes` (offset pairs, sizing loop, second buffer) | one `from_utf8(..).is_ok()` (fail ⇒ `(0,3,6)`), `root: &str = cfg_view(..)`, `join(root, "opencode-")` (the `normalize` of §10), `(joined).into_bytes()` as the request pool | `make_temp(conf.temp_root)`: `join(root, "opencode-")` | the pool goes to the host's mkdtemp request where the hand calls `DirBuilder` itself (step wire, 4.3g-SQ); the per-step UTF-8 check where the hand decodes once per run (same as `p100`, 4.4q-CU) |
| `path::dirname` | `&str -> &str`; counting-down `while` with an `endp == -1` loop condition and an `i64` sentinel | `&str -> &str`; counting-down `while i >= 1`, `end = Some(i); break;`, `match end { None => "/" or ".", Some(e) => "//" or get_unchecked(0..e) }` | `&str -> &str`; `for i in (1..n).rev()`, `end = Some(i); break;`, `match end { None if root => "/", None => ".", Some(1) if root => "//", Some(i) => &p[..i] }` | `while` vs `for .. rev()` (row 4.4t-RV: no `rev` in `for`, R-6q-3); `i64` vs `usize` index (the language's index type); the slice is unchecked (proved from the break state), the hand's `&p[..i]` checks the char boundary |
| `path::normalize_bytes` | byte `normalize` for the export root | removed | no counterpart | - |

Behaviour:
- `dirname` values do not change on the Pruv side. The hand changes on repeated slashes (`"//a"` ⇒ `"//"`,
  `"a//b"` ⇒ `"a/"`, `"//a//b"` ⇒ `"//a/"`), now equal to Node on 17 inputs (`""`, `"/"`, `"//"`, `"///"`, `"a"`,
  `"a/"`, `"a//"`, `"/a"`, `"//a"`, `"a//b"`, `"/a/b/"`, `"a/b//c//"`, `"//a//b"`, `"./x"`, `"é/ü"`, `"/é//"`,
  `"///a"`). Reference decision for 4.4q-DN: the pinned TypeScript (Node) is the oracle (RUST_SHAPE_RESET §1 and
  §7), so the hand follows it. The hand's only `dirname` caller is `admit` (`canonicalize(dirname(&db))`).
- `start_export`: a temp-root span of 1000000001 ..= 1073741000 bytes is now engine-missing `(0,1,3)` (it was a
  normalized path); Begin refuses a configuration text over 1000000000 bytes, so no admitted case reaches it. A
  configuration text that is not valid UTF-8 as a whole is now `(0,3,6)` here too (as `p100` and `utility` mode 3,
  row 4.4q-CU); a root span cut inside a character reads `""` (so the prefix is `"opencode-"`). The host builds
  `cfg_text` from Rust `String`s and cuts spans at field ends, so no test case reaches either.

Tests (`bench/l3/opencode`, `pruvc-rs` release binary of main `1a8f7c8d`, which includes L-gaps 9; the worktree has
no build of its own and the compiler was not changed):

| test | rs/rust | rs/llvm | before (4.4q) |
|---|---|---|---|
| `test_impl.py --verify-only` | VERIFIED | VERIFIED | VERIFIED |
| `test_impl.py --jobs 2` fixtures | 199/199 | 199/199 | 199/199 |
| `q.py --cases 300 --jobs 2` (499 cases) | exact 406, 0 unexplained | exact 406, 0 unexplained | exact 406, 0 unexplained |
| `q_mutants.py --check` | all 16 markers present exactly once | | 16/16 |
| hand `opencode.rs` (`cargo build --release`, serde_json + rusqlite bundled) | builds, no warnings | | builds |

Not measured: structure not identical (4.4t-RV, the step wire, 4.4o-VP in `normalize`). No audit and no
other-model comparison in this step: no new result meaning on any reachable input (the moved buffer holds the same
bytes; the export prefix is `normalize(root + "/opencode-")` both before and after).

Rows:

| row | state | note |
|---|---|---|
| 4.4o-SB | closed | `utility` mode 3 moves the `String` (`into_bytes`); `p100` was closed in 4.4q |
| 4.4o-EB | closed | export root borrowed as `str`, `join` + `into_bytes`; `normalize_bytes` removed |
| 4.4q-DN | closed | reference = pinned TypeScript (Node); the hand `dirname` follows it, same loop/`break`/`Option` shape as Pruv |
| 4.4t-RV (new, language) | open | hand `for i in (1..n).rev()`; Pruv has no `rev` adapter (R-6q-3, `parse/for_iterable`), so `dirname` keeps a counting-down `while` |
| 4.4o-VP (compiler) | open | `vec_push` of a view in `normalize` keeps the `2^40` length check; compiler work (pruvc-rs/ is not changed in this step) |
| 4.4q-CU | open | unchanged meaning; `start_export` now has the same whole-text check |
