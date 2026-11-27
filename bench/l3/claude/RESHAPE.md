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
| packet out | `new_packet` zero fill, `write_bytes` widens each byte to `i64`, `check_output` | 4.4c: `text` `[u8]` and `turns` `[i64]` built apart, `out` reserved and extended (`arr_push` / `arr_append`) | `Vec::with_capacity` + `extend(map(b as i64))` | D-R08-3 same as hand; CK |
| `one_line` | `substr` push per char, `p = 0; i = n` exit | 4.4c: pending space, `subview` push per char, width budget | `push(c)` | row 4.4c-OL |

## 2. Differences that remain

- D-R08-3 (ABI, kept): text crosses as one `i64` per byte in the packet. The hand does the same
  (`out.extend(text.iter().map(|&b| b as i64))`); a byte lane needs a new packet ABI on both sides.
- D-R08-2 / VL (pkgs, open): evicted turns are not dropped like `VecDeque::pop_front`; the ring keeps 200 slots of 8
  words and an append-only arena (`Vec<struct>` is rejected by the LLVM backend, row VL).
- 4.3d-M (language): `last = copy(iso.text)` in `tail_info` / `read_window`; a struct array field cannot be moved.
- CK (bench): `boundary::check_output` re-checks the packet at run time; the hand has no such pass.
- JSON tape (accepted): Pruv reads `pkgs/parse` tapes, the hand builds a `Value` tree.
- 4.3h-P (part), 4.3h-O, 4.3h-D, 4.3h-Q (bench, §3) and the 4.4c rows (§5).

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

## 5. Step 4.4c: bench rows T, R, W, Z, V, L, A (and part of P)

Closed, in the hand shape (`handrust/src/lib.rs`):

- 4.3h-T: `turns::answer` returns at once without an open turn, writes `completedAt` first (`complete`: in place when
  a stamp exists, else reserve + append), then returns on empty text or a full budget before any reserve or copy.
- 4.3h-R: `entry::root_of` / `user_text_of` are gone. `import::consume_entry(doc, ty, ..)` reads each key where the hand
  `consume` does (`timestamp` before begin and before content, `isApiErrorMessage` only in the synthetic branch);
  `read_window` reads `type` with `entry::string_id`.
- 4.3h-W: `entry::is_human_text` trims once (`trim_span`), then `text::interrupt_line`, `text::task_tags`, and reads
  `task_origin` only when both tags match. `engine_line` stays only for the A1 probe.
- 4.3h-Z: turn stamps cross as borrowed bytes (`begin(.., created: [u8])`, `answer(.., at: [u8])`); `stamp_from`
  returns the `time::Iso` (no `adapter_probe`, no second length check); `enum Stamp` removed.
- 4.3h-V: one byte check `time::canonical_at` (first mismatch returns, then `num` per field); `canonical_iso`, `dig`,
  `dig_at` removed; `format_iso` copies the template and writes with `put_digits`, the unreachable empty return is gone.
- 4.3h-L: `text::one_line` keeps a pending space and a UTF-16 width budget like the hand loop; `tail_info` /
  `read_window` call it only when a title was ranked (the hand `then` closure).
- 4.3h-A: `boundary::new_packet(nt, nx)` reserves `18 + nt + nx` and pushes the first 4 words; callers push the rest of
  the header in one `arr_push`, then `arr_append` the turns and the widened text. `head_result` / `tail_result` build a
  `[u8]` text first (`put` / `put_optional` order); `read_window` builds `text` (title, model, turns) and `turns` apart
  with `turns::put_turns` (replaces `write_turns`, oldest first). `import::put_str` and `boundary::write_bytes` are gone.
- 4.3h-P, part: `turns::plan` counts like `Collector::kept` and no longer stores planned ends in `sl[1600..]`
  (`plan_end`, `planned_end`, `planned_spans*` removed); `plan` takes `sl` read-only.

