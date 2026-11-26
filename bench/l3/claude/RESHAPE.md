# Claude R08 import: Rust-shape reset (RUST_SHAPE_RESET §6 step 4.3h)

Scope: the Claude R08 slice (`import`, `turns`, `text`, `entry`, `framing`, `boundary`, `time` and the `a0`-`a4`, `q`
roots) against the hand baseline `handrust/src/lib.rs` (safe Rust, std only). Every hand function now carries a
`// Pruv: module::fn` comment naming its Pruv counterpart, and the crate header lists the Pruv functions with no hand
counterpart (`boundary::check_output`, `boundary::copy_span` and the probe roots, `import::last_max` /
`first_present` / `replaces`). The hand builds with `cargo build --release --offline` with no warnings; its code did
not change. pruvc/, pruvc-rs/, SPEC.md, docs/RUST_PARITY_SPEC.md and tests/ were not
touched. No module moved to `lib` / `pkgs` and no shared module changed (`json`, `lex`, `parse`, `lines`, `jstr`,
`access` come from `bench/l3/pkgs` unchanged), so no consumer runs were needed. This step supersedes the old
"claude frozen" note in AGENTS.md (RUST_SHAPE_RESET §6).

What went:

- Byte loops the hand writes as slice copies. `turns::put_str`, `put_nl` and `move_stamp` (three proved loops) are
  gone: `write_piece` / `write_begin` / `write_answer` call `copy_range` (a `str` or literal source) and
  `copy_within` for the completed stamp moved forward (hand `extend_from_slice`, `copy_within`).
  `boundary::copy_span` is one `copy_range`. `framing::skip_first` and `window_next` find the LF with `find_byte`
  (hand `iter().position`).
- A clipped copy made only for its length. `turns::begin` and `answer` called `text::clip_bytes` (a `substr`
  allocation) and kept only `len(cp)`; the new `text::clip_len` (hand `clip_len`) returns the boundary and the
  writers copy `prompt[0..n)` straight into the arena. `clip_bytes` stays as `substr(s, 0, clip_len(s, max))` for the
  A1 probe.
- A copied prefix on the dropped-user path. `read_window` allocated and `copy_span`ned the 8 KiB head before calling
  `text::dropped_user`; it now passes `subview(data, s, e)`.
- Flags the hand writes as `break`. The write-only `idle` flags in `read_head`, `tail_info`, `answer_blocks` and
  `read_window` are gone. `Frame::Dropped` / `Frame::Done` arms and the "all three head fields found" exit are `break`
  (they set `offset = len(data)` before). `text::one_line`'s over-limit exit is `break` instead of `p = 0; i = n`.
- Work the hand does not do. `consume_entry` read and canonicalized the entry's timestamp before it knew whether the
  entry was a prompt or a reply; the timestamp is now read where the hand reads it (after a human prompt is found, and
  after the assistant content kind is 4 or 5), and `stamp_from` states the 0-or-24 length in its `ensures` instead of
  a re-check in the caller. The pub `consume_entry` wrapper that called `consume_entry_of(.., root_of(doc), ..)` is
  gone (one function, `r` passed by `read_window`).
- Copies of owned values. `copy(v)` / `copy(text)` / `copy(s)` / `copy(x)` of match binders and owned locals in
  `import` and `entry` are `move`. `stamp_from` returns `Stamp::At("")` (no `copy(none)` of a local `""`). In
  `time`, `absent` / `defect` / `from_integer` / `from_string` / `canonical_copy` move the array they just built into
  `Iso` (they copied it); `bytes24` and `canonical_copy` copy the 24 bytes with one `copy_range` (hand
  `copy_from_slice`); the unused `present` is gone.
- A parse path the hand does not have (review). `entry::window_line` re-parsed a line ending in a deep singleton
  number array (`..:[[0]]}`) from a shortened copy (`terminal_array`, `scan_shortened`, `number_byte`: three more
  scans, a zero-filled copy, a second parse on failure) and returned a document whose source was not the line.
  `read_window` now calls `entry::parse_line` like the hand's `parse_line`; the three helpers are gone.
- Work after the decision is known (review). `entry::array_content` copied the first text piece into a `String`
  and counted ignored blocks in an unused `seen`; it now holds the first piece (moved) and starts joining at the
  second, like the hand's `first` / `joined`. Titles: `entry::ranked` takes the rank held so far and drops a lower
  rank before `pick_title` reads and blank-checks the text (hand `if rank >= self.rank { pick(..) }`); the callers no
  longer call `import::replaces` after the text was read. `text::trim_span` leaves its two loops by `break` (it set
  `lo = i; i = n` and `hi = k; k = lo`); `time::digit_value` returns -2 as soon as the value passes 2^53-1 (an `over`
  flag read the rest).

