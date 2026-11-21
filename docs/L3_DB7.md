# L3 DB7: grok per-line allocation debt (D-R06)

Scope: bench/l3/grok and additive functions in bench/l3/pkgs. Debt rows D-R06-1..3 and D-R06-5 are in
docs/L3_RESTRUCTURE_SPEC.md §7.5.

## Profile before the fix

`sample` on the dense-8m pruv row (pruv 362.8 ms against hand 16.9 ms on the baseline run). The row has 2,857,588
lines and almost all of them are `0` or `{}`, so the fixed cost of each line is the whole cost. Inclusive samples
under the inlined `event_step` body:

| Part | Share | Cause |
|---|---|---|
| `RawVec::shrink` → `realloc` | about 1/3 | `parse::scan` makes a 192-slot tape and `truncate` shrinks it to the records count |
| `JsonDocument` drop and other `free` | about 1/3 | source, tape and frame stack freed every line; xzone `free` reads the clock each call |
| `calloc` + `memset` | about 1/6 | owned span copy (`lines::parse_range`), 1536-byte tape, frame stack |
| parse, field lookup, rest | the remainder | `access::field` 29 samples; the ASCII key path does not allocate |

`parse_range`'s `copy(owned)` and `copy(tape)` are already moves in the generated Rust. The cost is the allocation
count, not copies.

## What changed

- `bench/l3/pkgs/parse/parse.pruv`: new `scan_into(input, lo, hi, inout tape, inout stack)`. It scans `input[lo..hi]`
  in place into caller scratch. Kind 0 is ok with `records`, kind > 0 is a reject at a position relative to `lo`, and -1 / -2
  mean the tape or the stack is too small. Existing `scan` is unchanged.
- `bench/l3/pkgs/lines/lines.pruv`: new `scan_span` (size check, one `validate_utf8` over the span, then `scan_into`),
  `build_span` (owned `JsonDocument` from a scanned span: one source copy, an exact tape, `json::check_tape`) and
  `parse_span` (same result as `parse_range`, with a fallback to `parse_range` when the scratch is too small).
  `parse_range` is unchanged.
- `bench/l3/pkgs/access/access.pruv`: new `view_root_type(data, lo, tape, records)`. It reads the root `type` value from the
  scratch tape without a document: 1 user, 2 assistant, 0 other, -1 unknown (escaped key or value). It uses a raw byte compare
  bounded by `len(data) - lo`, so it has no 16 MiB absolute-offset limit like `jstr::ascii_equal`.
- `bench/l3/pkgs/turns/turns.pruv`: new `empty_buffer()`. The grok collector starts its buffer empty instead of at 16 slots.
- `bench/l3/grok/read.pruv`: `window` allocates one 24576-word tape and one 516-word stack. `line_step` scans each line
  into them, and builds an owned document only when `view_root_type` is not 0 (user, assistant or unknown). Other lines
  (`0`, `{}`, other types) allocate nothing. Spans that the scratch cannot hold still go through `parse_range`.

No proof invariant replaces a run-time check, so there is no audit for this bundle.

## Tests

All green on 86a29a08:

- grok `test_impl.py` (4 pairs, 254 fixtures). The full run under load 23 had 46 `acp-*` failures where the TS reference
  timed out at `initialize`. A rerun `--reuse-verified --only acp- --jobs 2` at load 6 passed 46/46 on all 4 pairs. The
  other fixtures passed in the full run.
- grok `q.py`: no unexplained differences. `q_mutants.py`: killed 16/16, 0 survived.
- `mcp/test_impl.py` 22/22 on both pairs, `qwen/test_impl.py` 26/26, `json/run_a5.py` 83/83, codex `test_impl.py`
  136/136 on rust and llvm (codex imports `access` and `lines`).
- `perf/run_perf.py check`: all four M/L rows `pruv == hand`.

## Perf

`run_perf.py all --rounds 7`. Load average 3.65 / 4.64 / 11.84 before, 3.03 / 4.31 / 11.31 after. Cells:
`bench/l3/grok/perf/work/perf_cells.json`.

| row | time before | time after | RSS before | RSS after (pruv / hand) |
|---|---:|---:|---:|---:|
| hist-ord-64k | 2.60x | 3.97x | 1.086x | 1.11x (2.52 / 2.28 MB) |
| hist-ord-2m | 2.62x | 2.66x | +5.6 MB | 1.25x, +5.1 MB (25.3 / 20.2 MB) |
| hist-ord-8m | 2.64x | 2.70x | +5.6 MB | 1.17x, +5.5 MB (38.7 / 33.2 MB) |
| hist-dense-2m | 21.3x | 2.02x | +8.6 MB | 2.73x, +6.8 MB (10.7 / 3.9 MB) |
| hist-dense-8m | 22.7x | 2.03x | +8.6 MB | 1.66x, +6.8 MB (17.0 / 10.2 MB) |

Pruv time on dense-8m went from 362.8 ms to 33.6 ms. Before numbers are from the D-R06 rows.

## Debt and gaps

| Id | Cell | Cause | Fix |
|---|---|---|---|
| D-DB7-1 | dense time M 2.02x, L 2.03x | Inferred, no profile after the fix: each line still runs `validate_utf8` and `scan_into` as two passes, plus `view_root_type`, where hand reads the line in one pass. | Fold the UTF-8 check into the scanner's string path. Profile first. |
| D-DB7-2 | ord time M 2.66x, L 2.70x (D-R06-2 unchanged) | Inferred, not isolated: user/assistant lines still get an owned document (`build_span`: source copy, exact tape copy, `json::check_tape`), and `entry::feed` still uses generic field lookups. | Feed turns from the scratch tape without an owned document (needs G-DB7-1), and do one raw-key pass for the turn fields. |
| D-DB7-3 | ord-64k time S 3.97x (was 2.60x) | Inferred, not isolated: a fixed per-run cost that the larger rows hide. The candidates are the 24576-word scratch tape and the stack allocated per `window` call. | Measure the scratch allocation; size it from the window length. |
| D-DB7-4 | RSS fixed part, +5.1 to +5.5 MB ord, +6.8 MB dense (D-R06-3) | Inferred: the collector slot stride is 65,584 bytes, about 3 pages per slot over 200 slots, and the buffer keeps that capacity. The empty start buffer removed only the 16-slot first step. | Size slots by the content they hold, not a fixed stride. |
| G-DB7-1 | `json::JsonDocument` owns `source: [u8]` and `tape: [i64]`, and the language has no borrowed slice type for a field. A document over a line inside the window therefore has to copy the line and the tape. | `build_span` copies only for lines that need a document; other lines use the scratch tape through `view_root_type`. |

Status: DB7: D-R06-1 partial, dense time 21.3x / 22.7x to 2.02x / 2.03x; rest is D-DB7-1
Status: DB7: D-R06-2 open, ord time 2.66x / 2.70x; cause moved to D-DB7-2
Status: DB7: D-R06-3 partial, empty start buffer; fixed RSS +5.1 to +6.8 MB remains as D-DB7-4
Status: DB7: D-R06-5 open, ord-64k time 3.97x, RSS 1.11x; see D-DB7-2 and D-DB7-3
Status: DB7: G-DB6-2 partial, span parse landed in bench/l3/pkgs (`parse::scan_into`, `lines::scan_span`, `lines::build_span`, `lines::parse_span`); qwen still calls `parse_range` and bench/l3/lib is unchanged