One Codex structure comparison (gpt-6.2-astra, high; the default account was at its limit, so the Codex 2 account),
read-only, of the diff against the hand. Applied: `tail_result` reserves the 24 stamp bytes only when a stamp is
present; the A3 fixture reserves its whole packet once. Not a packet bug: the A2 probe `canonical_w` returns `[-1]` for
non-UTF-8 bytes where the hand `canon` gives false (probe only, row 4.4c-SV). The other findings predate this step and
are kept as rows below.

Rows still open (claude):

| row | kind | finding | what the hand does |
|---|---|---|---|
| 4.3h-P (rest) | bench | `new_slots` fills 200 ordinals with -1 and `begin` writes one; every import contract carries the ordinal invariant and the A3 snapshot exports it | `VecDeque` order |
| 4.4c-SV | language | a non-literal `str` cannot be lent as `[u8]`, so `time::from_string` copies the 24 bytes (`bytes24`) before `canonical_at` and the A2 probe keeps the `adapter_probe` length check | `s.as_bytes()` |
| 4.4c-IS | language | `time::Iso` holds a heap `[u8]` (24 bytes per stamp, `copy(iso.text)` for `lastAt`, 4.3d-M); no fixed-size array value | `[u8; 24]` by value, `Option` |
| 4.4c-TS | bench (with VL) | `completedAt` lives after the reply in the arena; each later answer moves it 24 bytes (`copy_within`) | two fixed fields in `Draft` |
| 4.4c-DC | bench | `stamp_from` reports every non-absent timestamp defect as -4 (the packet only sees the failure) | `None` |
| 4.4c-TR | bench | `time::from_string` owns the trimmed `substr` before the digit check | borrowed `es_trim` |
| 4.4c-MB | bench | the model and title buffers are replaced by a new owned string (`move`) instead of reused | `clear` + `extend_from_slice` |
| 4.4c-OL | bench | `one_line` finds the next UTF-8 boundary, then reads the first byte again for the space test | one `chars()` step |

Also still open: 4.3h-O, 4.3h-D (hand candidate), 4.3h-Q, CK, D-R08-2 / VL, 4.3d-M.

Results after the last code change (rs pairs only; no perf timing):

| check | rs/rust | rs/llvm |
|---|---|---|
| root verify (300 s cap) | a0 6 s, a1 29 s, a3 11 s alone; a2, a4, q_root, root 30 s each with five roots at once; all VERIFIED | - |
| `test_a0.py` | 272/272 | 272/272 |
| `test_a1.py` | 2884/2884 | 2884/2884 |
| `test_a2.py` | 2443/2443 | 2443/2443 |
| `test_a3.py` | 134/134 | 134/134 |
| `test_a4.py` | 270/270 | 270/270 |
| `q.py --count 300` (seed 20261114) | 570/570, agrees with the oracle | 570/570, agrees with the oracle |
| `q_mutants.py` (rust backend by design) | 18 / 18 present once; 9 detected, 9 proof-rejected, 0 survived | - |

`q_mutants.py`: mutants 2, 11, 12, 14, 16 and 18 pointed at removed or rewritten code and were retargeted to the same
fault in the new code (2: veto block ignored; 11 / 18: early return before `complete`; 12: newest-first slot in
`put_turns`; 14: the window `flag`; 16: `tiled` reads one record too far). In `handrust` only the `// Pruv:` mapping
comments changed; `cargo build --release`, 0 warnings.


## 6. Step 4.4g: bench rows 4.3h-P (rest), 4.4c-TR, 4.4c-OL; MB and TS stay

Closed, in the hand shape (`handrust/src/lib.rs`):

