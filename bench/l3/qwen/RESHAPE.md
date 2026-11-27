# Qwen head, tail and window import: Rust-shape reset (RUST_SHAPE_RESET §6 steps 4.3d, 4.3e)

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

| step | Pruv before (4.3c tree) | Pruv now (4.3e) | hand Rust | copies / allocations (now vs hand) |
|---|---|---|---|---|
| head lines | `edge_next` (`lines::jsonl_next`, final line only when not cut) | same | `data.split(b'\n')`, `pop()` when cut, skip empty | 0 / 0; same |
| head fields | `string_field(cwd)` then `copy(v)`; `human_text` then `copy(t)`; `idle` flag | `move v`, `move t`; no flag | `entry.get("cwd")` clone; `human_text` → `Option<String>` | 1 per field (decode) vs 1 (clone); same |
| head result | zero-filled `alloc(total)`, three `put_str` byte loops (one always empty) | `arr_reserve(total)` + two `arr_append` | `Head { cwd, first_prompt }` | 1 alloc vs 0 (host wire, row ABI) |
| tail lines | `skip_first` byte loop, then `edge_next` | `skip_first` = `find_byte`, then `edge_next` | `split(b'\n')`, `next()` when cut | 0; same |
| tail fields | `iso_of` then `copy(iso.text)`; `title_of` then `copy(text)` | `copy(iso.text)` (row 4.3d-M); `move text` | `iso(timestamp)` owned; `one_line(text)` owned | +1 copy of the 24-byte stamp (4.3d-M) |
| tail title | ranked best kept raw, `one_line` once at the end | last custom title wins (`has_title`), `one_line` once | `one_line` per custom title line | Pruv fewer (difference T1 below) |
| window lines | 16-byte min-distance pre-pass + byte loop | one `find_byte` | `split_inclusive(b'\n')` | 0; same |
| window entries | `Vec`-free event log (`Events`: 9-byte records + text buffer), sized by an LF pre-count | `Events { records: [i64] (3 words), text }` grown per object; role, title and stamp decided in the scan (`role_of`, `custom_title`) | `Vec<Slot>` of `Entry { uuid, parent, action }` | row VL |
| uuid index | arena + splitmix hash buckets sized by an LF pre-count; `put` byte loop; `same_span` byte loop | arena / slots grown per object (`arr_append` / `arr_push`); buckets linked in `resolve` after the scan; `ul == klen && eq_range` | `HashMap<&str, &Entry>`, `HashSet<&str>` | arena 1 alloc vs map; row 4.3d-H |
| chain walk | `resolve` / `lookup` | same | `current_chain` | same steps |
| replay | else-if chain on `event.kind` | `match event.kind` (outer and inner) | `match &entry.action` | same |
| collector | `pkgs/turns` st/sl/bk flat arrays | same | `Collector { VecDeque<Turn> .. }` | row VL |
| window result | wire header by `put_word` byte loops, title/model `put_str` loops, outputs copied | `store_le` / `load_le` words; `copy_range` for title/model; `publish` / `make_output` take and move | `Window { .. }` | host wire, row ABI |
| title bytes (root) | zero `alloc(len)` + byte loop | `arr_reserve` + `arr_append` | `to_vec()` | 1; same |

## 2. Differences that remain

- VL (pkgs, open): the hand's `Vec<Slot>` and `VecDeque<Turn>` hold structs with owned strings; Pruv keeps the event
  log (`import::Events`, records + text) and the `pkgs/turns` flat store.
- 4.3d-H (language since 4.3e): the hand indexes uuids with `HashMap`/`HashSet` of borrowed `&str`. Pruv maps
  (R-6e-5) take owned `String` keys only; a `[u8]` or borrowed-slice key is rejected, so the hand's
  `HashMap<&str, &Entry>` cannot be written. `chain` keeps its arena + bucket index (DB6), now grown per object and
  linked after the scan like the hand's `latest` map. Closest form; no workaround.
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

### 4.3e: bench rows C, P, W, O, T and a second Codex review

Done in 4.3e (`qwen/` only):

- 4.3d-C: `chain::capacity` removed. `Index` starts empty; `note` appends uuid / parent to the arena (`put_field`) and
  pushes a 6-word slot per object; `resolve` sizes the power-of-two heads from the slot count and links buckets after
  the scan. `selected` is `alloc(ordinal)` after the scan; `Events` starts empty and grows.
