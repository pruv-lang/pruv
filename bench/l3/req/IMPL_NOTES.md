# L3-9 implementation notes (oracle section)

Run: `python3 gen_fixtures.py` (regenerates fixtures, prints `wrote 160 fixtures`), then `python3 check_oracle.py`
(flags `-v`, `--group G`, `--id ID`). Expected last line: `oracle check: 160/160 fixtures pass`, with `determinism: identical`.
Needs `node` (oracle.py pipes JSON cases to reference.mjs). Format: see SCENARIO.md.

Files: oracle.py (runner), reference.mjs / reference-hooks.mjs / world.mjs / fakes/ (driver over the pinned handlers),
common.py, gen_fixtures.py, check_oracle.py, fixtures/*.json.

Pinned behaviour worth knowing:
- Open order: sourceOf, folderOf, same checks, accountOf, initial bound check (returns existing before the busy check), busy, opening.add.
- Key is `engine:thread` (folder and account are not part of it); the key is released in finally, also after failures.
- Race re-search after the async read returns existing and removes a Grok copy (rm) if one was made.
- Cancel reason beats any later error (C07, C09); the deadline timer aborts but never forces rejection.
- Timer fires are attributed to the request that created the timer.

Known limits:
- S04 and S08 are visible only through the sources output and trace counts (account labels are not exposed; `world.accounts` is not a queue).
- P fixtures carry `layer:"packet"`; the oracle runs the honest scenario only and does not assert packet-layer faults.
- Mutant killers (S05 F03 B05 L03 L04 A04 A07 I06 I09 C03 E08 C07 I03 R01 U07 O08) were designed from the spec. They were
  later run against built mutants of the Pruv product, not of the reference (see Packet integrity below: 16/16 killed).

## Core/host (M0-M1)

Layout: `root.pruv` (req_begin / req_step, envelope validation, done/await assembly), `req_packet` (32-word header, sections),
`req_world` (world packet, active rows, owner/ticket/generation), `req_state` (control-state section), `req_child` (child entry
packets in1/in2, accessors), `req_value` (3-word slots, rebase, copy), `req_sources` (op 1 projection, done tables),
`req_error` (done records: thrown, sourceUnknown wire error, core codes), stubs `req_async/bound/list/open/fill`.
Host: `host.rs` (scheduler skeleton, Case, trace/observation), `leaf.rs` (reuses `bench/l3/entry/host.rs` via `include!`:
leaf serving, input encoding, done rendering), `harness.rs`, `dev_build.sh` (`sh dev_build.sh` builds `work/dev/harness`),
`dev_compare.py` (`python3 dev_compare.py S [ids]`: oracle vs harness on whole observations).

Flow: begin validates and calls entry_begin (op 1 input), then `after_op1`: child await -> request await (child packet in
section 5); child done -> op 1 builds the sources done, ops 2/3 call entry op 2 (find) and `after_find`: no match -> the
sourceUnknown wire error; match -> core InternalInvariant (stub until M2).
Entry envelope detail: the child input must have exactly the sections the op claims (op 1: pool + record = 2 sections; op 2:
pool + record + one table = 3). Extra empty sections make `all_claimed` reject the input as InvalidPacket.

Status: S01-S10 equal to the oracle (S05 and S06 included). Stubbed: found-source path of ops 2/3, async/timers/deadline,
bound-check, fill, non-start events (the host returns an error for them), checkpoint words.

### ABI deviations
- Section 3 of the request packet holds the accumulated source rows (6 slots each), not an item anchor.
- Section 4 and checkpoint words 16-23 are zero in M1.
- The request pool accumulates every child pool on each child await (rows are rebased onto it).
- Ops 2/3 record: the 24 op-1 words plus the source and folder slots (30 words).
- An invalid-packet done packet echoes the ids as read.
- The top-level API is in root.pruv, not a req_api module.
- Titles map (4.4): there is no arena or radix trie. The host keeps the titles sessionList snapshot of call 12 as an immutable
  per-request block (`ListSt.tsnap`) and hands it back once, through host call 18, when every item is processed and some item is
  bound. Pruv then picks the title (last session with an equal id wins, `||` fallback) in `req_list.title_at`; the host does not
  search, compare or choose. The continuation during the bound searches carries only the items, the header and the bound ids.

## M2 (partial: folder preflight and the list success path)

State (dev_compare): S 10/10, F 10/10, L 10/10, D 2/10 (D01 codex list, D04 opencode list); B, A not worked on.

Added:
- Folder preflight: await ck=2 (`emit_call`), `after_folder` maps a thrown folder error to the `wireWork.engineImport.folder`
  wire error; host `serve_folder` serves world.checkFolder.
- List flow in Pruv (`root.pruv` `list_step` pc 3-9, `req_list.pruv` working set W in section 4): pc 3 call [11,isCodex]
  (timer.set, makeCodexRunDeps, source/options refs), child op 3 (list_item, reader through invoke_engine code 9), pc 4/5 call
  [12,m,rowrefs] (timer.clear, sessionList scan with titles, row fields), per-item bound search via call [14,j] (next session,
  child op 8 excluded_item with isTemporary code 8), op 7 threads_item, call [16,j] (sessionList.return), result projection
  with exact key order and `boundSessionId`/`maybeOpen` presence rules (`fin_*`). Await ck=3 carries host call words in section 5.
- Host: `leaf.rs` `serve_call` (codes 11, 12, 14, 16, 17), `serve_reader` (code 9 intercept: reader event with options
  {keys, signal sigN, aborted, env, codex}), code 8 isTemporary prim converted to an oracle event with the world value, events
  flushed into the trace (`flush_events`) after prims. `Leaf::new` takes the raw world text (key order for reader/makeCodexRunDeps).

ABI deviations: ck 3 await (host call) with the call words in section 5 and the working set in section 4; the host replies with
the six completion sections. Op 3 input record has 39 words. Reader rows reach Pruv only as `RefVal::Row` refs plus field slots.

Not done (first differences):
- importFailure mapping (AbortError, EngineImportError codes, rethrow) is not implemented; a reader throw or a child op 3 done
  tag != 0 ends in core InternalInvariant. No F/L/D list target exercises it.
- D02, D03, D05-D08 (open only): `$.requests[0].terminal` oracle ok/thrown, impl core. D09: `$.requests` impl missing (the
  `handle` event is unsupported: "case: unsupported in M1: ev handle"). D10: `$.requests[1].terminal` oracle ok, impl core
  (open path).
- B and A groups unchanged (open paths, sameRefused, fromAppAction, grokAccounts).
- B07 under test_impl llvm/rs variants reports "case: script entry needs ok, throw or errno" (not examined).

## M3 (ordinary open flow and importFailure)

State (dev_compare): S, F, L, B, D, T, U 10/10; A 8/10, O 9/10, E 9/10, I 9/10. `test_impl.py --group S,F,L,A,B,D,O,T,U,E,I`:
all four variants identical, the only non-exact cases are the five Grok-copy cases below (M4).

Added:
- Open flow in `root.pruv` (open_pre, open_acct, open_bound, open_cont, open_fail, read4_done, pc 21-27): sameRefused, sameByAi
  (host call 30), accountOf (call 31), initial bound check before the busy check (an empty bound id does not count as a hit),
  busy, phase-2 race re-search, importFailure (call 25 classifies: AbortError -> timeout, EngineImportError codes -> wire
  keys, read failure -> readFailed, anything else rethrown as thrown).
- Host (`leaf.rs`): opening registry per instance (opening.has/add/delete events and `openingEvents`), listener/timer for
  `timed`, readOptions (codex, opencode with locateAcp and tempRoot), reader `read` (ok, reject, defer), call 27 (acquire and
  prepare), 29 (timer.clear, listener.remove), 32 (now, uuid per turn, modelFor, createImported with fill), and
  `createdSessions`/`activeTimers`/`activeListeners`/`finalOpening` observations. `host.rs`: events handle, cancel (no live
  listener), mutate add, settle, `fromAppAction` and `cancel` start fields, key release at request end.

Deviations: none that M3b leaves open (see below). The open path reuses the list bound-search machinery (working set of
54 words, one item).

Not done (first differences, all Grok copy / cancel timing, M4): A06, A08, O07, E10, I04 end with "case: grok copy is M4"
(`$.requests` missing). Also not supported: createImported modes other than ok, modelFor throw, cancelling a running read.

## M3b (handler decisions moved from the host into Pruv)

State (dev_compare and `test_impl.py`, four variant pairs): S, F, L, B, D, T, U 10/10; A 8/10; O 9/10; E 9/10; I 9/10.
The five non-exact cases are the Grok copy M4 cases A06, A08, O07, E10, I04.

1. Opening registry in Pruv: `req_world` (`open_find`, `open_add`, `open_del`, `rel_world`) keeps the keys in the world words;
   `req_open` builds the event blocks (`acquire_eb`, `busy_eb`, `rel_eb`); `root.pruv` decides has/busy/add and the release at
   the end of the request. The host only validates and records the events (`apply_events`) and sets up the listener, timer and
   read options (call 27).
2. importFailure in Pruv: host call 25 reports facts only (is EngineImportError, error name, code). `req_error.map_wire`
   maps AbortError to timeout and the EIE codes to the wire keys; anything else is rethrown as the thrown record.
3. Fill recipe in Pruv (`req_fill.pruv`): host call 33 returns the view of the read result (title, omittedBefore, lastPlan and
   per turn prompt, reply, createdAt, completedAt, tools, clipped). Pruv builds the title (whitespace collapse, trim, 80 units,
   basename(folder), folder), the thread slot name, the bound/origin flags (account, fork, lastTurnId, omittedBefore,
   lastPlan) and the per-turn flags (createdAt is now, completedAt present, tools > 0, clipped truthy), and sends them in call 32
   with the opening released. The host only supplies Date, randomUUID per turn, modelFor, createImported, and applies the recipe.
   Pc 28 is the view reply step.

Host calls now: 11/12/14/16 list (timer, session list, next session, return), 17 folder facts, 30 sameByAi fact, 31 grokAccounts
fact, 27 acquire (listener, timer, read options), 25 failure facts and cleanup, 29 timer clear and listener remove, 33 view of
the read result, 32 create (now, uuids, modelFor, createImported).

Remaining deviations: Grok copy (M4), createImported modes other than ok, modelFor throw, cancelling a running read, a
non-string title is a core error.

## M4 (Grok copy and cancellation)

State (dev_compare, all 15 groups; `test_impl.py` 160 fixtures x 4 pairs): S F L B D T U G A O E I R C X 10/10 on every pair.
A06, A08, O07, E10, I04 and the whole of G, R, C, X equal the oracle on the complete observation. The only non-exact cases
left in `test_impl.py` are P07 and P08 (bound classes P5/P6, not part of M4): P07 differs at
`requests[0].value.obj[0][1].arr[0].obj[1][1]` (a clipped long string), P08 at `requests` ("bound not enforced").

Added (Pruv, `root.pruv`):
- Grok open flow: pc 30 grokHomeFor (call 34), pc 31 copy plus verify (call 35), then the race (`open_race`). A thrown
  grokHomeFor is mapped by `open_fail` tag 3 (pc 32, engine-missing). A race hit removes the copy (call 36, pc 33) and returns
  the existing session.
- Cancellation: the host gives the aborted fact and an opaque reason reference with calls 29, 35 and 25 (12-word reply:
  [0, isEie, name slot, code slot, aborted, reason slot]). Pruv decides: for non-Grok the aborted fact after the read throws
  the reason (pc 25); for Grok the check comes after the copy (pc 31, `open_fail` tag 4 = remove the copy first, then the
  cancel). In the catch (pcs 27, 29, 32) the cancel reason is thrown before any error mapping (`cancel_done`).
- Cleanup requests: call 25 tag 11 and 14 request the rm of the copied path first (phase 2 failure, or cancel after the copy).

Added (host, facts only):
- Leaf-side holds and windows: a hold is a deferred world entry (reader, copy, loadCheck, rm, makeCodexRunDeps with `defer`),
  a window is one `timed()` (cancel listener, 20000 ms timer, lazily named signal). `settle`, `advance`, `cancel` events run the
  exact scheduler order of ABI 7 (timers by (due, id), `now` = max(now, due)). A held entry with `onAbort` rejects when its
  window signal aborts (trace line `settle ... via:abort`).
- Patched trace lines: a deferred call is recorded with `pending` and patched to its outcome when it settles.
- Host calls: 34 grokHomeFor (stores home, binary, env), 35 copyGrokSession (records `copy`, `copy.verify`, runs the verify
  window and loadCheck; wraps a non-Abort verify failure as EngineImportError grok-load-failed with cause; replies the aborted
  fact), 36 rm (recursive, force, of the copied path; a rejection is a thrown reply that Pruv swallows). Call 29 and call 25
  now reply with the aborted fact and reason reference. Call 32 no longer releases the opening itself.

Deviations: the cancel reason is an opaque `$cancel:<handle>` reference rendered by the host at the final value; the host
stores the grokHomeFor home and the copied path in the open state; the host builds the wrapped EIE of the verify failure; a hold
registered on an already-aborted signal is not auto-aborted (no fixture needs it). The list handler's timer is not a window and
cannot fire through `advance` (a case error).
Closed from M3b: createImported fail before/after fill, modelFor throw, cancelling a running read, non-string title (no
fixture left that differs).

## M5 (Q suite 2000/2000, P08 limit)

Q: `python3 q.py --impl work/dev/harness` 2000/2000 equal (1344 at the start of M5). Every fix below is in Pruv or in the
dependency side of the host; no oracle or generator file changed and no oracle/generator bug was found.

Clusters (cause -> fix):
- Title clip (about 160 cases): `req_list.title_at` took the wrong slot for a cut title (own vs cut position swapped) -> the cut
  title is slot +3 only when the entry action is cut, else +6.
- clockStart: `Date.now()` of open_create must read the scenario `clockStart` -> leaf `now` seeded from it.
- Lone-surrogate strings: `oj_slot` had no u16 arm for `{u16:[...]}` strings in item slots -> added.
- Settle override (87 cases): `settle {label, ok|reject}` replaces the held entry's outcome (oracle: `{...entry, ok: undefined,
  reject: undefined, ...override}`) -> the host pre-scans the script for overrides and the leaf applies them (`Leaf::ov`) at every
  world lookup that can carry a `defer` label (label with its `#n` repeat suffix).
- List reader defer / reject / onAbort: the list reader now supports ok, reject, defer (hold with pending trace line, `onAbort`
  completion) and the list timer is a window (`Win`) so it can fire through `advance` and abort the read.
- timer.clear: the `active` flag is now the real liveness of the timer; `finish25` emits timer.clear only for a live or fired,
  not yet closed timer (the open flow already clears some timers).
- mutate remove / replace (45 cases): the world sessions are filtered / replaced by `id` exactly like reference.mjs (`===`
  semantics through the rendered id), the trace line carries `{op, id}` (id null for add).
- EngineImportError message (3 cases, Q2): the reference builds `EngineImportError(code, detail)` whose message is
  `code: detail` for a non-empty detail and `code` otherwise. The shared entry host (`bench/l3/entry/host.rs`, not editable here)
  observes the scenario message unchanged, so the L3-9 leaf wraps `parse_err` (`parse_err_l`) and folds the code in. The
  folder error wire message is therefore `zzz: <message>`, `timeout`, `not-found: <message>` as in the oracle.

P08 (33 concurrent opens): the 32-active-request limit is enforced in Pruv: `req_begin` ends a request whose instance already
has 32 active rows with core InputTooLarge before any dependency call (`req_world.inst_active`, instance row word 1). The host
only records request.start / request.done. A settle for the hold of such a rejected request (label r#33, never created) is
skipped by the host when a request ended core InputTooLarge and the label has no hold.
P08 now (2026-11-17): compare.py's `default_policy` gives P08 the explicit P6 policy (trigger `active-requests`, limit 32,
`mustTrigger`). Under it the 33rd request's core InputTooLarge and the trace lines it no longer causes (the shifted global call
numbers and the `settle r#33` line) are explained, so `test_impl.py` reports P08 as P6 on every pair, with 0 unexplained.
`dev_compare.py P` still reports 9/10, because it compares exactly, without policies (`requests[32].terminal`: oracle ok,
impl core). That is the expected P08 result, not a bug.

## Integration and performance

Run from `bench/l3/req` with `PATH="$HOME/.cargo/bin:$PATH"` and `GROK_TS_TRANSPILER=<lair>/node_modules/typescript`.

- `python3 integrate.py` runs the 12 spec 9.2 cases through the Node oracle and `work/dev/harness` and compares them with compare.py. Part A (I01-I08) uses injected reader results. That is not real cancel forwarding. Part B (I09-I11) is the Grok copy/load lifecycle. Part C (I12) is one engine error through the real entry wrapper to a request WireError. Result: 12/12 exact, 0 unexplained. Claude is reported as a limit (L3-1 has no driver), not stubbed. Exit 0 = all pass, 1 = failed assertion, 2 = only limits remain.
- `python3 perf_req.py` runs the spec 13 grid (S 1000/5000/20000, M 1/10/50, placements none/first/last, list and open) and writes `work/perf_report.md`. Call counts (M+1 sessionList, M*S isTemporary) hold for every finished run. Packet words are an estimate, not a measurement. Replayed items are not exposed.
- First finding (fixed): with no early match Pruv time grew about S^2 (exponent 1.98-2.04, S=5000 took 57-84 s at M=1). Three
  causes, each O(S) per session step:
  1. leaf.rs codes 14/17 cloned the whole session snapshot per step -> only session j is cloned.
  2. leaf.rs code 8 (isTemporary) re-walked the whole trace per call -> only the lines this call adds are scanned.
  3. The list packet carried the titles snapshot (6 words per session) through every search step, so each step's digest,
     copy and await emission was O(S) -> call 12 returns only `[0, n]` and the item rows, call 16 returns the bound session id
     (kept in item slot 9), and the titles come back once at the finish (call 18, pc 10). This also removed a latent mix-up:
     the old item slot 9 held the search snapshot index and was read as an index into the titles snapshot.
- After the fixes (`work/perf_report.md`, 2026-11-17): every list and open row grows linearly. The local exponent is 0.97-1.03
  for all measured pairs; open S=20000 takes at most 1.6 s. The
  S=2000 list output is byte-identical before and after. Known limit, the constant is large: a searched session costs
  about 47 us at M=10 and 92 us at M=50 (about three steps per session; the packet size is constant in S but grows with M,
  516 words at M=2; each step digests the previous and the new await packet, `req_world.dig` about a third of the samples).
  Pruv is about 4x slower than Node at M=1, 19x at M=10 and 71-81x at M=50 (S=20000 M=50: Node 1.2-1.3 s, Pruv 92-97 s). Skipping the hash reductions for small words did not change the time, so the cost is the per-step
  packet work, not the division. Pruv RSS reaches about 1.1 GB at S=20000 M=50 (the host holds the sessions and the full
  trace). Trace cost is not separated from core cost; packet words are an estimate.
- After the storage chain and the call budget (`work/perf_report.md`, 2026-11-17, full grid): every finished list and open
  row grows linearly (exponent 0.97-1.02). A searched session now costs about 41 us at M=10 and 40 us at M=50 (S=20000
  M=10: Node 0.41 s, Pruv 8.17 s, about 20x), because the packet no longer grows with M. Pruv RSS is at most 0.9 GB. The
  no-match list at S=20000 M=50 needs M*S + M + 1 = 1,000,051 external calls, more than the spec 7.2 bound, so Pruv ends it
  with core InputTooLarge after about 1,000,000 calls (20 s; Node, which has no bound, finishes in 1.2 s). `perf_req.py`
  asserts that outcome for such cells instead of the call counts; the growth exponents 0.51-0.52 of that pair only reflect
  the stop.

## Packet integrity (digest, seal, driver freshness)

- World word 11 is the seal: `req_world.dig` of every other world word. `world_ok` rejects a world whose seal does not match;
  only the host's initial world (generation 0, no active or opening rows) may carry seal 0.
- Active row word 9 holds the digest of the await packet the core issued for that row. A resumed step must present a previous
  await whose digest matches, so an edited or replayed prev packet (or one from another row) ends core InvalidPacket.
- `dig` is a two-modulus polynomial hash (2147483629 and 2147483587). It detects edits and replays by an honest-but-buggy or
  replaying driver, not chosen collisions; it is not a MAC.
- Driver freshness (host.rs `drain`): an accepted step always advances the world generation. A step result that does not (a
  core rejection of a stale or forged input, or a replayed triple) is not adopted; the host reruns the step with the prev magic
  cleared, so the core rejects it against the latest world and no stale effect runs.
- Forgery suite (`forge_test.py`, report `work/forge_report.md`): 66/66 rows behave as required. These are 52 forgeries
  rejected with InvalidPacket and the observation equal to the reference rejection (F41-F48 storage, F49-F50 world budget,
  F51-F52 completion records),
  6 budget rows B01-B06 that end with InputTooLarge after exactly the owed cleanup, and 8 honest controls unchanged.
- Mutants (`q_mutants.py`, spec section 11): 16/16 killed by their killer fixtures (14 Pruv edits, M10 and M11 in the host leaf).

## Review fixes (2026-11-17)

- Finding 7, the call budget (ABI.md 10.1). World words 9 and 10 hold the external calls used and the cleanup credits
  reserved. `charge` in `root.pruv` admits every child, folder and host-call await against
  `w9 + 1 + w10 <= 1,000,000` and moves the request's credit with `held(op, pc)`. A refused request with a credit runs host
  call 25 at pc 34 (timer clear, window close, rm of a Grok returned copy, opening release) and then ends with InputTooLarge.
  A refused request without a credit ends with InputTooLarge at once. `world_ok` checks the range of both words. Storage ops
  (kind 4) are free, and so is host call 17 (the snapshot session re-read, no trace line; `await_cost`). Iterator steps
  with a trace line (host calls 14 and 16) count. Counting call 17 as well made a 45,000 x 11 list stop after 333,327
  `isTemporary` lines; it now finishes.
- Findings 5, 6 and 8, the storage chain (ABI.md 4.4 and 5.5): finished list items live in a per-request block chain
  checked by digest, so the list continuation holds one item, and a 100,001-session list fits the packet cap.

## Confirmation audit fixes (2026-11-17)

- Finding 4, completion formats: pc 8 (host call 17) now requires the record `[0]` and an empty pool, since the answer is in
  section 5 only (F51). The folder success slot at pc 2 must lie inside the completion pool (`slot_ok`, F52).
- Finding 7, the budget and the Grok copy: a copy that returns after a cancel (pc 31, `open_fail` tag 4) asks for the
  cleanup call 25 tag 14, which still holds the credit of pc 27. When the budget refuses that call, `over` saw phase 1 and
  picked tag 2, so the copy stayed on disk. `open_fail` tag 4 now refuses through `over_t` with tag 12 (rm first). Budget
  row B06 (R03 step 15, w9 = 999999) reproduced the missing rm before the fix and ends with listener removal, rm, opening
  delete and InputTooLarge after it.
