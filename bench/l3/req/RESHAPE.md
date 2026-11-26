# req: Rust-shape reset (step 4.4)

Scope: L3-9 engine-import request handlers (`engine-import-sources`, `engine-import-list`, `engine-import-open`;
`reference/backend-requests-engine-import.ts`, `docs/ENGINE_IMPORT_REQUEST_SPEC.md`). Pruv: `root.pruv` and 13
`req_*.pruv` modules, 196 functions, a world / packet step machine (`req_begin` / `req_step` bundles, world seal on
every bundle, `ABI.md` L3-9 ABI v1). Hand Rust: `hand/req.rs` (new in 4.4), safe std Rust in the synchronous-fact
shape the plan targets.

**The port is blocked.** The P10H gate (synchronous G6 host facts; fresh-query / order and timing contract,
`docs/L3_RESTRUCTURE_SPEC.md:232`, `:378`, `:485`, `:529`, `:1245`) has passed only as the `rq_find` kernel on the Rust
backend (rs/rust 1.013x); rs/llvm is 1.193x (P17b / P17c pending), and no full list / open grid was run. Porting
req's decisions now would mean keeping the world / packet structure the plan replaces, or building the
synchronous-fact `RequestStore` before its gate. So 4.4 writes the hand baseline and this table; the port itself is
row REQ-PORT. Nothing in the Pruv code changed.

## 1. Structure table (hand function -> Pruv functions that make the same decision)

`R` = `root.pruv`. "Same" = same inputs to the decision and the same order of checks and exits.

| hand `hand/req.rs` | Pruv (file:line) | Structure |
|---|---|---|
| `Engine::name` 51 | `req_open.pruv:7 key_of`, `req_fill.pruv:191 slot_name` | enum to text; Pruv reads the source's text slot |
| `wire_fail` 410 | `R:400 wire_req`, `req_wire.pruv:3 pool`, `:219 rec` | same key and message |
| `source_unknown` 414 | `R:916 after_find`, `req_error.pruv:28 unknown_pool`, `:35 unknown_rec` | same; exits before the folder check |
| `Requests::sources` 421 | `R:934 after_op1`, `req_sources.pruv:70 tabs` | same public projection; hand could move out of the owned `Vec` |
| `Requests::list` 439 | `R:327 read_done`, `R:725 list_step`, `R:265 list_done`, `req_list.pruv:655 title_at`, `:685 fin_tabs` | one title snapshot, then a fresh bound lookup per item; last write wins per id; empty title falls back. Hand: `HashMap` + `Vec::with_capacity`; Pruv: stored blocks, stable sort, binary search (REQ-W) |
| `Requests::open` 482 | `R:492 open_pre`, `R:474 open_acct`, `R:442 open_bound`, `R:508 open_cont` | source -> folder -> same-mode source check -> AI check -> account -> bound -> busy -> acquire: same order |
| `open_body` 518 | `R:547 open_fail`, `R:601 cancel_done`, `R:74 done_ev` | failure cleanup boundary; key release point differs (REQ-K) |
| `open_steps` 537 | `R:570 read4_done`, `R:624 open_step`, `R:612 open_race`, `R:529 open_recipe`, `R:408 open_done` | read -> Grok -> cancel -> fresh bound re-check -> create; race: `rm` then existing id, no clock / uuid / model / create |
| `copy_grok` 610 | `R:676-685 open_step` calls 34 / 35 | verify is a callback the copy side calls (fixed in 4.4, §3b) |
| `import_failure` 618 | `req_error.pruv:93 map_wire`, `R:709-719 open_step` | same keys for the classified codes; Pruv checks the error name before class / code |
| `timed` 638 | `R:897 after_folder`, `R:520 open_cont`, `R:580 read4_done`, `R:679 open_step` | no Pruv function; the call adapter owns timer and listener (REQ-T) |
| `source_of` 647 | `req_child.pruv:28 in2`, `R:916 after_find` | first match, borrowed |
| `folder_of` 653 | `R:881 after_folder`, `req_error.pruv:63 folder_key`, `:70 folder_default` | same |
| `bound_session_id` 664 | `R:272 next_call`, `R:363 after_op8`, `R:379 bound_hit`, `R:797-830 list_step` | temporary flag -> `is_temporary` -> candidates -> first match; hand copies the list first (REQ-B) |
| `session_threads` 684 | `R:376 after_op8`, `R:790-799 list_step`, `R:379 bound_hit` | hand compares in place; Pruv compares child op 7 `(engine, thread)` rows |
| `account_of` 711 | `R:452 acct_default`, `R:458 rows_acct`, `R:474 open_acct`, `R:642 open_step` | same branches; Pruv compares the literal `"default"` (REQ-D) |
| `read_options` 732 | `R:881 after_folder`, `R:647 open_step`, `req_child.pruv:63 in3`, `:84 in4` | same per-engine branches |
| `turns_of` 759 | `req_fill.pruv:203 gt0`, `:220 recipe`, `R:529 open_recipe` | same turn order and `tools > 0` check; hand clones bodies (could move) |
| `is_es_space` 780 | `req_fill.pruv:128 title_head` (`is_trim_unit`) | ECMAScript space units |
| `title_of` 787 | `req_fill.pruv:105 title_head`, `:151 title_pool` | title -> first prompt -> collapse / trim / 80 UTF-16 units; Pruv builds only the bounded prefix (REQ-L) |
| `basename` 830 | `req_fill.pruv:51 basename` | hand borrows `&str`; Pruv allocates a slice |
| `thread_slot` 840 | `req_fill.pruv:191 slot_name` | `engine:account` only for a non-empty account |
| `Host::*` 339-393 | child ops 1 / 3 / 4 / 7 / 8 and host calls 11, 14, 25, 27, 31, 32, 34, 35, 36 (`R:272`, `R:520`, `R:542`, `R:547`, `R:636-714`, `R:775-814`, `R:881-934`, `R:992`) | each synchronous fact is an await / call packet in Pruv; `session_list` and `is_temporary` are fresh per call on both sides |

