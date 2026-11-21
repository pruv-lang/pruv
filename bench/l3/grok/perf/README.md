# Grok history core perf (pruv vs hand safe Rust)

Compares the generated Pruv `read::window` (bench/l3/grok/read.pruv: framing cursor -> entry::feed -> turns collector -> `packet::result3`)
with `hand_history.rs`, a single-file safe-Rust implementation of the same function: window bytes -> the same `Vec<i64>` read result
packet (`[status, 24, nrows, ntext, meta x24, rows, text bytes widened to i64]`). Result cells go to `work/perf_cells.json` for
`tools/perf_gate.py`.

## Run

```
python3 bench/l3/grok/perf/run_perf.py --rounds 5            # check + perf, writes perf/work/perf_cells.json
python3 bench/l3/grok/perf/run_perf.py check                 # build + word-for-word check only
python3 bench/l3/grok/perf/run_perf.py --rounds 1 --time-ms 50 --rows hist-ord-64k,hist-ord-2m   # smoke
python3 tools/perf_gate.py bench/l3/grok/perf/work/perf_cells.json
```

`--pruvc PATH` (or `$PRUVC`) selects the compiler; default `pruvc-rs/target/release/pruvc`. `~/.cargo/bin` is put on PATH for rustc.

## Files

- `run_perf.py` builds both sides, generates the rows, checks, measures, writes JSON.
- `perf_root.pruv` root with `pub fn grok_history_window(win, omitted, title, model)`, a one-line wrapper over `read::window`.
  run_perf.py copies it and the grok modules it reaches (util, packet, path, framing, entry, read, unedited) into `work/` and writes
  a manifest listing the packages those modules reach (json parse access jstr lines bytes text time names turns, from `../pkgs`).
- `hand_history.rs` the baseline. One pass over the window with a cursor; each line is scanned once into a reusable entry (no JSON
  tape), fed to a 200-slot ring collector, and the packet is written once into a `Vec` reserved at its exact size. JSON is a small
  hand-written validating scanner (serde_json is not linked). Scope limits: object keys are compared by raw spelling; only a quoted
  canonical 24-byte ISO timestamp counts (a numeric or digit-string timestamp, which the Pruv `iso_of` would convert, is treated as
  absent); nesting over 128 skips the line. The bench inputs stay inside this scope; the check below proves equality on them only.
- `perf_driver.rs` modes `check`, `time`, `rss`; compiled together with `gen.rs` and `hand_history.rs` into `work/main.rs`
  (`rustc -C opt-level=3 --edition 2021`).
- `work/` is gitignored scratch (`.gitignore` in this folder): sources, gen.rs, the driver, the row files, the result JSON.

## Rows

Generated deterministically by `run_perf.py` (fixed seeds) and written to `work/rows/*.bin` only so the driver can read them.
All rows are LF-terminated windows, `omitted=false`, title `Fixing the flaky retry in sync`, model `grok-4`.

| row | size | shape |
|---|---|---|
| hist-ord-64k | S (sanity) | ordinary session lines, 64 KiB |
| hist-ord-2m | M | ordinary session lines, 2 MiB |
| hist-ord-8m | L | ordinary session lines, 8 MiB (the window cap) |
| hist-dense-2m | M | dense tiny lines, 2 MiB |
| hist-dense-8m | L | dense tiny lines, 8 MiB |

Ordinary: a user line (plain string content, `<user_query>` wrapped, or a content-part array; `timestamp` or `created_at`) followed by
1..20 assistant lines (200..4000 byte texts with escapes and non-ASCII words, optional `tool_calls`, some null timestamps) and
occasional `tool` lines (ignored by the collector) carrying nested extra fields. Long assistant runs hit the 32 KiB reply budget; the
8 MiB window overflows the 200-slot ring, so `trimmed` is set. Dense: lines `0` and `{}` alternating, with a small user line and an
assistant line (two empty tool calls) every 400 lines: the case that used to build a large event array.

## Cells

Per row, two cells (`kind: proportional`, size S/M/L as in the table):

- `grok history <row> time` (ns): one `window()` call including the drop of the returned `Vec`. A fresh process per (row, impl,
  round); the implementation order is rotated every round; one warm-up call; each process times calls for `--time-ms` (at least 3
  calls) and reports the median; the cell is the median of the round medians. `runs` = rounds.
- `grok history <row> rss` (bytes): peak resident set size of the whole process from `/usr/bin/time -l` (`maximum resident set size`),
  median of 3 processes, one call with the window read first. RSS therefore includes the input window bytes (and the returned packet)
  for both sides. `runs` = 3.

## Check

`perf_driver check` runs both implementations on every selected row with `omitted=false` and `omitted=true` and compares the packets
word for word; any difference prints the first differing word and exits 1 (run_perf.py then stops before measuring). The time mode
also prints a checksum of the packet and run_perf.py fails if the two sides disagree.

## Not done

- The optional OLD column (the event-array version from commit ca24f9b) is not built.
- No over-long (> 1 MiB) line row: the drop path of `hand_history.rs` (`dropped_user`) is written from framing.pruv but not exercised.