- 4.3d-P: `entry::parse_line` skips leading whitespace and returns `Skip` unless the next byte is `{`, before parsing.
- 4.3d-W: `boundary::put_word` / `get_word` are one `store_le` / `load_le` each.
- 4.3d-O: `boundary::make_output` and `import::publish` take meta, turns and text and move them.
- 4.3d-T: `entry::role_of` reads `type` once (an else-if chain over the hand's `match`), with `user_role` reading
  `provenance` once. `window_wire` takes the title from system lines in the scan (`entry::custom_title`); stamps are
  written for real user lines with text and assistant lines only. Removed: `replaces`, `last_max`, `first_present`,
  `pick_title`, `ranked`, event tags 2 and 3. `tail_info` keeps the last custom title. q mutants retargeted:
  `last-title` (`best = move text; has_title = true;`), `human-role`, `provenance`; all 12 match exactly once.
- `Events` dropped its struct invariant (it is checked at every `inout` exit and did not hold mid-loop);
  `replay` requires `len(log.records) <= 4294967295` and `window_wire` checks it once before `replay`.

Second Codex review (gpt-6.2-astra, high), hand vs every touched function. Applied:

- `human_text` stops at a non-`user` type before provenance (`edge.rs` `human_text`), sharing `user_role`.
- `window_wire` counts only parsed objects as entries (`ordinal` grows in the `Object` arm), like the hand's
  `Vec<Slot>`; blank and non-object lines no longer widen `selected`.
- `resolve` returns an inactive chain before allocating buckets when no entry can be the leaf (hand `current_chain`).

Open after 4.3e (bench, `qwen/` unless noted):

- 4.3e-D: `record_entry` joins the user prompt, writes the stamp and walks assistant parts before the chain is chosen
  and before turn linkage is known; `replay` reads answer text even when no turn is linked. The hand updates the
  model after selection and handles stamp and parts only for a linked turn (`lib.rs` 270-297). Needs the log to keep
  raw ranges and decode in `replay`.
- 4.3e-L: the leaf candidate is kept in `Index.leaf` / `role_seen` during the scan (`note`); the hand picks it in
  `current_chain`. `note` skips the parent when the uuid is empty, and `window_wire` decides the role before the
  uuid / parent (hand order: uuid, parent, role).
- 4.3e-F: `chain::put_field` decodes into an owned `str` and then appends it to the arena; the hand moves the
  parsed string. Needs a decode-into-buffer form in `lib/access` (shared; run all consumers).
- 4.3e-J: `entry::array_content` decodes each text part owned and appends; the hand collects `&str` and `join`s.
  `title_key` owns a blank title before the blank check.
- 4.3e-S: `write_stamp` turns the 24-byte `iso.text` into a `str` before the log copy (with 4.3d-M).
- Behaviour: the escaped `$serde_json::private::Number` timestamp key (hand artefact, above) also shows in
  `tail_info` `last_at`; not new in 4.3e.

## 3b. Step 4.4d: rows 4.3e-S, 4.3e-L, part of 4.3e-D

- 4.3e-S closed. `write_stamp` appends the 24 bytes of `time::iso_of` straight to the log text; `stamp_of`, the
  `Stamp` enum and its `adapter_probe` copy are gone.
- 4.3e-L closed. The leaf is chosen in `chain::resolve`, in the pass that links the buckets (the hand `latest` loop):
  `last_role.or(last_entry)` over the entries with a uuid. A slot is now 7 words (the 7th is the role bit); `Index`
  lost `leaf` / `role_seen`. The leaf's `parentUuid` defect is checked when the leaf is read, not in `note`. `note`
  still takes the role from `window_wire`, which computes it before the uuid (argument order only: both defects end
  in the same internal-invariant packet). `note` still skips `parentUuid` when the uuid is empty (the hand removes it
  from the map without decoding; nothing reads it).
- 4.3e-D partly closed. `replay` reads answer text (tag 7) and counts tools (tag 8) only for an attached turn
  (`st[1] == 1`), as the hand `if collector.attached`; the model was already updated only for a selected entry. Still
  open: `record_entry` decodes the prompt, stamps and answer parts of every entry during the scan, before the chain
  is known (the hand keeps the parsed objects and reads them only for selected entries). Needs the log to keep raw
  string ranges of `data` and decode in `replay`, or a parsed-document vector (gap VL).
- 4.3e-J and 4.3e-F stay open, one cause: there is no decode-into-buffer form of a JSON string
  (`pkgs/jstr` / `pkgs/access`), so `array_content` and `put_field` decode an owned `str` and then append it, and
  `title_key` owns a title before the blank check. Both are shared packages (every consumer reruns), not done in
  4.4d.

Codex structure comparison (one, gpt-6.2-astra high, Codex 2 account): the leaf choice, the leaf parent check and the
answer / tool gating match the hand. New rows (not changed in 4.4d):

| row | Pruv | hand | change |
|---|---|---|---|
| 4.4d-QS | an assistant stamp (tag 4) is read into a `str` even when no turn is attached | `iso(timestamp)` only `if collector.attached` | part of 4.3e-D (decode in `replay`) |
| 4.4d-QA | each answer is read from the log into an owned `str` (`read_text` / `adapter_probe`) before `answer2` | `collector.answer(text, ..)` borrows the part `&str` | lend the log span as `str` (gap 4.4c-SV: no `[u8]` to `str` view) |
| 4.4d-QC | `lookup` marks every slot of a uuid and fails on a `-2` parent of any duplicate | `latest` keeps the newest entry per uuid; only the followed entry's parent is read | split marking from the parent read; owned keys vs borrowed map keys stay gap VL |

Next (order kept in `../lib/RESHAPE.md` "Rows open after 4.4d"): Codex rows 4.3f-A, O, N, K, L, T, P, R, D, then
4.3f-B, E, F; then the OpenCode leftovers, 4.4b-SP first (checked against the TypeScript reference).

## 3c. Step 4.4u: rows 4.4d-QC, 4.3e-F, 4.3e-J

- 4.4d-QC closed. `chain::resolve` walks like the hand `current_chain`: the leaf is `last_role.or(last_entry)`, then
  `while selected.insert(uuid)` is `select` (marks every slot with this uuid, `changed` when one was new), the parent
  is read from the reached slot only, and `latest.get(parent)` is `newest` (first match in the newest-first bucket).
  `lookup`, `Hit` and `Leaf` are gone. A `-2` parent on an unfollowed duplicate no longer fails. Fixture
  `duplicate-bad-parent`: two `u` entries, the older with parent `"\ud800"`, plus a `system` entry `a` with a
  `"\udc00"` parent. Both decoders turn a lone surrogate into U+FFFD (`jstr.pruv`), so `-2` is not reachable from
  input; the fixture is a guard (the old code passed it too). Mutants `chain-filter` and `leaf-parent` were rewritten
  for the new text.
- 4.3e-F closed. `put_field` decodes with `access::string_append` straight onto the arena and truncates back on the
  8 MiB cap; `note` takes the lengths from the arena. No `pkgs/` change.
- 4.3e-J closed. `Block::Piece` carries the text node; `array_content` pushes the `plain_str` view (owned decode only
  for an escaped node), as the hand collects `&str` parts and `join`s. `title_key` checks blankness on the view and
  copies only a nonblank title (hand `as_str().filter(..)`, then `one_line`). `parse_line` now ensures `doc_strings`;
  `user_text`, `human_text`, `title_key`, `custom_title`, `title_of`, `record_entry`, `record_object` require it.
  The solver gave up on it inside the `tail_info` and `window_wire` loops, so `window_wire`'s custom title and
  `record_object` moved into `import::window_object` (returns the `Title`; `Defect` when the line fails), and
  `tail_info` reads the title before the timestamp (new row 4.4u-TO; same output, both defects fail the line).
- Still open: 4.3e-D / 4.4d-QS (scan-time decode of prompt, stamp and parts; needs raw ranges in the log or gap VL),
  4.4d-QA.

Structure, function by function against `handrust/src/lib.rs` `current_chain` (128-) and `edge.rs` `human_text` /
`tail`: leaf choice, insert-then-follow loop, cut on a missing parent, parts join over borrowed text and the blank
title filter on the borrowed value match. Differences left: 4.4u-TO, the per-entry log (4.3e-D), owned arena keys
vs borrowed map keys (gap VL).

## 3d. Step 4.4w: rows 4.4d-QA, 4.4u-TO, 4.3e-D / 4.4d-QS

- 4.4d-QA closed. `replay` lends the prompt (tag 5) and answer (tag 7) spans of `log.text` to `begin2` / `answer2`
  as `str` views (R-6g-10, `subview` of a `[u8]` as a `str` argument), and the stamp (tag 4) stays a span
  (`at_lo`, `at_n` with `at_n` 0 or 24) lent the same way; `read_text` (owned copy and UTF-8 pass in
  `adapter_probe`) is left only for the model (tag 6), which the hand also owns (`model = Some(id.clone())`).
  The proof: `write_event`, `write_text`, `write_stamp`, `record_answers`, `record_entry`, `record_object`,
  `window_object` and the `window_wire` loop keep `utf8_valid(log.text)` (appending a `str`, or the 24 ASCII bytes of
  `iso_of`, keeps a valid buffer valid), and `replay` requires it. Gap 4.4c-SV is the other direction (`str` lent
  as `[u8]`) and stays open; it does not block this row.
- New row 4.4w-CE: `replay` tests the byte at each end of a span (`char_edge`: the end, `< 128` or `>= 192`) so the
  view is on char boundaries; the hand has no test (its `&str` parts come from serde). It replaces the full UTF-8 pass
  and copy of `adapter_probe`. Closing it needs a per-record fact over `log.records` (every text record is a span
  between two appends), a quantified invariant through the whole scan.
- 4.4u-TO closed. `tail_info`'s per-object body moved into `import::tail_object` (requires `doc_strings`, returns the
  `Title`), which reads the timestamp and then the custom title as the hand `tail_import` does. Kept inline, the
  solver still gave up on `title_of`'s `doc_strings` precondition after `iso_of` (`verify_unknown`).
