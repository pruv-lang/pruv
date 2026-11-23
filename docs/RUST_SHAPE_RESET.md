# Rust-shape reset

User decision, 2026-11-24. This document is the plan of record for replacing Node-shaped Pruv code with code shaped like hand Rust. It overrides the per-row perf-debt work in `docs/L3_RESTRUCTURE_SPEC.md` §7.5 until the reset is done. That work is frozen. The R-queue in §7.4 still records what each piece must keep passing.

## 1. Principle

1. **Shape comes from hand Rust.** Every product module is written the way an experienced Rust programmer writes the same job: typed structs and enums, owned collections where Rust owns, borrowed slices where Rust borrows, `inout` where Rust takes `&mut`, ordinary loops. Pruv then adds proofs on top (requires/ensures, bounds and overflow facts that remove checks).
2. **Node/Electron is the output oracle only.** Tests feed the same input to the TS implementation and to Pruv and compare outputs. The TS code never decides data layout, module boundaries or state shape.
   - Bugs found in TS are fixed in Pruv and recorded as known differences, judged against the spec.
   - Behaviour must match the TS implementation, or improve on it.
3. **Packets live only in test harnesses.** Word packets (`[i64]` frames, `words_array`/`unpack`), step-by-step begin/step wire calls and checksum adapters may exist in `*_test`/`perf` roots that talk to the oracle. Product modules take and return typed values.
4. **Missing language features are fixed, not worked around.** If hand Rust can express something that Pruv cannot, the fix goes into the language, both compilers, `SPEC.md` and the shared library. During the reset this beats the AGENTS.md concept-5 rule "work around now and list the gap".
5. **Structure check before merge.** Each bundle puts the Pruv module next to its hand Rust counterpart and lists, per function, the types, ownership, loops and allocation sites. Any difference needs a written reason: a proof-driven removal, or a gap row.
6. **Performance follows from shape.** Each piece is still measured: time, peak RSS and allocation count at M/L against hand Rust, ≤1.0x. Gains below 1.0x come only from §5.

## 2. Diagnosis (why the reset)

- Fragments copied Node data shapes, and the shapes spread through shared `pkgs/` and `lib/`:
  - a global `HashMap<i64, WireNode>` node store (budget M12)
  - `[i64]` word packets and `Ctx` word sections (`lib/ent_*`, `req/*`)
  - line copy, then `parse_owned` tape, then an owned `str` per field (grok RPC, qwen, codex, opencode)
- Per-row fixing then spent each bundle on one gate cell.
- The classification of the 26 open M/L gate rows (2026-11-24) was: N9 T6 P4 S4 L2 C1. Only 3 of 26 were language or compiler causes; 14 of 26 were representation causes.

## 3. Inventory summary

Inventory delegation `64c782e8` (2026-11-24, readable with `history.read-delegation`) classified 164 Pruv modules against hand Rust:

| Verdict | Count | Meaning |
|---|---:|---|
| N | 35 | Representation differs from hand Rust at the core |
| P | 65 | Partly matches |
| R | 13 | Matches on the compared path |
| ? | 51 | No direct hand counterpart, or scope unconfirmed |

- **Node-heavy (N):**
  - `pkgs/turns`
  - `lib/ent_bound`, `ent_dispatch`, `ent_packet`, `ent_state`, `ent_value`, `entry_api`
  - budget `bud_auto`, `bud_charge`, `bud_cleanup`, `bud_clock`, `bud_halt`, `bud_lifecycle`, `bud_map`, `bud_store`, `bud_tools`, `bud_vlife`, `bud_vwords`, `bud_world`, and the word-packet roots `m4`/`m7`/`m8`/`m9`/`m10`/`m11`/`m12`
  - `codex/read`, `grok/rpc`, `mcp/perf/old_packet`, `qwen/boundary`, `qwen/chain`, `qwen/entry`, `qwen/import`
- **Keep (hand Rust uses the same representation):**
  - the JSON tape (`json/hand/hand_json.rs` `parse_tape`)
  - the MCP result packet (`mcp/perf/hand_packet.rs` `core_hand`)
  - the flat arrays in `perf/d`
  - `pkgs/lex`, `pkgs/bytes`, `codex/folder`, `grok/framing`, `qwen/framing`, `supervisor/packet`

  What goes is building a tape where the consumer does not need one, repackaging internal state at every step, and recopying output prefixes.
- **Unmapped:** 51 `?` modules, mostly grok list/copy/path/summary, opencode, req and lib `ent_*`. These have no hand counterpart yet. Each bundle that touches one writes the hand Rust first, then ports.

## 4. Language gaps

