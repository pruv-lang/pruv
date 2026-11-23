# Grok RPC line path in the hand_rpc.rs shape (DB65, Rust-shape reset fragment)

Branch `lair/fragment-job-of-the-rust-55f3`. Scope: the per-line RPC path of `bench/l3/grok/rpc.pruv` (`parse_line` and
what it calls per line) rewritten in the shape of `bench/l3/grok/perf/hand_rpc.rs` `line()` / `rpc_lines()`. pruvc/ and
pruvc-rs/ were not touched. Compiler: pruvc-rs release (`PRUVC_RS`), pair rs/rust.

## Result

- Output: acp fixtures 46/46 on rs/rust before and after, same policy line (P1 x2, P10 x2, P6 x2). `run_perf.py check`:
  pruv == hand on rpc-lines-64k / 2m / 8m (folds -8061104937171452716, -412955933078626502, 8269868883797785166).
  No output difference.
- Time and peak RSS, §7 protocol (lock `/tmp/pruv-perf.lock`, 7 rounds, median):

| row | size | pruv ns | hand ns | time | pruv RSS | hand RSS | RSS |
|---|---|---:|---:|---:|---:|---:|---:|
| rpc-lines-2m | M | 8307937 | 7166542 | 1.159x | 4128768 | 3801088 | 1.086x |
| rpc-lines-8m | L | 33198291 | 28787521 | 1.153x | 10616832 | 10125312 | 1.049x |

  perf_cells: `bench/l3/grok/perf/work/perf_cells.json` (not committed; work/ is local).
  For reference: the old tape path was 3.49x at M (docs/L3_DB62.md, rs-rust 92.0 / 26.4 ms); the first DB65 cut without
  the trim fast path measured 1.598x M / 1.594x L.

## Structure: line path, Pruv old vs Pruv new vs hand

| | Pruv old (`parse_line`, rpc.pruv:619) | Pruv new (`scan`, rpc.pruv:408) | hand (`line`, hand_rpc.rs:575) |
|---|---|---|---|
| entry per line | trim copy, tape parse, `kind_of`, owned-str field reads | `trim_bounds` (borrowed), `valid`, one recursive `value` walk filling `Cap` | `trim_bounds`, one recursive walk filling captures |
| calls per line (plain request/reply) | parse + tape build + per-field `json_value` / str copies | `scan` -> `value` (recursion per nested value) -> `key_slot` per key; `line_kind`; `text_span` per text field | `line` -> walk -> key match; text decode on demand |
| copied bytes (plain line) | trimmed line + tape words + every owned field str | 0 (spans into the line) | 0 |
| copied bytes (escaped text) | as above | decoded text only, into the reused `buf` | decoded text only, into the reused `buf` |
| allocation sites per line | trim copy, tape `[i64]`, owned strs, result packet | none on the scan; `not_handled_line` one exact-size alloc (rpc.pruv:461); `buf` `resize` only when it grows (rpc.pruv:537) | `nh` Vec with exact capacity (hand_rpc.rs:604); `buf` reuse |
| scratch across lines | none | `inout c: Cap`, `inout buf: [u8]` (perf_root.pruv:170) | `&mut buf` |

Pruv adds proofs only: termination (`decreases d` on depth 128), the span bounds and overflow guards, and the
`dec` invariant `o <= q - s`.

## Cells over 1.0x and their line-level causes

- Time 1.16x (M and L):
  - `valid` (rpc.pruv:404) runs `utf8_valid` over the whole trimmed line. The hand checks UTF-8 only inside strings
    while it walks (hand_rpc.rs:138, :223). This is an extra full pass.
  - Lines that fall back to the tape (`fast_kind` result 3, rpc.pruv:557 / :805; perf_root.pruv:153, :161): error
    replies (`c.err == 6`, about 3% of the bench lines) and tracked objects with an escaped key (rpc.pruv:369). These
    pay the old cost.
  - The per-byte `stop` / `ws` loops and the explicit span guards in `value` (rpc.pruv:324) are bounds checks the hand
    avoids with slice iteration.
- RSS 1.09x M / 1.05x L: the tape fallback lines still allocate the tape and owned strs; the `Cap` struct and the
  `[i64]` acc are on the measure side only. Fixed overhead dominates at M.
- Fixed in this fragment: `trim_bounds` (rpc.pruv:70) walked the whole line with `space()` per byte; it now takes the
  hand's fast path (an ASCII non-space last byte ends the walk). That alone moved 1.60x to 1.16x.

## Gaps (could not be expressed without borrowed fields; closest no-copy shape kept)

- rpc.pruv:106 `Cap` holds `(start, end, kind)` offsets instead of borrowed `&[u8]` fields: Pruv structs cannot hold
  borrowed slices, and a `subview` cannot be bound to a local (it is allowed only as a direct argument to a non-inout
  array parameter). Each read re-slices from the line with a runtime bound check.
- rpc.pruv:404 `fn valid` wraps `utf8_valid` because a builtin call is not a valid `subview` context.
- rpc.pruv:369 an escaped key in a tracked object sets `c.fb` and the line goes to the tape: matching an escaped key
  needs a decoded copy, and the tape path already has it.
- rpc.pruv:805 / perf_root.pruv:161 error-object replies keep the tape path, because `error_detail` / `json_value` read
  the error object as a tree and the string/compound `json_value` text differs from the raw token.
- Span guards in `line_kind` (rpc.pruv:431), `not_handled_line` (:461), `text_span` (:537) are runtime checks, not
  facts carried out of the `value` recursion: the recursion's postcondition does not yet state that the captured spans
  lie inside the line.

## Hand baseline change

`hand_rpc.rs` kind 1 lane now builds the not_handled reply as a Vec with exact capacity (NH_HEAD, id, NH_TAIL) and
folds it, so the hand computes the same full result the product writes (§7: the hand does the full job). The hand
detail stays a fold.

## Who did what

Claude Opus 5.5 (this session) did all of it: the rewrite, proofs, tests, measurement and this note. No delegation,
because the work is a shell compile-verify loop on pruvc-rs and the write workers have no shell.