- 4.3e-D / 4.4d-QS still open, the function is unchanged. The hand keeps each parsed object and runs `join`, `iso`
  and the parts walk only for selected entries (`iso` and parts only `if collector.attached`). Both Pruv forms are
  blocked:
  - a vector of parsed documents: `Vec<json::JsonDocument>` is `type/foreign_owned_type` (the element holds the
    `[i64]` tape), new gap row 4.4w-VD (and a per-element `doc_strings` fact would be needed);
  - raw ranges of `data` decoded in `replay`: `time::iso_of` needs a document (no range form in `pkgs/time`, new
    row 4.4w-TR) and `jstr::decode_append` ensures no UTF-8 fact on its output, so the decoded prompt or part could
    not be lent as `str` without a run-time UTF-8 pass the hand does not make (new row 4.4w-DU). Both are package
    changes, outside this bundle; a re-parse in `replay` would be a different shape, not done.
  The stamp no longer becomes an owned `str` in `replay` (now a span), but it is still formatted during the scan.

Structure, function by function (`handrust/src/edge.rs` `tail_import`, `lib.rs` `window_import` replay / `Collector`):

| function | hand | Pruv after 4.4w | difference |
|---|---|---|---|
| `tail_info` + `tail_object` | per line: `parse_line`, `iso(timestamp)` then system/custom_title; `last_at = Some(at)`, `title = Some(one_line(..))` | same order; `last = copy(iso.text)`, title kept as `str`, `one_line` once at the end | `copy` of the 24 iso bytes (hand moves the new `String`); `one_line` after the loop (hand per title) as before |
| `replay` tag 4 | `at = iso(timestamp)` (owned `Option<String>`) only when attached | span `(at_lo, 24)` of `log.text`, no copy | iso formatted at scan time (4.4d-QS); two edge byte tests (4.4w-CE) |
| `replay` tag 5 / 7 | `begin(prompt, iso)` moves the joined `String`; `answer(text: &str, at)` borrows | `begin2` / `answer2` get `str` views of `log.text`; both copy into the turn store | prompt / parts were joined and copied into `log.text` during the scan (4.3e-D); hand `begin` moves its `String`, `begin2` copies the view (turn store layout) |
| `replay` tag 6 | `model = Some(id.clone())` | `read_text` owned copy | same ownership; `adapter_probe` adds a UTF-8 pass |
| `write_*`, `record_*` | none (serde values in `Entry`) | append to `log.text` with `utf8_valid` kept | the per-entry log itself (4.3e-D) |