## 1. Structure table (§1.5 form)

| step | Pruv before (4.3g tree) | Pruv now (4.3h) | hand Rust (`handrust/src/lib.rs`) | copies / allocations (now vs hand) |
|---|---|---|---|---|
| line framing (`edge_next`, `skip_first`, `window_next`) | byte loops for the LF in `skip_first` / `window_next` | `find_byte` | `iter().position` | 0 vs 0 |
| head loop exits | `idle` flag, `offset = len(data)` | `break` | `break` / `return` | same |
| prompt clip (`begin`) | `clip_bytes` `substr`, then `len` | `clip_len`, copy `prompt[0..n)` | `clip_len` + `extend_from_slice` | 0 extra vs 0 |
| reply clip (`answer`) | `clip_bytes` `substr`, then `len` | `clip_len` | `clip_len` | 0 extra vs 0 |
| arena writes (`write_piece`, `write_begin`, `write_answer`) | `put_str` / `put_nl` / `move_stamp` loops | `copy_range`, `copy_within` | `extend_from_slice`, `copy_within` | 0 vs 0 |
| turns store | `st` / `sl` / `buf` word ring + arena | same | `VecDeque<Draft>` | row VL / D-R08-2 |
| dropped user line | alloc + `copy_span` of the head, then scan | `subview` | `from_utf8_lossy` + several `find` passes | 0 vs 1 (row 4.3h-D) |
| entry timestamp | read and checked first for every entry | read at begin / before content | read at begin / before content | same |
| `consume_entry` | pub wrapper + `consume_entry_of` | one function | `consume` | same |
| title / cwd / model locals | `copy(v)` of binders | `move` | owned moves | 0 vs 0 |
| `lastAt` | `copy(iso.text)` | same | `clone` of the stamp | row 4.3d-M |
| window line parse | projection of a terminal `[[n]]` chain, shortened copy, then full parse | `parse_line` | `parse_line` | 0 extra vs 0 |
| array prompt join | every piece into `String`, `seen` counter | first piece held, join from the second | `first` + `joined` | 0 extra for one piece vs 0 |
| title rank | text read and blank-checked, then `replaces` | rank checked in `ranked` before the read | `rank >= self.rank` before `pick` | same |
| `trim_span` | `lo = i; i = n`, `hi = k; k = lo` exits | `break` | `trim_matches` | same |
| `Iso` build (`time`) | `copy` of the local array, 24-byte loops | `move`, `copy_range` | owned `[u8; 24]`, `copy_from_slice` | 0 extra vs 0 |
| JSON | `pkgs/parse` tape + `access` readers | same | `Parser` → `Value` tree | accepted representation |
| packet out | `new_packet` zero fill, `write_bytes` widens each byte to `i64`, `check_output` | same | `Vec::with_capacity` + `extend(map(b as i64))` | D-R08-3 same as hand; zero fill row 4.3h-A; CK |
| `one_line` | `substr` push per char, `p = 0; i = n` exit | `substr` push per char, `break` | `push(c)` | row 4.3h-L |

## 2. Differences that remain

- D-R08-3 (ABI, kept): text crosses as one `i64` per byte in the packet. The hand does the same
  (`out.extend(text.iter().map(|&b| b as i64))`); a byte lane needs a new packet ABI on both sides.
- D-R08-2 / VL (pkgs, open): evicted turns are not dropped like `VecDeque::pop_front`; the ring keeps 200 slots of 8
  words and an append-only arena (`Vec<struct>` is rejected by the LLVM backend, row VL).
- 4.3d-M (language): `last = copy(iso.text)` in `tail_info` / `read_window`; a struct array field cannot be moved.
- CK (bench): `boundary::check_output` re-checks the packet at run time; the hand has no such pass.
- JSON tape (accepted): Pruv reads `pkgs/parse` tapes, the hand builds a `Value` tree.
- 4.3h-R, 4.3h-W, 4.3h-O, 4.3h-Z, 4.3h-T, 4.3h-P, 4.3h-V, 4.3h-L, 4.3h-D, 4.3h-A, 4.3h-Q (bench, §3).

## 3. Structure review (hand vs Pruv, per function) and what was applied

One Codex review (gpt-6.2-astra, high), read-only, of every function against `handrust/src/lib.rs`: 1 behavior
difference, 16 structure findings, 3 notation findings; about 60 functions match. It ran in the background and its
result was read back inside the step. Applied (all verified, §4):

- Behavior / structure: `entry::window_line` returned a document parsed from a shortened copy (`{"x":[[0]]}` came
  back with `x = []`); the projection is removed and `read_window` calls `parse_line` (no packet changed: no reader
  consumed such a member).