Pruv functions with no hand counterpart (REQ-W, removed by the port; the synchronous shape uses calls, locals and
`Result` instead):

| Kind, file | Functions | Replaced by |
|---|---|---|
| world, `req_world.pruv` (27) | `m_world, cnt, wgen, next_owner, next_ticket, world_ok, initial_ok, dig, seal, act_set, inst_at, act_at, open_at, inst_find, inst_active, act_find, act_word, apply, pool_len, key_eq, open_find, open_word, open_add, open_del, open_find_rid, open_key, rel_world` | `Requests.opening` and the call lifetime |
| world, `req_open.pruv` (8) | `ev, eb0, eb, eb_cat, rel_eb, acquire_eb, busy_eb` (+ `key_of`, §1) | `HashSet` insert / remove |
| packet, `root.pruv` (31 of 54) | `mkh, out, done_same, done_ev, done_req, core_req, bad, child_end, held, held_prev, await_cost, charge, over, over_t, emit_await, emit_call_x, emit_raw, emit_call, call2, store3, item_end, load_next, open_w, thr, thrown_done, cancel_done, begin_ok, req_begin, words_eq, step_ok, req_step` | calls, locals, `Result`; budget / resume / seal are outside the port |
| packet, `req_packet.pruv` (12) | `m_req, m_bundle, rd, nsec, sec_len, sec_start, sec, empty_pool, hdr_ok, child_extra, mk, bundle` | typed arguments and returns |
| packet, `req_state.pruv` (5) | `m_ctl, new_state, state_ok, owner, pc` | the call stack |
| adapter, `req_child.pruv` (16) | `in1, in2, in3, in4, kind, call_id, seq, op, sec, table, pool, record, rows, ok, ok_any, done_tag` | direct host calls with typed results |
| adapter, `req_async.pruv`, `req_bound.pruv` | `stub` (body `return 0`) each | nothing |
| wire, `req_wire.pruv`, `req_error.pruv`, `req_sources.pruv` | `pool, rec`; `unknown, unknown_pool, unknown_rec, core_rec, thrown_rec, folder_key, folder_default`; `has_acct, nrows, dpool, drec, tlens, make_keys, tabs` | `WireError` statics; `Vec<PublicSource>` |
| packet / value, `req_list.pruv` (33) | `hget, hset, ib, sb, w_new, w_items, w_fill, w_sess, pack, pack_bound, blk, w_one, w_load, pack_units, rec_ok, chain_ok, chain_dig, w_full, chain_units, any_bound, wset, make_keys, fin_pool, fin_rec, present, has_bound, cnt, fin_tl, scmp, cmp_units, sess_order, title_at, fin_tabs` | an item `Vec`, one title map, a fresh lookup per row |
| value, `req_fill.pruv`, `req_value.pruv` | `view_ok, collapse, title_ok, view_strs_ok` (+ §1 rows); `tag_at, nullish, put, copy_slot, units, is_str_lit, rebase, copy_slot_off, truthy, slot_eq, slot_ok, slots_ok` | typed `External` / `Seed` / `Turn`, `Field<T>` for missing vs null |

## 2. Line counts