## 3e. Step 4.4x: package rows 4.4w-TR, 4.4w-DU; 4.3e-D / 4.4d-QS still open

- `pkgs/time` has `iso_raw(b, lo, hi)` (hand `iso(&Value)` over raw token bytes) and `jstr::decode_append` in the
  non-scanned mode ensures `utf8_valid(out)` when `out` was valid (see `lib/RESHAPE.md`, Step 4.4x). The Qwen
  sources are unchanged; this step reruns the Qwen tests against the new packages.
- 4.3e-D / 4.4d-QS are no longer blocked by a package row but were not done here (bundle time cap). Design for the
  next step, matching the hand's scan / replay split:
  - scan (`record_entry`, `record_answers`): keep the visibility decisions read from the tape (`thought`, `text`,
    `functionCall`) and record raw ranges instead of decoded text. `doc.source` is a per-line copy with no proof
    link to `data`, so each kept raw string token (escapes intact) and the raw `timestamp` token are appended to
    `log.text`; `doc_strings` keeps `utf8_valid(log.text)` (comparable to serde allocating each string at parse);
  - replay, chosen entries only: a plain span (tape slot 5) is lent as a view; an escaped span is decoded with
    `decode_append(.., false)` into a scratch buffer lent as `str`; user parts are joined with "\n\n" and the
    blank test runs here; the stamp is `iso_raw` over the raw token, for an assistant only when `st[1] == 1`
    (hand `collector.attached`).