| Id | Gap | Hand Rust form | Blocks |
|---|---|---|---|
| B | Borrowed input stored in a struct or returned (`Doc<'a> { src: &'a [u8], tape }`, `&str` field results) | `json/hand/hand_json.rs` `span`, `grok/perf/hand_rpc.rs` `text_of` | `pkgs/json`, `parse`, `lines`, `access`, `jstr`, `lib/json`, `lib/parse`, `lib/lines`, json roots, `codex/item`, `codex/util`, `grok/util` |
| V | Recursion through collections (`enum Value { Array(Vec<Value>), Object(Vec<(Text, Value)>) }`), order and duplicate keys preserved | `budget/m12_test/hand_life.rs:38` | budget M12, `bud_auto`, `bud_lifecycle`, `bud_vlife`, `bud_vwords`, `ent_value`, `req_value`, part of `qwen/import` |
| U | Exact UTF-16 keys as a first-class view (`&[u16]`) next to UTF-8 `str` | `p01_keys/hand_keys.rs`, M4 `decode_key` | `bud_keys`, `bud_vwords`, `bud_text`, m11, `p01_keys`, `ent_u16`, `ent_path` |
| ABI | Owned collections (`Vec<Turn>`, typed state) across the host boundary, for the Rust backend and the LLVM backend separately | `qwen/handrust` `window_import` | `qwen/boundary`, `q_root`, `root`, `bud_tlife` |
| S | Empty-string allocation (`string_new` effect) | — | Not shown to block; check during V |
| C | Capacity-keeping clone, and clear without free (G-DB64-4, G-DB64-6) | `Vec::clone_from`, `Vec::clear` | Not shown to block; check during V |

SPEC.md §6e currently restricts recursive types (`SPEC.md:744`). V lifts that restriction for `Vec` and `HashMap` payloads.

## 5. Beyond hand Rust: where Pruv can win

Matching hand Rust gives parity. Going below 1.0x needs something that safe hand Rust cannot do. These are the sources, in expected order of payoff:

1. **Proven bounds and overflow.** Safe Rust keeps `[]` bounds checks and `checked_*`/panic paths that LLVM cannot remove. Pruv proves them and emits `get_unchecked` and wrapping ops. Biggest in tape access chains, ring/tail spans and per-unit decoding loops.
2. **Proven validity removes re-validation.** Once a parse proves the input is valid UTF-8 and well-formed JSON, later stages skip `from_utf8`, `unwrap` and re-checks that hand Rust repeats at module boundaries.
3. **Exact sizes from contracts.** When `ensures` gives the output length, Pruv reserves once and writes without growth checks. This saves allocations and memory, not just time.
4. **No panic and unwind paths.** Code without reachable panics is smaller and inlines better.
5. **Effects in signatures.** A pure function is known pure, so calls can be hoisted, merged or memoised without alias analysis.

Each claimed win must show an assembly or instruction diff naming the check that survives in hand Rust and is gone in Pruv (`docs/L3_RESTRUCTURE_SPEC.md` §4.2). A hand Rust author can write the same `unsafe` code; the Pruv claim is that it is proven safe, so the comparison stays against safe hand Rust plus a labelled unsafe variant.

## 6. Order

1. **Language bundle (B, V), both compilers.** Borrowed doc and text fields, recursive owned value with order, duplicates and UTF-16 preserved. Rust-backend boundary first, then LLVM. Tests in `tests/` plus `SPEC.md`. U and ABI follow in the same bundle if small, else a second language bundle.
2. **Shared packages, in dependency order:**
   1. `json`, `lex`, `bytes`, `text`, `names`, `args`, `tail`
   2. `jstr`, `parse`, `turns`, `transport`
   3. `access`, `lines`
   4. `time`

   The old `lib/` JSON copies are removed, not ported; consumers move to `pkgs/`.
3. **Entry/req shared lib, in order:**
   1. `ent_u16`
   2. `ent_path`, `ent_packet`
   3. `ent_value`, `ent_state`
   4. `ent_accounts`, `ent_dispatch`
   5. `ent_bound`, `ent_canon`
   6. `ent_sources`
   7. `entry_api`
   8. `entry/root`, then `req`
4. **Fragments.**
   1. MCP stdio check first: it is the first product candidate, and its `R` modules make it the smallest port.
   2. Supervisor.
   3. Grok RPC and history. The RPC line path can start before B lands: original arg plus start/end positions plus an `inout` buffer (`grok/rpc.pruv:138` against `grok/perf/hand_rpc.rs:575`).
   4. Qwen, Codex, OpenCode.
   5. Entry/req.
   6. Budget, starting from the M12 value model once V lands.
5. **Bulk remeasure.** All pieces, all M/L cells, time, RSS and allocation count against hand Rust. §7.5 rows are closed in bulk. Cells still over 1.0x are grouped by cause (shape, language gap, codegen) and fixed once in shared code.

Each bundle keeps to AGENTS.md: at most 2 subs at a time, 2 hours per bundle, disjoint files.

## 7. Done criteria per piece

- Oracle match: 0 unexplained differences against the TS implementation on the existing differential suites, with fixed TS bugs listed as known differences.
- The structure table from §1.5 is filled in, with no unexplained shape difference.
- Time, peak RSS and allocation count ≤1.0x hand Rust at M and L, measured with interleaved runs on the same machine.
- No word packets or step wire in product modules; harness-only adapters are named `*_test` or live under `perf/`.
- Both compilers, both backends where the piece already ran on both.

## 8. Existing work

- DB63 (grok RPC, branch in worktree `pruv-57782787`) and DB64 (budget M12 slab, branch `lair/db64-brief-budget-m12-typed-fcff`) were stopped unmerged. They are kept as reference for the grok and budget ports.
- L3-1..L3-10 test suites, fixtures and hand Rust lanes stay. They are the oracle and baseline harness for the reset.

## 9. Toward replacing Lair

The reset applies to every later migration step (BLUEPRINT L track). L3 backend pieces follow §6. L4, the screen, is built on Rust UI tools (GPUI, Slint, iced) in Rust shape from the start; Electron is the behaviour oracle there too.