- 4.3h-P (rest): the ordinal column is gone. `sl` is 1600 words (200 slots of 8), `new_slots` is `alloc(1600, 0)`
  with no fill loop, `begin` writes no ordinal, and the ordinal invariant is dropped from every `turns`, `import` and
  `a3_root` contract. The ordinal was read only by contracts and the A3 snapshot; the current slot always holds the
  newest begun draft (`begin` is the only writer of `st[0]` and `st[3]`), so snapshot word 11 is `st[0] - 1` (-1
  before any begin), and the A3 fixtures are unchanged (134/134).
- 4.4c-TR: `time::from_string` borrows the trimmed text (`text::trim_span` + `subview`) for the digit check; no
  `substr` before it (hand `es_trim`).
- 4.4c-OL: `text::char_step` is one `chars()` step: the lead byte decides ASCII (end `i + 1`, space test on that byte),
  otherwise `next` gives the width and the 2- or 3-byte space test reads the continuation bytes once; `one_line` calls
  it once per character and no longer calls `next` and then `space_len` on the same bytes.

Rows still open (claude), changed in this step:

| row | kind | finding | what the hand does |
|---|---|---|---|
| 4.4c-MB | pkgs | `pkgs/access` / `pkgs/jstr` only decode into a new owned `str` (`string_text_owned`); there is no decode into an existing buffer and no borrowed view of an escape-free string, so `model` / `best` keep `move value` (a `clear` + append would add a copy and keep the allocation) | `clear` + `extend_from_slice` from a borrowed `Cow` |
| 4.4c-TS | bench (with VL) | `completedAt` still lives after the reply in the arena (24-byte `copy_within` per later answer). Re-probe: a local `Vec<D>`, a struct field `Vec<D>` (`vec_push` with a length bound) and `vec_at_mut` on an `inout Vec<D>` where `D` has `[u8]` fields now compile on both backends, so VL looks closed on this compiler (SPEC.md line 393 still says LLVM rejects it; runtime output of the probe not compared). The `Vec<Draft>` rewrite of `turns` (D-R08-2) is a whole-module change and was not started; there is no `VecDeque` / `pop_front` | `VecDeque<Draft>`, two fixed stamp fields |
| 4.4g-DV | bench | `digit_value` checks, accumulates and bounds the digits in one loop with early returns; the hand checks all digits, strips leading zeros, rejects more than 16, then parses | `all(is_ascii_digit)`, `trim_start_matches('0')`, `parse` |
| 4.4g-MC | bench (with MB) | `read_window` compares the new model token with the held model (`string_is`) and decodes only when it differs; the hand clears and copies every nonempty nonsynthetic model | `clear` + `extend_from_slice` each time |