| | Lines |
|---|---:|
| Pruv `root.pruv` + `req_*.pruv` (196 functions) | 3575 |
| hand `hand/req.rs` (non-test part, 24 functions + 23 host facts) | 850 |
| hand tests (`FakeHost`, 19 tests) | 637 |

## 3. Differences left as rows

| Row | What | Where |
|---|---|---|
| REQ-PORT | the port itself: every row of §1, in the synchronous-fact shape, after P10H passes on both backends | all `req_*.pruv`, `root.pruv` |
| REQ-W | world / packet / adapter / wire functions with no hand counterpart (§1 second table) | removed by REQ-PORT |
| REQ-K | Pruv releases the opening key when it emits the failure `rm` (call 25, `R:557-566` via `emit_raw` `R:239`); TS / hand hold it through cleanup, cancel check and error mapping and release it once (`finally`) | `R:547 open_fail`, `R:74 done_ev` |
| REQ-D | Pruv `acct_default` compares the literal `"default"`; TS / hand use the bound `DEFAULT_ACCOUNT` | `R:452` |
| REQ-B | hand `bound_session_id` copies the whole session list (`to_vec`) because `session_list` borrows the host while `is_temporary` needs `&mut`; TS / Pruv walk one session at a time and stop at the first match. Needs a cursor or a separately borrowable view | `hand/req.rs:664` |
| REQ-T | hand `timed` skips the work when the cancel is already set; TS links the parent signal, starts the 20 s timer and still runs the work with an aborted child signal; the timeout does not force a failure. No signal / env in `ReadOptions` | `hand/req.rs:638` |
| REQ-X | hand host facts that can throw in TS (`session_list`, `is_temporary`, `grok_accounts`, `from_app_action`, `now_iso`, `uuid`, `model_for`) are infallible, and `create_imported` failures are all `Other(String)`; TS propagates them by site (raw / wire / cleanup then cancel) and R4-R8 drive those paths in Pruv (`R:635`, `R:698-714`, `leaf.rs:1357`) | `hand/req.rs:339-393` |
| REQ-O | `open(&mut self)` serializes opens on one instance, so a second open cannot see `busy` for a key held by a running first open (tests insert the key); TS / Pruv allow overlap | `hand/req.rs:482` |
| REQ-S | `Seed` is data, not the `fill` callback: `progress`, `status`, `usage`, nested `imported`, and keeping other `engineThreads` slots are the host adapter's job and are not checked here | `hand/req.rs` `Seed`, `turns_of` |
| REQ-L | hand `title_of` collapses the whole candidate to UTF-16 and then cuts at 80; Pruv builds only the bounded prefix. Rust `String` input cannot hold a lone surrogate | `hand/req.rs:787`, `req_fill.pruv:105-178` |

## 3b. Codex review (gpt-6.2-astra, high, one run)

Fixed in `hand/req.rs` (behavior differences from TS):

1. Grok verify: the hand ran `grok_load_check` after every copy and removed the copy itself. TS passes a verify
   callback; the copy side decides whether and with which home to call it, maps the failure and cleans its own partial
   copy (`R:676-685` call 35 can skip verify). `Host::copy_grok_session` now takes the callback; `copy_grok` only
   builds it.
2. Empty copy path: `rm("")` ran on failure and race; TS `if (copied)` skips it. Both sites filter empty paths.
3. `Seed.account`: an empty non-Grok account was seeded; TS seeds only a truthy account (`model_for` still gets the
   raw value).
4. Missing vs null: `ListItem` / `ListRow` `updatedAt`, `size`, `messages`, `path` and `completedAt` now use
   `Field<T>` (missing / null / value; TS omits only `undefined`); `updatedAt` takes a number or a string (`Stamp`).
5. `lastPlan`: was a string only; now `Plan` (text or raw object), included when truthy, never stringified.

Kept as rows: REQ-K, REQ-D (Pruv side), REQ-B, REQ-T, REQ-X, REQ-O, REQ-S, REQ-L (hand-model limits or port work).
The review confirmed the list title priority and the `open` early-exit order match TS.

## 4. Results

Pruv unchanged, so these are the 4.4 baseline runs (logs in `work/reset4_4/`, not committed):

| Test | Pair | Result |
|---|---|---|
| `rustc --edition 2021 --test hand/req.rs` | hand | 19 passed, 0 failed, no warnings |
| `test_impl.py` | rs/rust, rs/llvm | 160 variants x 2 = 320 runs, 0 unexplained |
| `q.py` (300 cases, Q1-Q8, first 300 of `q_gen` 2000) | rs/rust | 300 / 300 equal |
| `q.py` (same 300 cases) | rs/llvm | 300 / 300 equal |

No perf run (the port is blocked; P10H rows in `lib/RESHAPE.md` step 4.4).