- `entry::array_content`: hold the first piece, join from the second, drop `seen`.
- Titles: rank compared before the text is read (`entry::ranked(.., best)`); `import::replaces` is now only the A2
  probe helper, and its `best == 0` branch (the same as `rank >= best` for ranks 1..3) is gone.
- `time`: `move` into `Iso`, `copy_range` for the 24 bytes, `present` removed, `digit_value` returns at overflow.
- `text::trim_span`: `break` exits.

Not applied (rows). Each changes contracts across several functions or a shared package, more than the time cap
allowed:

| row | finding | where | what the hand does |
|---|---|---|---|
| 4.3h-R | `entry::root_of` allocates 8 words and looks up 7 keys for every object, title-only and empty objects included | `entry::root_of`, `import::read_window` / `consume_entry` | `get` per key in the branch that needs it |
| 4.3h-W | `is_blank` scans the leading whitespace, then `engine_line` trims again; `origin` is read before the tags are known to match | `entry::string_content` / `array_content`, `text::is_blank` / `engine_line` | one `es_trim`, `task_origin` only when both tags match |
| 4.3h-O | read-only uses (block type, title blank check, a reply that keeps 0 bytes) still ask `access::string_text_owned` for an owned decode | `entry::string_field_of` / `block_of`, `import::answer_blocks` / `consume_entry` (`pkgs/access`, `pkgs/jstr`) | borrows `&str` from the parsed value |
| 4.3h-Z | `stamp_from` turns the 24 `Iso` bytes into `str` with `jstr::adapter_probe` and checks the length again, because `turns::begin` / `answer` take `str` | `import::stamp_from`, `turns` | passes the `[u8; 24]` stamp |
| 4.3h-T | `turns::answer` with empty text or a full budget still runs `reserve`, `write_answer` (0-byte copies) and rewrites the slot | `turns::answer` | sets `completed` and returns |
| 4.3h-P | `new_slots` fills 200 ordinals with -1, `begin` writes an ordinal, `plan` stores each end, `write_turns` reads them back; a forward cursor would do (with VL) | `turns` | `VecDeque` order, `kept` count |
| 4.3h-V | the canonical ISO check exists twice (`str` and raw bytes), reads all 17 digits before deciding; `format_iso` zero-fills 24 bytes and has an unreachable empty return | `time::canonical_iso` / `canonical_at` / `format_iso` | one `canon`, first mismatch returns; template + `put_digits` |
| 4.3h-L | `one_line` pushes `substr(s, i, j)` per character and checks the pending space together with the next character; `one_line("")` runs when no title was found | `text::one_line`, `import::tail_info` / `read_window` | `push(c)`, `then` closure |
| 4.3h-D | the hand's `dropped_user` decodes the 8 KiB prefix with `from_utf8_lossy` and runs several `find` passes; Pruv scans once on bytes | hand `dropped_user` | hand candidate (Pruv shape is the cheaper one) |
| 4.3h-A | `boundary::new_packet` zero-fills the packet that is then overwritten | `boundary`, `import::*_result` | `Vec::with_capacity` + `extend` |
| 4.3h-Q | `framing.pruv` is byte-identical to `bench/l3/qwen/framing.pruv`; not moved to `pkgs` in this step | `claude/framing`, `qwen/framing` | one module |

Also kept: `turns::tool` caps the tool counter at 10^15 (proof bound; unreachable with 8 MiB input) and the window
keeps the model token when it equals the held one (one compare saves a copy; the review counts it as a choice).

## 4. Results

After the last code change, `rs/rust` and `rs/llvm`:

| check | rs/rust | rs/llvm |
|---|---|---|
| root verify (one root at a time, 300 s cap) | a0 3 s, a1 4 s, a2 26 s, a3 92 s, a4 77 s, q_root 22 s, root 65 s; all VERIFIED | - |
| `test_a0.py` | 272/272 | 272/272 |
| `test_a1.py` | 2884/2884 | 2884/2884 |
| `test_a2.py` | 2443/2443 | 2443/2443 |
| `test_a3.py` | 134/134 | 134/134 |
| `test_a4.py` | 270/270 | 270/270 |
| `q.py --count 300` (270 fixed + 300 generated, seed 20261114) | 570/570, agrees with the oracle | 570/570, agrees with the oracle |
| `q_mutants.py` originals | 18 / 18 present exactly once | - |
| hand `handrust` | `cargo build --release`, 0 warnings | - |

No case needed a rerun. The frozen py pairs were not built; `q.py` ran on the two rs pairs only. No perf timing was
run.