Unchanged: 4.4c-SV, 4.4c-IS, 4.4c-DC, 4.3h-O, 4.3h-D, 4.3h-Q, CK, D-R08-2, 4.3d-M (4.3h-O and 4.3h-Q need `pkgs`
or a shared module; out of this step's scope).

Results after the last code change (rs pairs only; no perf timing; each test verifies its root before running):

| check | rs/rust | rs/llvm |
|---|---|---|
| `test_a0.py` | 272/272 | 272/272 |
| `test_a1.py` | 2884/2884 | 2884/2884 |
| `test_a2.py` | 2443/2443 | 2443/2443 |
| `test_a3.py` | 134/134 | 134/134 |
| `test_a4.py` | 270/270 | 270/270 |
| `q.py --count 300` (seed 20261114) | 570/570, agrees with the oracle | 570/570, agrees with the oracle |
| `q_mutants.py` (rust backend by design; run as `--ids` 1-10 and 11-18) | 18 / 18 present once; 9 detected, 9 proof-rejected, 0 survived | - |

One Codex structure comparison (gpt-6.2-astra; the default account was at its limit, so the Codex 2 account; the first
attempt timed out at thread start), read-only. It confirmed that `st[0] - 1` is the ordinal of the current slot for
every reachable state, the borrowed trim in `from_string`, and the `one_line` rules. Not applied, kept as rows:
4.4g-DV and 4.4g-MC (new), `one_line` walking `i64` byte positions instead of `char` values (no `char` iterator in
the language; same reads and output buffer), the `bytes24` copy before `canonical_at` (4.4c-SV, 4.4c-IS), and the
slot ring / `reclaim` against the hand's per-draft `Vec` reuse (D-R08-2, 4.4c-TS).

In `handrust` only the `one_line` `// Pruv:` comment changed; `cargo build --release`, 0 warnings.

## 7. Step 4.4h: D-R08-2 / 4.4c-TS, `turns` as `Vec<Draft>`

`turns.pruv` no longer has the `st` / `sl` / `buf` word ring and byte arena. The collector is now the hand's
`Collector` / `Draft` pair. `import.pruv` and `a3_root.pruv` take one `inout col: turns::Collector`. `reclaim`,
`reserve`, `write_begin`, `write_answer`, `complete`, `reply_room`, `spans`, `fields` and the `limit` argument are
gone.

Structure next to `handrust/src/lib.rs` (`Draft` 635-646, `Collector` 647-755, `window_packet` 926-993):

| item | hand | Pruv now | before 4.4h |
|---|---|---|---|
| draft type | `struct Draft { prompt: Vec<u8>, reply: Vec<u8>, used, tools, created: Stamp, completed: Stamp, clipped: bool }` | `pub struct Draft { prompt: [u8], reply: [u8], used, tools, created: [u8], completed: [u8], clipped: bool }`; stamps are 0 or 24 bytes (4.4c-IS) | 8 words per slot in `sl[1600]` plus spans in `buf` |
| collector | `Collector { drafts: VecDeque<Draft>, begun, active, lost }` | `Collector { drafts: Vec<Draft>, begun, active, lost }`; the oldest draft is at index `begun % 200` once 200 exist, the current one at `(begun - 1) % 200` | `st[5]` + `sl` + `buf` |
| invariant | none (types) | one `pub pred ok(c)`: `len(drafts) == min(begun, 200)`, `active ==> begun >= 1`, per draft prompt <= 32768, reply <= used <= 32768, tools <= 1e15, stamps 0/24 | `spans`, `fields`, `reply_room` and `st[4] <= 32792 * pos` in every loop |
| begin, < 200 drafts | `Draft { prompt: Vec::new(), .. }`, `extend_from_slice(&p[..n])`, `push_back` | fresh `[u8]` + `arr_append`, `vec_push::<Draft>` (moves the draft) | arena append |
| begin, 200 drafts | `pop_front`, `clear` prompt and reply, `push_back` (blocks reused) | `vec_at_mut` on the front index, each field moved out, `truncate(x, 0)` + `arr_append`, moved back (`refill`, `refill_text`) | arena append, `reclaim` when evicted spans pass half |
| answer | `back_mut`, `completed = Some(at)`, `reply.extend(sep)`, `reply.extend(&t[..take])` | `vec_at_mut` on the current index; `completed` refilled in place (24 bytes, no move of other data); `reply` moved out, `piece` appends sep and take, moved back | completed stamp after the reply in the arena, 24-byte `copy_within` per later answer |
| tool | `back_mut().tools += 1` | `vec_at_mut`, `d.tools = d.tools + 1` | `sl[8 * c + 3]` |
| plan / put | `iter().rev()` / `iter().skip(len - n)` with `put` | index loop over `(begun - 1 - j) % 200`, `vec_get::<Draft>(...).field` | same loops over `sl` and `buf` offsets |
| allocation sites | one `Vec` per draft field, reused after eviction | same: per field, reused after eviction; no arena | one arena grown by doubling, one 1600-word table |

Language gaps met (no compiler change; closest shape kept):

| row | kind | finding | closest shape used |
|---|---|---|---|
| 4.4h-VD | language | no `VecDeque` / `pop_front` / `push_back` on `Vec<S>` | `Vec<Draft>` with a front index `begun % 200`, reuse in place |
| 4.4h-FA | language | a `[u8]` field cannot be appended in place (`arr_append` needs a local `inout`); R-6e-16 move-out / restore with a `take` helper | `let r: [u8] = move d.reply; d.reply = piece(move r, ...)` |
| 4.4h-TR | language | `truncate` takes no `move` argument; the clear goes through a local (`let x0 = move b; truncate(x0, 0)`). On LLVM a truncate that leaves more than half unused may shrink the block (LG33), so the reuse of evicted blocks is only certain on the Rust backend (not measured per allocation) | `refill` / `refill_text` |

Closed: D-R08-2 and 4.4c-TS (the arena, `reclaim` and the `completedAt` move are gone), and 4.4h-PC: the old
per-field ensures are restated over `c.drafts[(c.begun - 1) % 200]` (`K` below). `begin`: `len(K.prompt)` is the
clip length (`<= len(prompt)`, equal when `len(prompt) <= 32768`, a char boundary, and no later boundary up to
32768), `K.clipped == (len(prompt) > 32768 || old(c).lost)`, reply/used/tools/completed reset, `len(K.created) ==
len(created)`. `answer`: `len(at) == 24 ==> len(K.completed) == 24`, and the `+ 2 + len(text)` reply growth when the
piece fits. `tool`: the current draft's tools `+ 1` below the cap, `len(drafts)` kept. `plan`: `total` is the sum of
prompt + reply over the kept drafts (loop invariant), and one more draft would pass 2097152. No language gap was
met for these; `old(c).drafts[i].field` and `sum` over `c.drafts[...]` both verify.

Results after the last code change (rs pairs; each test verifies its root before running; the default `q.py` run
also lists `py/*` pairs, which fail with `build: 'py'` in this harness and are not part of the check):

| check | rs/rust | rs/llvm |
|---|---|---|
| `test_a0.py` | 272/272 | 272/272 |
| `test_a1.py` | 2884/2884 | 2884/2884 |
| `test_a2.py` | 2443/2443 | 2443/2443 |
| `test_a3.py` | 134/134 | 134/134 |
| `test_a4.py` | 270/270 | 270/270 |
| `q.py` (default count, seed 20261114) | 20270/20270 | 20270/20270 |
| `q_mutants.py` (rust backend; ids 1-7, then `--ids` 8-18) | 18 / 18 present once; 9 detected, 9 proof-rejected (7, 8, 9, 11, 13, 15, 16, 17, 18), 0 survived | - |

`q_mutants.py`: mutants 6, 10, 11, 14, 17 and 18 pointed at removed code and were retargeted to the same fault in the
new code (11 and 18 now return before the `vec_at_mut` of `answer`; 17 writes the tools of index `begun % 200`).
`q_measure.py`: the table print failed on a cell whose hand time is 0 (`ratio` None); it now prints `—` there.

`q_measure.py --pairs rs/rust,rs/llvm` (7 rounds, load average 7-11; `ring_wrap` and `depth_limit` not reached
inside the time cap). Time ratio / RSS ratio, Pruv over hand:

| input (class) | op | rs/rust | rs/llvm |
|---|---|---|---|
| turns16 (M) | window | 1.086 / 1.047 | 0.936 / 1.028 |
| turns16 (M) | head | 0.768 / 1.020 | 0.635 / 1.020 |
| turns16 (M) | tail | 0.821 / 1.029 | 0.790 / 1.020 |
| turns17 (M) | window | 1.057 / 1.028 | 0.913 / 1.018 |
| turns17 (M) | head | 0.644 / 1.020 | 0.733 / 1.020 |
| turns17 (M) | tail | 0.943 / 1.029 | 0.940 / 1.020 |
| turns200 (L) | window | 1.099 / 1.023 | 0.939 / 1.000 |
| turns200 (L) | head | 0.797 / 1.028 | 0.723 / 1.019 |
| turns200 (L) | tail | 0.914 / 1.038 | 0.806 / 1.019 |
| window_8MiB (L) | window | 1.228 / 1.008 | 0.985 / 1.005 |
| long_text (L) | window | 2.106 / 1.093 | 1.834 / 1.094 |
| block_arrays (L) | window | 0.898 / 0.768 | 0.703 / 0.756 |
| window_8MiB, long_text, block_arrays (L) | head, tail | 0.98-1.02 / 0.998-1.000 (sub-microsecond calls) | same |

Performance debt (cells above hand 1.0x): rs/rust window time on turns16/17/200 and window_8MiB (1.06-1.23x),
long_text window time on both backends (1.83-2.11x, G-DB60-2: the long_text bytes are not in `turns`), RSS 1.02-1.09x
on M/L window/head/tail (D-R08-5 / D-DB1-6: the fixed ring is gone, so the remaining part is elsewhere; not profiled
in this step). No before/after pair was run under the same load, so these are not deltas against 4.4g.

In `handrust` only the `// Pruv:` comments of `Draft`, `Collector`, `begin` and `answer` changed; `cargo build
--release`, 0 warnings.

Different-model structure comparison (Codex gpt-6-astra, research only, `turns` / `import` / `a3_root` against
`handrust/src/lib.rs`, differences listed per function without verdicts). The `turns` differences not already in
the table above are added here. No code was changed for them: each one is a language gap, a proof guard, or a
fixture entry point.

| row | kind | function | difference (Pruv / hand) |
|---|---|---|---|
| 4.4h-ST | language (4.4c-IS) | `begin`, `answer` | stamps are `[u8]` fields, so a fresh draft allocates 4 empty arrays (hand 2 `Vec` + 2 `None`); reuse refills `created` / `completed` through `refill` (hand assigns an `Option<[u8; 24]>` value) |
| 4.4h-CT | proof | `tool` | `tools < 1e15` saturating check before `+ 1` (i64 bound for `ok(c)`); hand `usize += 1` with no cap |
| 4.4h-AE | shape (minor) | `answer`, `piece` | empty text / full budget skip the rest through an `if` around the body, not an early `return`; `piece` always calls `arr_append` twice (the separator call may append 0 bytes), hand extends once in the `room <= sep` branch |
| 4.4h-PL | shape | `plan` | returns `Plan { kept, total, bytes }` (`bytes` adds the stamp lengths and feeds `arr_reserve(text, base + plan.bytes)` in `import.pruv`); hand `Collector::kept` returns only the count, and `window_packet` grows `text` without a reserve |
| 4.4h-PT | shape | `put_turns` | one flat loop with the two stamps written inline and the 10 record words in one `arr_push`, plus a `shift` argument (A3 fixture prefix); hand iterates the kept suffix with an inner loop over the two stamps and `put` / `push` per word |
| 4.4h-CU | fixture | `current` | a Pruv helper used only by `a3_root.pruv` (the A3 step driver); hand reaches the current draft with `back_mut()` |
| - | language (i64) | all | `begun`, `used`, lengths are `i64` (hand `usize`); the existing integer-width rows apply |

## 8. Step 4.4i: 4.4c-MB (model) with 4.4g-MC, 4.3h-Q; 4.3h-O and the title buffer stay

New package API (`pkgs/access` 337, `pkgs/jstr` 223): `access::string_append(doc, id, inout out: [u8]) -> i64` appends
the string token `id` to the caller's buffer. A plain token (tape word 5 == 1, no backslash) is one `arr_append` of
the source span; an escaped one goes through `jstr::decode_append`, the old `decode_utf8` body writing into `out`
after `arr_reserve(out, base + need)`. `decode_utf8` is now `alloc(0, 0)` + `decode_append`, so `string_text_owned`
and every other caller keep their code and contracts. Contract: `result <= 0`; on 0, `len(out) <= old(len(out)) + 3
* span`; on a negative code `out` is unchanged.

Structure next to `handrust/src/lib.rs` (`consume` 795-835, `window_packet` 927-993):

| item | hand | Pruv now | before 4.4i |
|---|---|---|---|
| `model` buffer | `st.model: Vec<u8>`, `clear()` + `extend_from_slice(m.as_bytes())` from a borrowed `Cow` each time | `model: [u8]` local of `read_window`, `model = truncate(model, 0)` (R-6g4-3 in place: Rust `Vec::truncate`, block kept) + `string_append(d, model_id, inout model)`: one copy of a plain span into the reused block, decode in place when escaped | `str`, `string_is` against the held model, then `string_text_owned` + `model = move v` (a new block per changed model) |
| model filter | `consume`: `!m.is_empty() && m != SYNTHETIC` | `consume_entry`: `string_is(doc, model_id, "<synthetic>")`, then `string_is(doc, model_id, "")` (both compare in place, no decode) | synthetic only; the empty check came after a decode in `read_window` (`string_is` vs held) |
| model compare (4.4g-MC) | none: copies every kept model | none: copies every kept model | compared with the held model first, decoded only when different |
| framing | one module (`LINE_BYTES`, `DROP_HEAD_BYTES` consts next to it) | `pkgs/framing` (one file, mounted by claude a0/a2/a4/q/root and qwen q_root/root); `limit_line_bytes` / `limit_drop_head` moved there from both `boundary.pruv` | two byte-identical `framing.pruv`, limits read through `crate::boundary` |
| allocation sites (model) | one `Vec` grown once, reused | one `[u8]` grown by `arr_reserve` / `arr_append`, reused (in-place `truncate` keeps the block on both backends) | one block per changed model |
| `turns::refill` / `refill_text` (4.4h) | `clear()` on the evicted draft's blocks | `let mut x = move b; x = truncate(x, 0)`: in place, block kept on both backends | `let x0 = move b; let x = truncate(x0, 0)`: the copy form, which on rs/rust emits `truncate` + `shrink_to_fit` when `capacity / 2 > len` (always at length 0), so every evicted prompt/reply block was freed |

Not changed (the language cannot state the hand shape here without a run-time check; no workaround):

| row | kind | finding | kept |
|---|---|---|---|
| 4.4i-UF | proof (pkgs/parse, pkgs/lex) | the tape carries no proved UTF-8 fact for a string token span: `lex::validate_utf8` and `parse::string_token` check it at run time but export only `result == -1 \|\| (start <= result && result < end)`. A borrowed `ref str` of a plain span, or a `str` view of a reused `[u8]` buffer, needs that fact (R-6g-10) or a second run-time `utf8_valid` | `entry::string_field_of` / `string_content`, `import::answer_blocks` / the `consume_entry` text answer, and the title buffer (`best` in `tail_info`, passed to `text::one_line(str)`) keep `string_text_owned` + `move` |

So 4.3h-O stays open (now as 4.4i-UF) and 4.4c-MB stays open for the title buffer only. Closing them needs a
`pkgs/parse` / `pkgs/lex` contract change (an exported per-token UTF-8 predicate) that every parse consumer (json,
mcp, supervisor, qwen, codex, opencode, grok, claude) would retest; that is out of this bench-only step.

Closed: 4.4c-MB for `model`, 4.4g-MC, 4.3h-Q, the `joined` copy of `array_content` (below), and 4.4h-TR: §7 read the `let x0 = move b; truncate(x0, 0)` form
as keeping the block on rs/rust; the generated Rust shows it shrinks there too. A `move` into the destination local
first and then `x = truncate(x, 0)` is the R-6g4-3 in-place form on both backends, so no language gap remains.


Different-model structure comparison (Codex gpt-6.2-astra, research only, given the generated `a4_root_gen.rs` and
`handrust/src/lib.rs` without these conclusions; it read the file generated before the two in-place `truncate`
changes). Every pair was rated DIFFERENT; each difference is placed below.

| finding | placed |
|---|---|
| model buffer: moved, `truncate(0)` then `shrink_to_fit` (hand `clear`) | fixed in this step (in-place `truncate`, the table above); the regenerated code has `model.truncate(0)` only |
| `array_content`: the joined text is copied once more (`string_to_str(joined)` -> `clone()`); hand moves `joined` | fixed: `whole = string_into_str(move joined)` (R-6e-23) in `entry.pruv` |
| title candidates, user single string, user text blocks, assistant `content` / text blocks, string timestamps decoded into owned `String`s before the check; hand borrows `&str` / `Cow::Borrowed` | 4.4i-UF (and 4.3h-O); timestamps also 4.4c-IS |
| whole line copied into `JsonDocument.source` (`to_vec`) where the hand parses the borrowed line | existing `pkgs/lines` / `pkgs/json` shape (the tape document owns its source); not a 4.4i row, left as reported |
| `decode_append` counts the output first, `reserve_exact(base + need)`, then a second pass; hand decodes once during the parse and borrows when there is no escape | new 4.4i-DP (pkgs/jstr, shape): the escaped path keeps the count-then-write form of `decode_utf8`, which gives the `3 * span` growth bound in one reserve; plain tokens take one `arr_append` |
| `consume_entry` returns `Effect::Done(model_id)` and the caller updates the model; hand `consume` updates title and model itself | existing shape (the model buffer lives in `read_window`, `inout` of a struct-held `[u8]` is 4.4h-FA) |
| framing returns `i64` positions / `Frame` indices, hand borrows `&[u8]` slices; limits are `i64` functions, hand `usize` consts | existing i64-position and integer-width rows; no copies on either side |

Results after the last code change (rs pairs; each test verifies its root first):

| check | rs/rust | rs/llvm |
|---|---|---|
| claude `test_a0` / `a1` / `a2` / `a3` / `a4` | 272 / 2884 / 2443 / 134 / 270, all pass | same |
| claude `q.py` (default count, seed 20261114) | 20270/20270 | 20270/20270 |
| claude `q_mutants.py` (all 18 in one run) | 9 detected, 9 proof-rejected, 0 survived | - |
| qwen `test_impl.py` | 26/26 | 26/26 |
| mcp `test_impl.py` | 22/22 | 22/22 |
| codex `test_impl.py` | 136/136 | 136/136 |
| opencode `test_impl.py` | 199/199 | 199/199 |
| grok `test_impl.py` | 254/254 | 254/254 |
| json `run_a0`-`run_a5` | 37 / 351 / 114 / 454 / 6344 / 83, all as expected | same |
| supervisor `test_a0`-`test_a3` | 522 / 410 / 78 / 35 | same |

`q_mutants.py`: mutants 7 and 8 now edit `work/qm/<id>/pkg_framing/framing.pruv`, whose `pruv.json` dependency
paths are rewritten to the shared `pkgs`. Not measured: 4.4i-UF keeps the title and read-only paths on owned
decodes, so the structure is not yet the hand's for every function on the window path.

After the review fix in `entry.pruv` (`string_into_str(move joined)`), claude `test_a0`-`test_a4`, `q.py` and
`q_mutants.py` were run again on both rs pairs: same counts as above, all pass (q_mutants 9 detected, 9
proof-rejected). The fix touches only `entry.pruv`, so the other pieces were not rerun.

Step 4.4m (4.3h-O, consumer side): `import::answer_blocks` and `consume_entry` try `access::plain_str` (a borrowed
`str` of a plain string, under `doc_strings`, which `entry::parse_line` now ensures) before the owned decode. It
does not take effect yet: `parse::record` never sets the plain bit, so the borrow is always `None` (row 4.4m-PB).
Title and user content rows: 4.4m-TB, 4.4m-FB. See `pkgs/RESHAPE.md` §Step 4.4m.
