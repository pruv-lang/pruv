# Grok history core perf (pruv vs hand safe Rust)

Compares the generated Pruv `read::window` (bench/l3/grok/read.pruv: framing cursor -> entry::feed -> turns collector -> `packet::result3`)
with `hand_history.rs`, a single-file safe-Rust implementation of the same function: window bytes -> the same `Vec<i64>` read result
packet (`[status, 24, nrows, ntext, meta x24, rows, text bytes widened to i64]`). Result cells go to `work/perf_cells.json` for
`tools/perf_gate.py`.

## Run

```
python3 bench/l3/grok/perf/run_perf.py --rounds 5            # check + perf, writes perf/work/perf_cells.json
python3 bench/l3/grok/perf/run_perf.py check                 # build + word-for-word check only
python3 bench/l3/grok/perf/run_perf.py check --pair py/rust --rows hist-ord-64k
python3 bench/l3/grok/perf/run_perf.py --rounds 1 --time-ms 50 --rows hist-ord-64k,hist-ord-2m   # smoke
python3 tools/perf_gate.py bench/l3/grok/perf/work/perf_cells.json
```

`--pair` selects `py/rust`, `py/llvm`, `rs/rust` (default), or `rs/llvm`.
For `rs/*`, `--pruvc PATH` (or `$PRUVC`) selects the compiler, which defaults to `pruvc-rs/target/release/pruvc`.
For `py/*`, the script runs the checkout's `pruvc/pruvc.py` with `$PRUV_PYTHON` if set, then tries the checkout's
`.venv/bin/python`, `~/Projects/pruv/.venv/bin/python`, and the current interpreter in that order.
`~/.cargo/bin` is put on PATH for rustc. LLVM pairs link the generated Rust bindings, LLVM object, and Pruv runtime
through the shared benchmark helper. Set `PRUV_CLANG` to select clang.

The default build and reports are in `work/`. Other pairs put their driver and reports in
`work/py-rust/`, `work/py-llvm/`, or `work/rs-llvm/`. All pairs use the same deterministic rows in `work/rows/`.
The Python CLI exposes `check` and `all`. The built `perf_driver` also accepts `time` and `rss` directly.

## Files

- `run_perf.py` builds both sides, generates the rows, checks, measures, writes JSON.
- `perf_root.pruv` root with two entries: `pub fn grok_history_window(win, omitted, title, model)`, a one-line wrapper over
  `read::window`, and `pub fn grok_rpc_lines(win) -> [i64]` (15 words, see Rows). run_perf.py copies it and the grok modules it
  reaches (util, packet, path, framing, entry, line, read, rpc) into `work/` and writes a manifest listing the packages those
  modules reach (json parse access jstr lines tail bytes text time names turns transport, from `../pkgs`).
- `hand_rpc.rs` the rpc-lines baseline, `pub fn rpc_lines(win) -> Vec<i64>`. One pass over the window; each line is trimmed
  (ECMAScript whitespace as UTF-8, the steps of `rpc::trim_bounds`) and scanned once by a validating recursive-descent JSON scanner
  (strict RFC 8259, invalid UTF-8 fails, depth over 128, over 1048576 records (keys and values) or over 16 MiB fails) that only
  records the spans of the members the pump needs (root id/method/params/error, params.update.content.text, error.message/code).
  No tape, no per-line copy; strings are decoded into one reused buffer only when they hold escapes, the error detail is folded
  straight from the source. Scope limits: object keys are compared by raw spelling (a key written with escapes never matches) and
  the raw `error.code` is the source token of the value (equal to `util::json_value` for JSON integer codes); duplicate keys: the
  last occurrence wins. The rows stay inside this scope (plain ASCII keys, integer codes); the check proves equality on them only.
- `hand_history.rs` the baseline. One pass over the window with a cursor; each line is scanned once into a reusable entry (no JSON
  tape), fed to a 200-slot ring collector, and the packet is written once into a `Vec` reserved at its exact size. JSON is a small
  hand-written validating scanner (serde_json is not linked). Scope limits: object keys are compared by raw spelling; only a quoted
  canonical 24-byte ISO timestamp counts (a numeric or digit-string timestamp, which the Pruv `iso_of` would convert, is treated as
  absent); nesting over 128 skips the line. The bench inputs stay inside this scope; the check below proves equality on them only.