- Tests after 4.4x (rs/rust and rs/llvm): `test_impl.py` PASS 27/27; `q.py --count 300` (rs pairs) same 317, P1 1,
  P4 4, P5 1, unexplained 0, list same 21, P4 1; `q_mutants.py` 11 differential-killed, turn-cap
  verifier-rejected. No perf timing (structure still differs at 4.3e-D).

## 4. Results

4.4w, after the last code change (`pruvc-rs`, verified, no `--no-verify`):

| check (4.4w) | rs/rust | rs/llvm |
|---|---|---|
| `qwen/test_impl.py` (verifier + fixtures) | PASS, 27/27 | PASS, 27/27 |
| `q.py --count 300` (+21 list; `PAIRS` set to the rs pairs) | same 317, P1 1, P4 4, P5 1, unexplained 0; list same 21, P4 1 | same |
| mutants (`q_mutants.py`, 12 markers) | 11 differential-killed, turn-cap verifier-rejected | - |

No `pkgs/` change, so no consumer runs. No perf timing: the structure still differs (4.3e-D / 4.4d-QS).

4.4u, after the last code change (`pruvc-rs`, verified, no `--no-verify`):

| check (4.4u) | rs/rust | rs/llvm |
|---|---|---|
| `qwen/test_impl.py` (verifier + fixtures) | PASS, 27/27 | PASS, 27/27 |
| `q.py --count 300` (+21 list; `PAIRS` set to the rs pairs) | same 317, P1 1, P4 4, P5 1, unexplained 0; list same 21, P4 1 | same |
| mutants (`q_mutants.py`, 12 markers) | 11 differential-killed, turn-cap verifier-rejected | - |

No `pkgs/` change, so no consumer runs. No perf timing was run.

4.4d, after the last code change (`pruvc-rs`, verified, no `--no-verify`, 300 s cap per module):

| check (4.4d) | rs/rust | rs/llvm |
|---|---|---|
| `qwen/test_impl.py` (verifier + fixtures) | PASS, 26/26 | PASS, 26/26 |
| `q.py` 300 cases (+21 list; `PAIRS` set to the rs pairs) | same 316, P1 1, P4 4, P5 1, unexplained 0; list same 21, P4 1 | same |
| mutants (`q_mutants.py` has no `--check`: 12 markers found once each, then the full run) | 11 differential-killed, turn-cap verifier-rejected | - |

Consumers of `pkgs/transport` and `pkgs/turns` are in the Grok table. No perf timing was run.

After the last 4.3e code change, `rs/rust` and `rs/llvm` (hand `edge.rs` unchanged in 4.3e; its row is from 4.3d):

| check | rs/rust | rs/llvm |
|---|---|---|
| `qwen/test_impl.py` (verifier + fixtures) | PASS, 26/26 | PASS, 26/26 |
| `q.py` 300 cases (+21 list) | same 316, P1 1, P4 4, P5 1, unexplained 0; list same 21, P4 1 | same |
| hand `edge.rs` vs fixtures (21 files) | 0 differences | - |

P1 / P4 / P5 are the classes `q.py` already explains. No perf timing was run.