- `perf_driver.rs` modes `check`, `time`, `rss`; compiled together with `gen.rs`, `hand_history.rs` and `hand_rpc.rs` into
  `work/main.rs` (`rustc -C opt-level=3 --edition 2021`). A file whose basename starts with `rpc-` runs the rpc pair
  (`grok_rpc_lines` / `hand_rpc::rpc_lines`), every other file the history pair.
- `work/` is gitignored scratch (`.gitignore` in this folder): sources, gen.rs, the driver, the row files, the result JSON.

## Rows

Generated deterministically by `run_perf.py` (fixed seeds) and written to `work/rows/*.bin` only so the driver can read them.
All rows are LF-terminated windows. History rows: `omitted=false`, title `Fixing the flaky retry in sync`, model `grok-4`.

| row | size | shape |
|---|---|---|
| hist-ord-64k | S (sanity) | ordinary session lines, 64 KiB |
| hist-ord-2m | M | ordinary session lines, 2 MiB |
| hist-ord-8m | L | ordinary session lines, 8 MiB (the window cap) |
| hist-dense-2m | M | dense tiny lines, 2 MiB |
| hist-dense-8m | L | dense tiny lines, 8 MiB |
| rpc-lines-64k | S (sanity) | ACP stdout lines, 64 KiB |
| rpc-lines-2m | M | ACP stdout lines, 2 MiB |
| rpc-lines-8m | L | ACP stdout lines, 8 MiB |

Ordinary: a user line (plain string content, `<user_query>` wrapped, or a content-part array; `timestamp` or `created_at`) followed by
1..20 assistant lines (200..4000 byte texts with escapes and non-ASCII words, optional `tool_calls`, some null timestamps) and
occasional `tool` lines (ignored by the collector) carrying nested extra fields. Long assistant runs hit the 32 KiB reply budget; the
8 MiB window overflows the 200-slot ring, so `trimmed` is set. Dense: lines `0` and `{}` alternating, with a small user line and an
assistant line (two empty tool calls) every 400 lines: the case that used to build a large event array.

rpc-lines (`gen_rpc`, seeded): about 85% `session/update` notifications (mostly `agent_message_chunk` with 20..2000 byte texts, some
`agent_thought_chunk`, a few contentless `tool_call`), plus `session/request_permission` server requests (integer or string id),
responses with id 2 (result object) and other ids, error responses with id 2 (non-ASCII message, some over 300 units, and one whose
message is a number; integer codes), notifications with raw `\u` escapes (accent, surrogate pair, occasionally a lone `\ud800`),
a few blank lines, a few space/tab/CR padded lines and a few truncated (malformed) lines. One call = every line of the window
through the per-line work of the rpc pump (`rpc::parse_line`, `kind_of(d, 2)`, then by kind the not-handled reply, the error detail
and raw code, or the method and `params.update.content.text` decodes), no process and no event loop. Result: 15 words
(0 lines, 1 failed parses, 2 kind 0, 3 kind 1, 4 kind 2, 5/6 text bytes/fold, 7/8 not-handled bytes/fold, 9/10 detail bytes/fold,
11/12 code bytes/fold, 13/14 method bytes/fold; fold `s = (s * 31 + byte) % 1000000007`, counters add modulo 1000000007).

## Cells

Per row, two cells (`kind: proportional`, size S/M/L as in the table). Names are `grok history <row> ...` for history rows and
`grok rpc <row> ...` for rows whose name starts with `rpc-`:

- `grok history <row> time` (ns): one `window()` call including the drop of the returned `Vec`. A fresh process per (row, impl,
  round); the implementation order is rotated every round; one warm-up call; each process times calls for `--time-ms` (at least 3
  calls) and reports the median; the cell is the median of the round medians. `runs` = rounds.
- `grok history <row> rss` (bytes): peak resident set size of the whole process from `/usr/bin/time -l` (`maximum resident set size`),
  median of 3 processes, one call with the window read first. RSS therefore includes the input window bytes (and the returned packet)
  for both sides. `runs` = 3.

## Check

`perf_driver check` runs both implementations on every selected row (history rows with `omitted=false` and `omitted=true`, rpc rows
once) and compares the packets word for word; any difference prints the first differing word and exits 1 (run_perf.py then stops before measuring). The time mode
also prints a checksum of the packet and run_perf.py fails if the two sides disagree.

## Not done

- The optional OLD column (the event-array version from commit ca24f9b) is not built.
- No over-long (> 1 MiB) line row: the drop path of `hand_history.rs` (`dropped_user`) is written from framing.pruv but not exercised.
