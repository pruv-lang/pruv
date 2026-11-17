# L3-wide restructure framework: correcting every piece

Design only; inspected 2026-11-18. No implementation, builds or new benchmark results. Citation prefixes: `B/` = `bench/l3/budget/`, `RC/` = `pruvc-rs/crates/pruv-core/src/`, `RP/` = `pruvc-rs/crates/pruv-proof/src/`. Proposed types/signatures are design notation, not necessarily accepted syntax.

This framework governs ALL earlier L3 pieces and L3-10 budget: req, budget, entry/shared ent libraries, json/shared packages, mcp, qwen, grok, opencode, claude, codex and supervisor. Budget is the detailed worked example, not the scope boundary. Preserve the AGENTS rule supplied in this task: rework **L3-1..L3-9 as one restructure batch before plugging them into real Lair**. Intermediate proof/test slices are not separate product integrations.

Pruv owns transitions and decisions; Rust performs allocation and external effects. Product structure follows Pruv, not Node module boundaries. Per-step packets are test transports only (`docs/REQUEST_BUDGET_SPEC.md:3-33`). Every item must reach **<=1.0x natural safe hand Rust**; **>1.1x must be fixed**, including test adapters. (1.0,1.1] is unfinished parity work. Older notes' “within bar”/“explained” labels do not override this requirement. Stage two must beat Rust. Historical TS/Node comparisons remain separate gates; they cannot establish hand-Rust parity.

## 1. Causes and evidence

### 1.1 Cause re-attribution: boundary versus representation

**Flat arrays are not slow per se.** Leg D uses flat `[i64]` records, including a UTF-16/i64 string pool (`bench/l3/perf/d/kernel.pruv:1-8,55-65`), and runs at **0.63–0.68x hand Rust** with matching checksums (`bench/l3/perf/REQ_THREEWAY.md:48-60`). The experiment removes the step protocol, not the flat layout. It has no keyed hash-table lookup/compare workload: its unit-by-unit string comparison does **not** establish that key width is free in M8 hashing, keyed equality or budget-ID comparison.

Historical measurements below are not predictions. M8 failure has approximately 694 generated versus 284 hand instructions (`B/M8_NOTES.md:185-202`). Its bins total approximately 662: driver 14, valid 70, decode 68, lookup frame 80, hash 100, equality 90, budget ID 110, record/rest 130; approximately 32 are unassigned/rounding. These are generated-work bins, not independently removable excess. Cross-piece seconds, samples, ns and instructions are different measurements and must not be added.

**BOUNDARY costs — re-check/re-pack, round trips, record copies and re-checks of proven accessor invariants**

| ID / item | Numbers and evidence | Attribution / fix priority |
|---|---|---|
| X1 req world seal/digest/check | S=20000, M=10: B 8.189s vs C 4.993s, **3.196s (~39%)**; 420k calls (`bench/l3/perf/REQ_THREEWAY.md:38-41,64-71`) | Rechecking and sealing each session × item exchange. Trusted resident state and proved transitions remove internal integrity work; validate external answers. |
| X2 req host round trip/re-pack | C remains **11.3x Node**; B **19.5µs/call**, C **11.9µs/call**, residual **4.99s (~61%)** (`bench/l3/perf/REQ_THREEWAY.md:41,70-77`) | Residual includes packet construction, world encoding, host service/trace, not just transport. Execute search within Pruv; batch only facts whose stability and ordered observations are proved under §3.2. |
| X3 req copies/allocation | Self profile: memmove/memset **489 samples (11%)**, malloc/free **~1200 (~27%)** (`bench/l3/perf/REQ_THREEWAY.md:86-95`) | Whole-array/packet copies within X1/X2, not an extra additive share. Ownership moves and resident buffers; per-record copy savings elsewhere need measurement. |
| C3 budget state re-validation | M8 valid **~70 instructions (~10%)**; M4 **~2ns of 2–3ns gap**, sessions **1.13–1.22x** (`B/M8_NOTES.md:190`; `B/perf_headroom/HOST.md:173-184`) | Already once per root. Existing resident contracts + private adapter eliminate trusted per-call valid, not merely move it to the root. First M4 fix. |
| C4 budget packed argument/result boundary | M8 decode **~68 instructions (~10%)** (`B/M8_NOTES.md:191`); M7 dispatch **~7.6ns includes ~4.6ns rawcall** (see §1.2) | Source-only key layout first, then direct typed operations. Never claim a separate 12ns saving. M7 lookup **2.22–2.84x**, isHalted **2.73–2.76x**, close/reopen **1.61–1.70x** (`B/PERF_HEADROOM.md:36-38`). |
| C2 budget accessor re-check / record materialization | Lookup frame **~80 (~12%)**, record/rest **~130 (~19%)**, including useful work; stubbing the 14-word copy saves **~0** (`B/M8_NOTES.md:192,196`) | Carry owner/generation/span/field invariants into accessors. No separate timing isolates checks; the 210-instruction bin is an upper envelope, not removable cost. Copies are cross-piece targets, but this M8 copy is not the measured bottleneck. |

**REPRESENTATION costs — width and repeated content work on keys/IDs**

| ID / item | Numbers and evidence | Attribution / fix priority |
|---|---|---|
| C1a keyed hash | **~100 instructions (~14%)**, 13 i64 units instead of 13 bytes (`B/M8_NOTES.md:193`) | Selected packed keys with existing stored hash; historical hand uses SipHash/UTF-8, so redo attribution against §4.1, not width-only excess. |
| C1b keyed equality | **~90 instructions (~13%)**, compare **104 vs 13 bytes** (`B/M8_NOTES.md:194`) | Selected packed-content equality after admission; preserve collision resolution and measure against equal UTF-16 semantics. |
| C1c budget ID comparison | **~110 instructions (~16%)**, **288 vs 36 bytes** (`B/M8_NOTES.md:195`) | Selected packed IDs first; intern handles only if later profiling warrants them. No lossy UUID normalization. |
| C5 tool keys/arena footprint | tooldup **1.05/1.63/1.61x**, grow+forget **1.09/1.20/1.23x**; grow **1853–1875 vs 1643–1752 instructions**, larger-size gap **~17–19ns** (`B/M8_NOTES.md:173-181,208-211`) | Width/cache effects not separately timed. Existing hash-once saving (~205 instructions) must not be claimed again. M4 hand uses u16 keys, only 4x narrower (`B/perf_headroom/host/src/bin/m4_hand.rs:20-39`). |

**Residual causes and controls** (kept separate from the two measured cost families):

| ID | Hot item: hand Rust versus generated Rust | Category | Evidence | Estimated share / limitation |
|---|---|---|---|---|
| C6 | M8 charge/failure/tool increment: hand direct += versus safe-integer operand/result checks and atomic refusal | representation choice | `B/bud_charge.pruv:38-47,54-91,97-117`; `B/m8_test/hand_m8.rs:65-95`; `B/M8_NOTES.md:140-141` | Notes estimate ~65 extra charge instructions for safe-number work; ns share unknown. Semantic/work mismatch, not demonstrated codegen defect. |
| C7 | Hot helpers: inlined hand work versus residual generated call frames despite inline hints | compiler codegen | `B/M8_NOTES.md:198-202`; `B/perf_headroom/HOST.md:163-169` | Scratch all-inline(always) saves failure 107, charge 128, tooldup 139 instructions (~15–20%); overlaps C1–C4. Loop-free-only forcing did not help. |
| C8 | M9 refresh 1.05–1.07x, fire ~1.07x: typed clocks versus remaining key/record/root overhead | representation choice | `B/PERF_HEADROOM.md:34-35`; `B/M9_NOTES.md:167-192` | Refresh ~153–155ns vs 143–148ns; ~7–12ns headroom, no causal split. Active counting already exists on both sides. |
| C9 | M5 store/M11 card: preserve successful byte index and single-pass projection/copy | representation choice | `B/host.rs:109-120,190-222`; `B/PERF_HEADROOM.md:26-28,58-61` | M5 find+encode 0.96/1.01/1.04x, save 0.96/1.00/0.99x; M11 ~0.79–0.96x. No new bottleneck attributed; M5 >1.0 rows still need closure. |
| C10 | M6 controls: stack/single-allocation hand output versus remaining allocation/total-access overhead | representation choice | `B/perf_headroom/COMPILER.md:14-28,45-66` | p_int 1.03–1.09x; p_wr up to 1.08x. Removing total guards gave no measured simple-loop win; dependent accesses can differ. |

**Budget re-ranking:** P01 compares boundary removal and key layouts against equal-semantics hand Rust; P03–P05 implement the selected **source-only key layout before language work**. M8's ~300 width-associated instructions exceed valid+decode's 138, but are not 300 removable instructions: the 288-byte budget-ID memcmp was **749 versus 694 instructions**, worse than the existing loop (B/M8_NOTES.md:185-202). Withdraw the old 100–220-instruction saving estimate. P01 compares F1, four UTF-16 units per i64, and eight ASCII bytes per i64 (lossless fallback), including ingestion and canonicality cost. Preserve existing stored hashes/hash-once work; interning is conditional on repeated-hashing evidence.

Boundary removal alone is not a parity plan. The review's residual M7 ~1.7–2.0x is a rough subtraction estimate from historical probes, **not a matched ablation result**; §1.2 must measure it. After the key slice, remove C3 through the existing trusted resident pattern, then C4 direct calls and measured I1 accessor work (C2). Re-rank residual C6/C7/C8/C10 from profiles, without making full opaque types or L2 prerequisites. Keep flat tables where they win; compact budget keys are required, F1 specifically is not. No blanket byte recoding of payloads follows from C1.

### 1.2 Correct boundary experiment

`World::call` calls t_call at `B/m7_test/driver_m7.rs:130`. Dispatch measures that wrapper (340–361), including valid/key_end; rawcall measures the prepared inner call (362–370). Thus `B/PERF_HEADROOM.md:68-72` double-counts when adding 7.6+4.6 and then validation. Even 7.6−4.6 is not clean wrapper cost: UUID/quiet inputs and warm-up differ (121–124,329,365–367). key_end validates lengths, not every unit (`B/m7_test/root.pruv:20-28,67-73`).

Measure matched A=wrapper+packed root, B=identical prepared packed root, C=direct typed root for each real opcode, plus matched no-ops, preparation and observation separately. Match state, UUID/quiet, warm-up, black-box inputs, full result consumption and per-call normalization. Alternate order for >=7 rounds; record median/spread, ns/instructions, allocation and effects. A−B estimates wrapper cost; B−C includes removed packed checks. Validation-only ablation is separate, never added again. Compare direct typed APIs with hand typed APIs, and legacy adapters with hand accepting identical packets. Include ingestion, host encoding plus Pruv canonicality checks (two O(K) passes where needed), and any optional interning in end-to-end time. No >1.1 adapter is excused by a fast core.

### 1.3 Cross-piece causes and priority

Order is **budget source-key slice → mcp source-only linearisation → budget boundary/operations → entry/lib → json/pkgs → mcp remaining work → qwen → grok → opencode → claude → codex → supervisor → req after its synchronous-host gate**. It combines measured severity with shared dependencies and unmeasured structural risks; it is not a fabricated numeric ranking of pieces with no performance baseline. Shared language/package prerequisites may land earlier, but every consumer is reworked in the single batch. Paths in this table are repo-relative.

| Priority / piece | Measurement and source | Cause / framework application |
|---|---|---|
| 1 budget M4/M7/M8 | Slow rows roughly **1.1–2.8x hand Rust** (M4 1.13–1.22; M7 up to 2.84; M8 charge 2.11–2.43), not every operation (`B/PERF_HEADROOM.md:22-43`) | C1/C5 source layout first, then C3/C4 and measured I1 work under §1.1. Preserve exact numerical behavior. |
| 2 entry and lib/ent_* | No hand-Rust ratio. Existing writeback table fix: **2.6s → 0.13s at 20000**, already achieved (`bench/l3/entry/IMPL_NOTES.md:712-729`) | Every replay revalidates original input and entire answer log, appends/copies log and re-packs state (`bench/l3/lib/entry_api.pruv:40-67`; `bench/l3/lib/ent_state.pruv:64-124,163-195,489-502`). G1 resident progress (continuation only at a genuine async boundary), G2 local records, G3 measured copy removal, G4 source-byte spans. Do not re-credit existing dedup/index fixes. |
| 3a json | Plain 8MiB: rs/rust **17.040084ms / Node 2.710833ms = 6.29x (~6.3x)** (`bench/l3/json/measure.json:39-72`); median 3, no warmup, unequal representation/API work (:2). Other cells differ, e.g. 100k-object parse is faster than Node (:145-212). | Source/tape copies at wrap and output, publication re-check, accessor invariant loss (`bench/l3/pkgs/json/json.pruv:67-76`; `bench/l3/pkgs/parse/parse.pruv:240-267`; `bench/l3/json/a2_root.pruv:5-9`). G2/G3/G4. Individual contributions to 6.3x are not isolated. |
| 3b pkgs | Shares json implementation; **no independent package-wide 6.3x measurement** (`bench/l3/pkgs/README.md:3-10,26-30`) | Repeated valid/span/parent/skip checks in access (`bench/l3/pkgs/access/access.pruv:7-47,53-99`); carry verified document/NodeId facts. G2/G3/G4 benefit every package consumer; keep lib compatibility row too. |
| early source / 4 remaining mcp | No matched latency/hand baseline; Q wall time is explicitly not throughput (`bench/l3/mcp/Q_README.md:34-37`) | Per-tool copy(input) and full-prefix append can make packet copying quadratic in accepted tool count (`bench/l3/mcp/packet.pruv:2-8,34-61`). Repeated array_at also rescans (`bench/l3/mcp/root.pruv:132-142`; `bench/l3/pkgs/access/access.pruv:40-48`). First pre-size output and write via existing inout buffers (P05M), independent of G2/G3; later G2 cursor and G4 byte result. This is structural analysis, not a measured scaling curve. |
| 5 qwen | 2/4/8MiB: rs/llvm **5.23/9.31/13.43ms** vs TS **5.69/8.73/16.66ms**, **0.92/1.07/0.81x** (`docs/QWEN_IMPORT_SPEC.md:144-159`) | Prior quadratic chain search already fixed by hash indexing; remaining dense/collision/admission costs need profiling. Cold single-call Pruv window vs TS file/head IO is not matched parity. Reuse G2 records, G3 move and G4 bytes around existing index. |
| 6 grok | No matched runtime ratio; dense 8MiB window event array was ~192MiB, now **~96MiB**, already fixed partly (`bench/l3/grok/IMPL_NOTES.md:237-240`) | Two-pass framing allocates 3 i64 words/event (`bench/l3/grok/framing.pruv:190-237`); profile spans/copy volume/peak memory with G1–G4. Q's 672.5s is stall-dominated suite wall time (`RESULTS.md:2218`), not runtime ratio. |
| 7 opencode | No matched runtime ratio; Q **555s** is suite wall time (`RESULTS.md:2272-2273`) | Initial env uniqueness remains O(E²); later steps redo O(E) geometry checks (`bench/l3/opencode/root.pruv:68-99,336,357`). G1/G2 carry validated config, G3/G4 output views. Spawn pool's repeated growing copy is already removed (`bench/l3/opencode/export.pruv:276-312`). |
| 8 claude | Latest small **0.060ms vs hand 0.02ms (~3x)**; 8MiB **47.4ms**, ring **2.35ms** (`RESULTS.md:1811-1812`). Mixed historical runs, not a complete four-pair parity matrix. | Small-input allocation remains a measured priority within this piece; initial 16-slot ~1MiB ring grows to 200 slots (`bench/l3/claude/turns.pruv:45-53`). G2/G3/G4; profile residual checks/copies rather than re-credit old clone fixes. |
| 9 codex | No runtime ratio: TS 55.18s vs Pruv 56.88–57.33s Q wall times are timeout-dominated (`bench/l3/codex/Q_README.md:51-52`) | Fixed 8MiB line + 64KiB chunk + 8.7MB held arena and replay-to-packet copies are profiling candidates (`bench/l3/codex/root.pruv:110-116`; `bench/l3/codex/read.pruv:251-263`). G1–G4; no causal timing attribution yet. |
| 10 supervisor | 64MiB/64KiB chunks: **0.311s / 0.280s = 1.11x TS**, rs/llvm 0.313s (`bench/l3/supervisor/results/measure.json:2-49`) | Harness hashing/startup asymmetry prevents a pure core attribution (`RESULTS.md:2050`). G1 ring state, G2 spans, G3 chunk transfer, G4 bytes; retain deadlines and child protocol checks. No hand baseline yet. |
| 11 req (blocked on host gate) | S=20000, M=10: **18.5x TS/Node**, **11.3x without digest/check**, 420k calls (`bench/l3/perf/REQ_THREEWAY.md:38-46`); Leg D **0.63–0.68x hand Rust** (:48-60) | Per session × item world seal/digest comparison and round trip, plus re-pack/copy; `bench/l3/req/root.pruv:1018-1035`, `bench/l3/req/req_world.pruv:81-123`. Synchronous trusted host-fact calls are the primary candidate (§3.2); full req parity is blocked until their complete-operation cost and fresh-query semantics are measured. Source kernel is already fast. |

## 2. Shared contracts, with budget as the worked example

**Invariants that must survive every piece:** M1/M2 equivalence requirements, including lone-surrogate preservation, exact UTF-16 semantics, optional-value distinctions, errors and ordered effects, stay. Validation of external input stays: raw bytes, snapshots, host answers and untrusted IDs must be admitted. Remove only internal re-encoding/copies and re-checks of facts already established by proof and preserved by ownership. A parse-success tag or cached valid bit alone is not a proof. No optimization licenses loss of semantics or treating unsupported paths as passes.

### 2.1 Key representations and payload bytes (F1)

**F1 is a candidate for budget keys/IDs only.** It is M5's existing encoding (B/host.rs:109-120): all-ASCII UTF-16 units become bytes; otherwise FF followed by every original unit in UTF-16LE. Empty is empty bytes; non-ASCII form must contain a unit >=128. Reject noncanonical external encodings. Preserve NUL, astral pairs and lone surrogates without normalization. Store key offset, encoded length, unit length and tag; equality is canonical-content equality, never hash equality. F1 byte order is not UTF-16 lexical order; use a unit comparator where ordering is specified.

P01 compares the current layout with three source-expressible alternatives before selecting one for P03–P05:

| Candidate | Storage and semantics | Required ablation |
|---|---|---|
| F1 | U ASCII bytes, otherwise 1+2U bytes; tag and cached unit length | Host encode plus Pruv canonicality validation are two O(K) passes, charged to complete operations; report retained-key lookup separately. |
| Packed-u16 | Four UTF-16 units per i64, big-endian lane order, explicit unit length and zero tail padding; no variable text codec | Preserve surrogates and lexical unit order with **unsigned** word comparison (or sign-bit bias), then length on equal prefixes; signed i64 or native-endian memcmp is not the comparator. Validate imported units/padding. |
| ASCII-8 | Eight ASCII bytes per i64 with explicit length/tag and packed-u16 fallback for all other units | Isolate storage width from hash/call/loop changes; no non-ASCII truncation. Include form transitions, NUL and tails. |

Existing pack4 in B/bud_map.pruv:43-84 packs only for hashing, not storage. Keep the existing stored hash (entry field 6, :8-9) and hash-once API as the starting point. Do not claim these savings again. KeyId interning with owner/generation/cached hash is deferred until profiles show repeated hashing remains material; account for admission, extra state, churn, stale IDs and generation exhaustion if enabled. UUID fast comparison requires exact spelling/shape; arbitrary, empty and surrogate IDs remain supported and differently cased strings stay distinct.

**G4 payloads do not use F1.** json, mcp, claude, codex and supervisor retain source-byte spans in UTF-8, with cached UTF-16 unit length where semantics require it. Escaped JSON strings can retain source spans; when decoded materialization must preserve lone surrogates, use tagged WTF-8 and a verified unit iterator, only for those escaped strings. Never replace a lone surrogate or pretend WTF-8 is valid UTF-8 str. MCP raw schemas/stderr remain UTF-8; decoded name/title/description cuts retain the surrogate-pair rule (bench/l3/mcp/packet.pruv:1,24-30,40-47). Cache lengths at admission/decoding, not by rescanning each access. G4 removes widening/internal re-encoding, not necessary external decoding or validation.

JV/persistence/final observation retain exact UTF-16 values (B/compare.py:62-69; B/Q_README.md:35-40). Card validation retains ECMAScript trim, <=300 UTF-16 units and raw note length (B/bud_card.pruv:184-247). Use unit iteration at semantic edges, not byte/scalar length. Maps preserve insertion order. Localized/date/number output follows its existing output contract, never the key codec by default (docs/REQUEST_BUDGET_SPEC.md:670).

The implementation bundle must add this behavior ledger to REQUEST_BUDGET_SPEC (not edited by this task):

| Decision | intent / bug / accident | Comparison / known difference |
|---|---|---|
| Lossless key packing and source-byte payload spans, insertion order | intent-preserving | No new KD; unit equality, lone surrogates, card length and exact text stay. |
| One now/public call; follows present at changed | previously chosen accidents | Retain KD-1/KD-2 in §15; judge separately from normalized comparison. |
| ResourceBound(number) outside ±(2^53−1), including increments/negative charges | intentional Pruv bound; JS rounding deliberately not copied | Add KD-R1/P6 cross-reference to §8.4 (docs/REQUEST_BUDGET_SPEC.md:874-878); refuse before writes, not an exact oracle success. |
| UTF-16 card length and raw note behavior | retained accident, §15 row 19 | No byte/scalar limit substitution. |
| Malformed old state/packets | test representation | Keep legacy rejection; safe product roots cannot receive forged backing words. |
| Query executed synchronously at its semantic position | intended ordered observation | Replace prefetch dependency of KD-M10-1 in the future implementation/spec bundle; archived codec remains test-only (§2.4). |

C6 remains a behavior difference, not permission to drop checks: safe operands do not prove safe sums. Equal-semantics hand rows must match operand/result validation, refusal precedence, atomicity and UTF-16/surrogate behavior (§4.1).

### 2.2 Records: now and target (F2)

Use domain records: Session (id, budget slot/generation), Budget (id/state/limits/used/flags and halt/ended/card/auto/follows references), Clock (budget/owner, since/timer presence+values, ordered run links/count, active_count), Run (key/waiting/links), Turn (key/activity links/count). Variable text/lists are arena references, not JSON tails. Preserve observed absent/undefined/null distinctions with tags; unvalidated card/JV values use a typed tagged-value arena. current returns stable logical identity through a generation-checked BudgetHandle; snapshots are materialized only when needed.

**Today:** use scalar-only structs as local views/index wrappers and internal parallel arrays for columns, live bits, generations, links and bytes. Each mutable array is a distinct whole inout binding; validate aligned lengths at admission. Vec<plain struct> works for local ownership but has no general resident mutable-row API. Do not assume [Struct], collection fields, Vec<buffer-bearing struct> or aggregate/field inout. Non-generic str/buffer fields already work, but are read-only views and owning copies can deep-copy. Source-only packing can retain the current flat store. Do not expose SoA columns as entry parameters: the LLVM wrapper checks pointer overlap in O(k²). Internal SoA is optional, not P05 scope.

**Deferred target if measured C2 needs it:** private BudgetStore owns typed tables/arenas with exclusive borrowing and disjoint field views (extended L1/L2). First use the existing private resident adapter and flat storage. Lower hot records to Rust structs/tables only when profiles justify it. Choose records for updates and SoA for scans using cache data. References carry store identity, slot and generation; scalar wrappers alone remain forgeable. Growth preserves slots, deletion invalidates handles, generation exhaustion refuses rather than wrapping/ABA. No borrowed pointer survives mutation/growth; retained logical handles resolve again.

### 2.3 Typed resident state and proof ownership (F3)

Start with docs/RESIDENT_STATE_SPEC.md §4.2/§4.5 (:94-103,159-172): state requires/ensures carried by constructor/grow/mutators, --trust-callers behind a private Rust adapter, and executable validation of **new external inputs on every call**. Expand fixed-cost header predicates into contracts because ordinary calls are forbidden there (RC/check.rs:3609-3615). Add a codegen entry-visibility option: raw trusted entries are pub(crate) behind the adapter or explicit unsafe fn with documented preconditions, never public safe arbitrary-Vec entry points. Full language opaque types wait for measured C2 need.

| Tier | Facts and admission | What it permits |
|---|---|---|
| I0 | Header geometry, lengths, capacities and header range facts from current O(1) valid (B/bud_store.pruv:57-82); constructor/import/grow establish and mutators preserve expanded contracts | Remove per-root valid only. I0 alone proves neither key spans nor dependent field accesses. |
| I1 | Per-entry offset/span/link bounds, legal tags/unit encodings and owner/generation facts used by a particular unchecked access; validate at import in a linear entry pass and prove preservation for each optimized path | Remove only dependent guards with their own obligations discharged; dynamic stale-handle/input checks remain. Keep total accessors elsewhere. |
| I2 | Acyclic/order consistency, index completeness, free/live logical disjointness, active_count equals nonwaiting runs and other functional properties | Functional regression/model/mutation tests, **not a trusted-entry or memory-safety gate**. Do not assume these facts for unchecked lowering or remove step budgets on their account. |

Import validates I0+I1 and retains existing external-format/semantic rejection rules; it does not require a new global I2 graph proof. Accessors are total (B/bud_store.pruv:87-115) and walks/probes are step-budgeted (B/bud_map.pruv:13-14), so memory safety/termination need not assume acyclicity or index completeness. Mutator proofs cover I0/I1 facts actually used on every normal exit/refusal/grow, with alias/frame obligations; functional mistakes remain test failures.

**Panic safety:** a private poison word is checked once at call entry, set before any trusted operation (including host callbacks), and cleared only on a proven normal exit preserving the required tier. Panic/unwind or host failure leaves it set; subsequent calls, including observation, refuse without touching trusted arrays. Drop releases owned buffers without traversing logical links. Do not clear poison in an unwinding drop guard or let catch_unwind reuse half-mutated state. C/LLVM wrappers contain unwinds and return terminal failure with poison retained, or abort; no unwind crosses C ABI. Fault-injection coverage includes partial writes, allocation and host callbacks. Measure the poison check; removing valid does not imply a free boundary.

C/LLVM product ABI uses a **single-pointer opaque store handle**, with unsafe preconditions of provenance, lifetime, serialization and no reentry. There is no per-call registry lookup. Safe Rust owns the private box and borrows it exclusively; foreign callers unable to uphold this raw contract need a separate validated ingress. Never convert untrusted integers into pointers. No public SoA-array entry signatures; the adapter supplies internal disjoint buffers with a fixed small pointer count. Existing raw LLVM overlap checks remain where applicable (docs/RESIDENT_STATE_SPEC.md:126-142); measure wrapper, poison, foreign shim and call costs. A registry might consume the same order of budget as the ~2ns valid removed, but this is a concern to measure, not a measured registry result (B/perf_headroom/HOST.md:148-149).

Reserve before writes/effects. Grow before effects is mechanical and preserves owned state; allocation failure keeps the old valid state or a terminal poisoned store if unwinding occurred. After an observation/effect, allocation must continue in-call without replay, or end terminally; do not restart the operation. Report import/grow latency and copied bytes separately. Geometric growth and cached hashes preserve amortized bounds.

### 2.4 Direct typed operations/results (F4)

Target entries: current(store, Ref)->OptionalBudgetHandle; is_halted(store, Ref)->bool; charge(store, Ref, ChargeArgs)->ChargeResult; note_failure(store, Ref)->Status. These are design notation. Start with scalar records/tags and existing bounded output buffers behind the adapter; aggregate out-params are deferred L2, not a prerequisite. Direct per-operation bindings and test-only packet shims call the same Pruv decision engine. Keep logical identity and external argument validation.

**Primary host path: synchronous trusted host-fact functions.** Budget methods are synchronous (docs/REQUEST_BUDGET_SPEC.md:20,58,742). G6 provides trusted rust fn (docs/G6_SPEC.md:11,18-19), already used for a synchronous query in bench/l3/mcp/root.pruv:10-12. Pruv invokes isQuiet/runningDelegationIds/releaseHeld at the reached semantic point, validates returned values in executable code, and retains every decision. Host code is trusted for memory safety, effects and ABI, not for bypassing result validation. Do not give it a mutable Store alias; same-store reentry is forbidden. Establish each required concrete signature on both backends; the example alone does not prove all budget/req signatures or costs.

For halt, trace **SETTLE (including any timerClear) → query runningDelegationIds(arguments, validated answer) → halted write → subsequent effects**. Log the query at actual invocation, with ordered arguments, answer and between-request events. fx-30 is only an archived prefetch/consumed-marker adapter (B/M10_NOTES.md:40-58), not proof of live query timing. Retire product prefetch and its KD-M10-1 assumption only after the synchronous lane matches observations. Add a mutation test moving the query before SETTLE: a state-sensitive host answer/trace must make it fail. Apply the same rule to isQuiet and releaseHeld; no trace normalization may hide ordering.

**Need/Resume is conditional, only for a host proved asynchronous.** Such a host needs a separate affine-continuation, effect-once and saved-fact validity/serialization design with its own tests/costs before use. No continuation state, pending-operation lockout or new timer queue is imposed on synchronous budget calls. Existing callback ordering stays run-to-completion. If an asynchronous req host cannot supply equivalent synchronous observations, its parity row stays blocked; an async fallback is not assumed to meet the floor.

### 2.5 Every flow/op and complexity

S=sessions, M=public calls (old spec N), C=clocks, R=runs of addressed clock, A=all activities, At=activities of one turn, D=delegation IDs, E=live entries, K=examined key bytes, U=semantically examined UTF-16 units, V=inspected/copied value bytes, O=output. H(k)=O(len key) hash + O(1) expected probe at bounded load; conservative collision worst case O(table capacity × maximum compared key length). Retained entry handles avoid hashing only after admission; a new key interner is not assumed. Add actual output and amortized allocation to all rows; no hidden full-world observation per call.

| Flow/op (codes `B/bud_api.pruv:33-63`) | New bound; perf must confirm |
|---|---|
| constructor/ingest/import | Expected O(S+E+K+V) with indexed construction, validation once; collision worst-case/fallback policy applies here too |
| current (1) | H(session)+O(budget ID bytes); O(1) admitted same-store handles |
| isHalted (2) | current+O(1) |
| attach/OPEN (3) | H(session)+new record bytes+DROP(old clock) if present; existing-budget no-op constant after lookup |
| replaceForUserMessage (4) | current+CLOSE(old)+OPEN+follows/carry output |
| reopen (5) | current+O(1), no record rewrite |
| charge (6), success/no-op | current+O(1), five counters; rejection adds halt |
| noteFailure (7) | current+O(1) |
| noteTool (8), duplicate | H(turn)+H(activity), no current lookup/A scan |
| noteTool (8), new | duplicate bound+current+inserted key bytes |
| forgetTurn (9) | H(turn)+O(At+reclaimed bytes); time deletion separately |
| runStarted (10) | H(budget)+H(run)+current+REFRESH+new key bytes |
| runWaiting (11) | H(budget)+H(run); changed waiting adds current+REFRESH |
| runEnded (12) | H(budget)+H(run); successful deletion adds current+REFRESH |
| halt (13) | current+SETTLE+O(D+delegation bytes+required snapshot bytes), exactly D ordered stops |
| maybeClose (14) | H(session)+H(clock)+O(1) rejection; successful quiet path adds CLOSE, query only if reached |
| raise (15) | current+O(1) limits; time change adds SETTLE+REFRESH and reached FIRE/halt |
| decide (16), extend | current+O(1) additions+REFRESH+reached FIRE/halt+required save/effects |
| decide (16), end | current+CLOSE+effects |
| setCard (17) | current+O(V+U) validation/projection; bound failures short-circuit |
| hasCard (18) | current+O(1) presence/truthiness, no validation |
| clearAutoContinue (19) | current+O(1) unlink, reclaim charged by bytes |
| recordAutoContinue (20) | current+O(V) admission/copy, O(1) admitted binding |
| stopAutoContinue (21) | bounded owner/current lookups+key comparisons+required save bytes, no S scan |
| bootRestore (22) | H(session)+DROP(clock)+O(1), no settlement |
| forgetSession (23) | O(C+owner comparison bytes+removed runs/bytes), clock order preserved |
| dispose (24) | O(C+R_total+A+reclaimed bytes), sessions unchanged |
| fire (25) | current+SETTLE+O(1) rearm, or halt |
| limitText (30) | O(formatted output) |
| refusalWire (31) | O(1) numeric fields+output |
| refusal (32)/continueText (33) | O(formatted output) |
| cardMissing (34) | O(output), constant literal selection |
| budgetUnitOf (35) | O(1) fixed projection |
| amountText/integer/ISO | O(output), bounded stack result where proved |
| REFRESH | O(1) after current/clock lookup from active_count, not O(R) |
| BEGIN/ARM/SETTLE | O(1) after lookup+timestamp/effects; ARM can invoke FIRE |
| DROP/CLOSE | O(R+reclaimed bytes), CLOSE adds SETTLE/timestamp |
| map/session/clock/run/tool get/set/delete | H(key)+copied blob bytes; owner deletion adds descendants |
| ordered list/final observe | O(entries visited+K+O), once finally unless requested |
| reserve/rehash/grow | O(new capacity+live bytes) worst case; geometric growth/cached hashes amortize to O(1) per inserted byte/entry, excluding hostile collisions |
| test decode/admission/host query | O(packet bytes+values); include host encoding and validation passes and each synchronous query. Async resume requires a host-specific design. |
| M-call script | sum of above+ingestion+final observation; no M×S, M×A or D² |

This corrects optimistic old DROP/bootRestore bounds: freeing R runs is not constant. Active counting already exists (`B/bud_clock.pruv:230-245,351-399`), so constant refresh is preservation. Vary S at fixed M, M at fixed S, plus R/A/D/C, key length/Unicode width, churn and collisions. External service scans are excluded from core but named in end-to-end latency (`docs/REQUEST_BUDGET_SPEC.md:926`).

## 3. Language/compiler changes and complete coverage

### 3.1 Facts and changes

Non-generic str/buffer fields work (`tests/g9/frontend/a18_buffer_struct_u8_clone.pruv`; `pruvc/check.py:475-491`). Option<str> fails (`pruvc/check.py:442-451`); a dedicated nongeneric enum can hold str but is buffer-bearing. HashMap keys are i64/bool (`pruvc/check.py:394-399`; `RC/check.rs:1011-1019`). Collection fields fail (`pruvc/check.py:488-491`). Vec<plain struct> works (`tests/g6/frontend/a21_vec_of_struct.pruv:7-19`), buffer-bearing elements fail (`pruvc/check.py:632-647`). Aggregate inout fails (666–673), but root pub array inout works (675–685; `docs/RESIDENT_STATE_SPEC.md:63-78`) despite stale `SPEC.md:473-479`. No scalar u8/[Struct] is assumed. Whole-string == is currently rejected, including in contracts; eq_range is also not currently callable from contracts (RC/check.rs:3418-3434).

Every generated Rust function already gets #[inline] (`RC/codegen.rs:1651-1652`), verified indexing get_unchecked (3798–3817), eq_range slice equality (3972–3990). eq_range proof result is fresh/unconstrained (`pruvc/verify.py:1822-1829`; `RP/symbolic.rs:2183-2202`); **LLVM currently rejects it** (`RC/llvmgen.rs:3660-3661`; `SPEC.md:120`). Ordinary functions cannot be called in contracts (`RC/check.rs:3609-3615`): reusable invariant predicates need a feature or generated contract expansion, not just requires valid_state(st).

| ID/owner | Change and necessity | Expected gain tied to causes (hypotheses) | Proof effects |
|---|---|---|---|
| L1 resident contracts/ABI | Reuse resident requires/ensures with header contract expansion and raw-entry visibility flag; private adapter + poison. Full language opaque types deferred to measured C2 need | C3 ~70 historical instructions/~2ns; subtract new poison/ABI overhead, parity only | Both checkers/verifiers prove I0 and each used I1 contract. Both emitters restrict raw trusted entry access. No reliance on I2 or public safe arbitrary buffers. |
| L2 deferred ownership/lowering | Disjoint aggregate/field inout, typed out-params, resident collections only if a profile shows need | No assumed M8 gain: record-copy stub saved ~0 (B/M8_NOTES.md:196); M4 tools/clocks already near/below parity in many rows | Field-sensitive state, alias exclusion, moves/drop, frames/capacity and backend agreement need separate gates. Current mutable-call proof forgets contents except length (pruvc/verify.py:1958-1971; RP/symbolic.rs:2610-2648). Not on critical path. |
| L3 equality/library/backend | Content-equivalent eq_range and LLVM byte equality or verified loop lowering | No 100–220-instruction promise; P01 determines layout benefit. LLVM support blocks four-pair completion | Both verifiers relate results to bounded equality incl. empty/alias/old arrays; preserve bounds, reject false contracts, measure solver cost. |
| L4 codegen | Source inline attribute or simple size heuristic for hot calls, no profile-guided specialization machinery | C7 historical -107/-128/-139 instructions overlap other causes; residual gain unknown | Preserve call/effect semantics; both emitters, compile CPU and binary-size gate including M11. |
| L5 source/verifier | Local range/tag/frame contracts and sum reuse, proof-guided removal of guards | Residual C2/C6/C8/C10 only; no fixed 10–40-instruction saving until disjoint checks identified | Safe operands do not prove JS-safe sums. Keep result checks and refusal order; never assume tested I2 properties as proof facts. |
| L6 optional scalar/library | u8/u32 storage/casts and bounded byte helpers | No required gain; source packing first | Both checkers/emitters/verifiers model ranges/casts/overflow; no narrowing arbitrary counts/IDs or wrapping generations. |

P01/P03–P05 source layout comes first, then minimum L1 resident contracts and direct F4 calls. Full opaque types and L2 are conditional follow-ups, not prerequisites for C3. Whole-string equality, string HashMap keys and Option<str> are not needed for packed arenas. L3 can use explicit loops/lemmas if quantifiers are costly. Any optional work promoted by measurement gets explicit additional slices and gates in §6.

### 3.2 Four cross-piece language/ABI gaps

These are shared facilities, not budget-only special cases. Reuse existing source/ABI facilities first. Any necessary new compiler facility must pass both compilers/backends before its check elimination is enabled; deferred L2/full opaque types are not prerequisites for all consumers. L1–L6 identify the possible implementation owners.

| Gap | What changes / why | Expected gain (unverified unless historical) | Proof impact |
|---|---|---|---|
| G1 resident state (L1, F3/F4) | Existing trusted resident pattern and private adapter, poison and visibility; host owns memory, Pruv owns decisions. Optional opaque language types later | Remove C3 header recheck after net poison/ABI measurement; reduce seals/packet materialization, not semantic host queries | I0 and needed I1 preserved on normal exits; panic prevents reuse. Async continuation only with a proven async host. |
| G2 invariants across functions (L1/L5) | Admitted document/NodeId/record handles carry the local span/kind/owner facts needed by accessors | Remove identified redundant publication/access checks; no isolated ns claim yet | Tie handles to document lifetime/generation, prove each removed guard, retain dynamic wrong-kind/missing-key and stale/external-ID checks. No blanket I2 gate. |
| G3 ownership transfer (existing facilities first; L2 conditional) | Use existing inout/pre-sized buffers or consuming returns where supported; extended moves only when copy profiles justify them | One allocation/owned result and fewer payload copies; M8 copy-only ablation ~0 | Prove initialized prefixes, no use-after-move/double drop and allocation-failure ownership. MCP linearisation does not depend on new G3. |
| G4 source-byte ABI (existing bytes first; L3/L6 or L2 only if needed) | Borrowed/owned source [u8] spans with length/capacity/destructor contract; metadata may stay i64. No payload F1 | Avoid byte-to-i64 widening and repeated internal encoding; no blanket 8x runtime gain | UTF-8 raw payloads, tagged WTF-8 only for materialized escaped strings needing it; cached UTF-16 length and semantic unit iteration (§2.1). Verify ownership/bounds and all four pairs. |

G1 alone does not remove req's **420k semantic host queries at S=20000,M=10** (bench/l3/perf/REQ_THREEWAY.md:41). docs/ENGINE_IMPORT_REQUEST_SPEC.md:108-121,138-146 requires fresh per-item session observations, ordered isTemporary and first-match termination. Never reuse a cross-item snapshot or query later sessions speculatively.

The primary parity mechanism is **in-process synchronous trusted host-fact calls from Pruv**, using G6 and a mechanical host adapter, with executable result validation and no world packet/seal per query. At 2µs per query, 420k calls cost **0.84s**, far above millisecond-scale hand work; 2µs is not an acceptable parity budget. P10H measures actual per-query and total costs including allocation, answer admission and tracing on both backends, against the equal-semantics hand request engine. Required incremental call budget is (hand complete-operation time minus measured non-call work)/actual query count; if nonpositive or not met, continue redesign rather than claim parity. Until both timing and live fresh-observation semantics pass, **req hand-Rust parity is blocked** and req stays off the queue head. Host signatures, allocation and live async constraints remain unverified. Need/Resume is only an async-host contingency under §2.4, not an assumed performance solution.

### 3.3 Cause-to-fix coverage

| Cause | §2/§3 fix | Closure evidence |
|---|---|---|
| C1 | F1 selected packed keys, optional later interning, F2 IDs, L3 equality/LLVM, optional L6 | M7 lookup/M8 charge/failure <=1.0, ASCII/surrogate Q, bytes/instructions |
| C2 | F2 existing layout, F3 I1, L5; L2 conditional | M4 sessions/M7 close <=1.0; only locally proved I1 guards removed, other total paths retained |
| C3 | F3+L1 | No valid in direct roots; constructor/import/grow proofs, boundary ablation |
| C4 | F4 existing buffers/direct calls; L2 conditional | Matched A/B/C; direct and legacy adapter <=1.0 respective same-work baselines |
| C5 | F1 existing stored hash/hash-once, F2 storage, optional L6 | dup/new/forget individually <=1.0; churn/collision/amortized data |
| C6 | F1 ledger, F2 validated numeric fields, L5 | Atomic boundary tests and equal safe-number work, charge/failure <=1.0 |
| C7 | F4 fewer frames+L4 | Residual assembly/profile, compile/code-size data, M11 no regression |
| C8 | F1–F4+L5 | refresh/cycle/fire <=1.0 over R/S; same effects |
| C9 | M5 existing codec+F2/F4 projection/observation | M5 encode included <=1.0; M11 A/B/C/error <=1.0 |
| C10 | G4 source bytes+F4 outputs+L5 sized/stack helper contracts | helper suite <=1.0 or continue profiling; total guard removal needs proof |

Estimates do not prove parity. Profile remaining excess after each phase. Every new >1.0 cause gets a row and §2/§3 fix; an explanation alone does not close it.

## 4. Beyond hand Rust

### 4.1 Baselines and goals

(i) The floor is **<=1.0x natural safe idiomatic hand Rust with equal semantics for each row**: typed records, ordinary slices/maps, equivalent input domains, admission, output ownership, effects and numerical refusal. Do not make Rust copy state or parse needless packets; keep its active-count optimization. Match raw/retained-key lifetimes and full output consumption.

Historical M8 uses UTF-8 String keys and lacks the same safe-integer refusal (B/m8_test/hand_m8.rs:17-46,65-95); keep it as informational, not the surrogate/numeric parity bar. P01 authors equal-semantics UTF-16-capable safe hand rows and redoes C1 attribution against them. Claude uses serde_json but also a **lone-surrogate-to-U+FFFD fallback**, rather than simply rejecting all such lines (bench/l3/claude/handrust/src/lib.rs:328-386,437-460). Audit observable equivalence per row; it cannot stand in for preserved unit identity wherever the contract requires that identity. Corrected baselines must preserve lone surrogates, exact refusals and optional values before their timings count.

(ii) Keep a separately labeled **hand-optimized Rust comparator with the same selected data structure**: packed keys, stored/cached hashes, dense IDs/interning, UUID specialization or compact fields as applicable. Report safe and any unsafe variants, hash threat model, versions and unsafe preconditions. This controls data-structure advantages; winning (i) does not imply winning (ii).

Stage one closes every operation/size at <=1.0x (i), with uncertainty, both compilers/backends after bring-up. Stage two must beat Rust on selected useful complete flows, with numerical goals below; the goals are hypotheses, not evidence. M10 needs a hand/TS baseline before any claim (B/M10_NOTES.md:81-91). No average hides an unfinished >1.0 row or >1.1 regression.

### 4.2 Proven facts that can buy speed

**Allocate each saving once.** Keep a ledger of check/copy/call identity, before/after instructions, workload and owner. C3 valid removal and C4 decoding pay for parity only. C2/L5 removal on one instruction site belongs to either parity or incremental stage two, never both. Measure stage-two deltas against the completed parity version; historical inline/hash-once wins are not new credits. A proof-speed claim needs an assembly/instruction diff identifying a check still present in equal-semantics hand Rust and absent in Pruv, with equal data structures/work.

| Opportunity | Classification and owner | Target and evidence |
|---|---|---|
| Header valid and dependent budget guards | L1/I0 and L5/I1 parity repair | <=1.0x; withdraw M4 sessions <=0.95x as a proof-win claim. Hand HashMap::get_mut/struct fields have no corresponding header/field bounds checks (B/M8_NOTES.md:188-196). |
| Budget arithmetic/sum reuse and clock branches | Local L5 residual parity work; keep safe-number refusal and tested active_count semantics | <=1.0x; no charge/failure/refresh <1.0 proof target without an identified surviving hand check. No proof assumption from I2 tests. |
| Dense IDs/interning | Conditional data-structure win, only after repeated-hash profile | Exploratory admitted current/isHalted <=0.80x (i), raw complete <=0.95x (i); also compare against (ii) with the same structure, aiming <=1.0x. No guaranteed new interner or proof credit. |
| Stored hash/packed key/UUID comparison | Source data-structure win; existing hash-once savings excluded | Exploratory tooldup <=0.90x and toolgrow+forget <=0.95x (i); publish same-layout (ii) comparison too. Raw external key remains O(K), admission charged. |
| Compact metadata | Optional L6 data-structure/cache experiment | Large toolgrow <=0.90x (i) only if measured; compare (ii); no assumed gain or undocumented narrowing. |
| Bounded output | Source initialized-prefix/size facts, existing small-result lowering | amountText <=0.85x (i) exploratory allocation target with equal ownership; classify as allocation/layout benefit unless hand check removal is shown. |
| Card unit iteration | Local decoded-unit/span facts | Selected validation/projection <=0.95x (i), **conditional proof target** only if assembly shows residual hand bounds/decoder checks removed by proof; retain trim/raw-note/number semantics and charge decoding. |

Hash threat model: most budget keys are internal UUIDs, engine IDs or local snapshots; external/imported keys still require validation and resource bounds. Default to a **per-store random seed mixed into existing mix/fin**, not a mandatory strong keyed hash for all keys. Seeded mix is not a cryptographic hash-flood defense. Maintain a max-probe counter and bounded reseed/rehash attempts; exhausted limits cause an explicit resource refusal with KD/P6 and atomicity/effect-order rules, never silent key/effect loss. Reseed recomputes stored hashes, counted as O(total key bytes+entries). Use a strong keyed hash for attacker-controlled keys; document trust classification per ingress. Test chosen collisions, retry limits and latency on both baseline implementations. Removing diffusion previously caused 43–50 average probes and ~4.1x sessions (B/M4_NOTES.md:166-176), so keep mix/fin and require evidence for changes.

### 4.3 Cross-piece opportunities where proof removes checks

First establish equal-work baselines. Prioritize **json tape access, ring/chunk/tail spans and card unit iteration**, where safe hand Rust may retain bounds checks LLVM cannot eliminate. For each claimed proof win, publish the hand/Pruv assembly diff and surviving/removed check counts; if none exists, classify the gain as algorithmic, allocation or data-structure work. Admission, adapters and complete operations still meet <=1.0x.

| Piece / fact reused | Candidate work removed | Stage-two target and classification |
|---|---|---|
| req / entry: admitted inputs and progress state | World seals/replay validation and rebuilding records | Parity repair only unless a separate surviving hand check is identified; req remains blocked on P10H. Leg D's 0.63–0.68x is a control, not new credit. |
| json / pkgs: tape and document-owned NodeId bounds | Residual tape bounds in accessor chains, validated publication facts | Access chains <=0.90x, parse+publish+consume <=0.95x (i), conditional proof targets with assembly evidence; publication copy savings counted separately. Prove each deleted guard, not an unrelated global graph theorem. |
| mcp: forward traversal and initialized output prefix | Quadratic prefix copies/scans; later residual cursor bounds | Linear source-only algorithm first (P05M), packet core <=0.95x (i) exploratory algorithmic goal; proof credit only for separately demonstrated hand guards. |
| qwen / grok / opencode / claude / codex: window/ring/span bounds | Repeated span checks and internal copies | One selected flow <=0.95x (i), others <=1.0x. Separate assembly-proven checks from copy/layout gains; do not re-credit existing Qwen index or OpenCode pool fixes. |
| supervisor: ring/chunk/tail initialized spans | Residual safe-hand bounds on drain/tail | Core <=0.95x (i), complete <=1.0x, with assembly proof and equal startup/hash/output work. Deadlines and lifecycle checks remain. |

## 5. Adversarial review

D1–D22 are resolved below. Accepted means the design was changed; none of these decisions is an implementation or performance result. Accepted-modified records a narrower mechanism or corrected evidence, not a waived floor.

| ID | objection (one line) | severity | decision | what changed in the spec (section refs) or why rejected |
|---|---|---|---|---|
| D1 | Boundary removal cannot close M7; key-width work must precede language work | must-fix attribution / should order | accepted | §§1.1,6.2 put P01 then source P03–P05 before L1; matched ablation decides layout and residual priorities. ~1.7–2.0x is labeled an estimate, not a measured result. |
| D2 | Width bins mix other costs and 288-byte memcmp regressed | should | accepted | §§1.1,2.1,3.1 remove 100–220-instruction promise; retain 749 vs 694 evidence and add ASCII-8 ablation. |
| D3 | Packed four-u16 words may beat a new codec | should | accepted | §§2.1,6.2 compare packed-u16 with F1; explicit length/tails, unsigned big-endian word comparison and surrogate preservation. |
| D4 | G4 payloads must not be recoded as F1 | must-fix | accepted | §§2.1,3.2,7.2 limit F1 to budget keys/IDs; payload source UTF-8 spans, escaped-string WTF-8 only when needed, cached unit length and MCP pair-safe cuts. |
| D5 | Host encode plus canonicality validation still costs two linear passes | should | accepted | §§1.2,2.1,6.1,7.1 charge both passes to complete operations; retained-key lane is separate. |
| D6 | A caught panic can expose half-mutated trusted state to unchecked indexing | must-fix | accepted | §2.3 adds persistent poison on entry/failure, normal-exit-only clearing, safe drop and no C unwinding; P09 fault-injection gates. |
| D7 | Registry lookup and LLVM pointer-overlap checks can consume the boundary saving | should | accepted | §§2.2–2.3,6.2 use single-pointer opaque adapter handle, unsafe provenance contract, no registry/SoA entry signatures; measure LLVM boundary including poison/overlap. |
| D8 | Global graph invariant exceeds memory-safety needs | must-fix | accepted | §§2.3,3.1–3.2 separate I0 header, I1 local bounds and I2 functional tests; import I0+I1, keep total accessors and step budgets. |
| D9 | Reuse resident requires/ensures and trust-callers before new opaque types | should | accepted | §§2.3,3.1,6.2 replace old P06–P10 opaque-language plan with header expansion, visibility and adapter work; full opaque types conditional on C2. |
| D10 | Synchronous trusted host calls avoid budget continuation/prefetch machinery | must-fix | accepted | §§2.4,3.2,6.2 make G6 synchronous queries primary with executable result validation; Need/Resume only for proven async hosts. |
| D11 | 420k req queries make microsecond-scale calls incompatible with parity | must-fix | accepted | §§1.3,3.2,7.3–7.4 name synchronous in-process calls, account for 0.84s at 2µs, block req parity until P10H measured; move req off head. |
| D12 | Budget sub-1.0 goals mostly describe data structures, not proof wins | should | accepted | §§4.2–4.3 withdraw unsupported budget proof targets; label dense-ID/hash goals as data-structure wins vs (i), compare (ii), prioritize tape/ring/card and require assembly evidence. |
| D13 | Historical hand rows lack equal Unicode/numeric semantics | should | accepted-modified | §§1.1,4.1,6.2 require per-row equal-semantics hand and redo C1 attribution. Claude actually rewrites lone surrogates in a fallback, rather than simply rejecting them; this still needs a row-specific equivalence audit. |
| D14 | Strong hashing for every key lacks a threat model | should | accepted | §4.2 uses seeded existing mix/fin plus bounded probes/reseed/refusal, strong keyed hashing for attacker-controlled ingress; no cryptographic claim for simple seeded mix. |
| D15 | Aggregate inout/out-params/resident collections have no measured critical need | should | accepted | §§2.2,3.1–3.2,6.2 defer L2 until profiling; retain existing inout/buffers and the zero-copy-stub evidence. |
| D16 | Profile-selected specialization is excessive | minor | accepted | §§3.1,6.2 use source inline attribute or size heuristic with compile CPU/binary-size gates. |
| D17 | Existing budget runners do not establish LLVM parity | should | accepted | §§6.1–6.2 add equality lowering, LLVM driver/ABI and separate perf slices P17a–c; four-pair closure remains required, unsupported is pending. |
| D18 | Compiler changes cannot share one-hour product-Q gates | should | accepted-modified | §§6,7.4 replace whole-feature time promises with 3–5 implementation substeps per changed work package and explicit compiler/frontend/proof/ABI agreement gates; product slices use narrow families, full Q at closure. Counts are planning ranges, not measured durations. |
| D19 | M10a consumed markers do not prove live query order | should | accepted | §§2.4,6.2,8.1 specify actual query trace position after SETTLE and a query-before-SETTLE mutant; archived fx-30 adapter is not the live contract. |
| D20 | New KeyId state duplicates existing stored hashes/hash-once work | should | accepted | §§2.1,4.2,6.2 start with packed keys and stored hashes; interning only after repeated-hashing profile with full lifecycle costs. |
| D21 | Parity and stage two claim the same savings twice | should | accepted | §4.2 adds one-saving/one-owner ledger and incremental stage-two baseline; C3/C4 parity only, L5/C2 sites allocated once. |
| D22 | MCP quadratic copies are source-level algorithmic work | should | accepted | §§1.3,6.2,7.2–7.4 add P05M early pre-sized output/inout writes independent of G2/G3, with accepted-tool copy/visit slope gates. |

## 6. Shared facilities and budget port: gated work packages

No steps are performed in this design task. P labels are work packages, **not whole-feature <=1-hour promises**. Plan each changed package as 3–5 separately reviewable implementation substeps, normally half-day to one-day work units plus actual build/proof/run time; revise after discovery. A focused <=60-minute edit/review session may advance a substep but is not its completion gate. Deferred L2/full opaque features need a fresh estimate and are excluded from the required count. Keep test directories in place.

### 6.1 Shared Q and S/M protocol

Use two gate classes, not product Q as a proxy for compiler correctness:

- **C (compiler/ABI):** per changed feature, a manifest of frontend acceptance/rejection files, positive proof and false-contract/frame/alias negatives, codegen/visibility snapshots, and runtime ABI/lifetime/panic probes on both compilers. Compiler agreement covers acceptance, obligations and output, not just one product run. Budget 24 positive/40 negative frontend files when syntax/type rules change, 24 positive/20 negative proof files when proof rules change, and 24 runtime/16 ABI probes when boundaries change; unaffected categories need a written applicability mapping to existing coverage. G6's A0–A4+Q gates (docs/G6_SPEC.md:24-33,360-388) demonstrate scale, not evidence these new features already pass. Run targeted compiler regressions at each substep; feature integration closes the manifest.
- **P (product):** touched narrow Q families totaling 300 deterministic generated cases plus relevant fixed/negative controls, semantic oracle comparison, and S/M measurements. Baseline-only packages validate apparatus/controls without claiming product correctness. Fixture-only suites stay fixture suites; add a distinct generated lane if needed. Full product Q is reserved for closure after narrow gates.

Existing budget runners commonly default to Python/Rust compilers' **Rust output**, not LLVM (B/m11_test/run_m11.py:387); eq_range is rejected by LLVM (§3.1). P17a implements lowering, P17b brings up drivers/ABI, P17c measures LLVM core/boundary/complete operations. Before these pass, LLVM cells are explicitly **pending**, never inferred from Rust output or counted as successful four-pair runs. Afterward, touched product slices use both compilers × both backends. Full four-pair parity remains the final gate.

Record seed 20261117 and exact compiler/backend/flags. Codec/apparatus and cuts/not-implemented are not semantic passes. q without --impl is oracle determinism only (B/q.py:99-111,176-179). Generic q/test_impl needs the integrated scenario adapter, not a milestone's different stdin protocol.

Supported patterns from repo root:

```sh
python3 bench/l3/budget/q.py --families Q1,Q2 --count 300 --impl '<absolute scenario command>'
python3 bench/l3/budget/test_impl.py --group B,C,D --q-count 300 --impl '<absolute scenario command>'
python3 bench/l3/budget/q_gen.py --families Q1 --count 300 --out bench/l3/budget/work/m7/cases.jsonl
python3 bench/l3/budget/oracle.py bench/l3/budget/work/m7/cases.jsonl bench/l3/budget/work/m7/oracle.jsonl
python3 bench/l3/budget/m7_test/run_m7.py q --pruvc "$PWD/pruvc-rs/target/release/pruvc"
python3 bench/l3/budget/m7_test/run_m7.py perf --rounds 7
python3 bench/l3/budget/m8_test/run_m8.py perf --rounds 7
python3 bench/l3/budget/perf_headroom/host/run.py --only m4
python3 bench/l3/budget/perf_headroom/compiler/run_compiler_perf.py --rounds 7
```

Repeat generate/oracle/runner for M8=Q2/work/m8, M9=Q3/work/m9, M10=Q4,Q5/work/m10; run derive/same where supported, with fresh oracle for derived cases. M11 uses `PRUVC_RS="$PWD/pruvc-rs/target/release/pruvc" python3 bench/l3/budget/m11_test/run_m11.py --pairs rs --count 300 --seed 20261117`. Quote paths individually inside --impl; new runners use argv. These commands are templates, not execution results.

S/M are proposed profile names, not current CLI flags: S=100 sessions/calls,10 runs/delegations/clocks; M=500 sessions/calls,50 runs/delegations/clocks. Vary state size and call count independently. Existing M4/M7/M8 grids are 100/500/1000; M9/M10 10/50/100: run/extract those. M11 A=100/300/1000,B=100/500/1000,C=100/1000 (`B/m11_test/run_m11.py:355-356`); add 500 for A/C before uniform S/M claims. Do not invent --sizes for milestone runners. `perf_budget.py --axes sessions,calls --sizes 100,500 --repeat 5` is TS-only (`B/perf_budget.py:20-28`); add native/hand lanes. Final grids: `docs/REQUEST_BUDGET_SPEC.md:940-946`.

Every perf cell includes host encoding plus Pruv canonicality/input checks (two O(K) passes when applicable), and alternates equal-semantics Pruv/safe hand plus touched legacy adapter, ns/instructions when available, bytes/allocations/probes, growth separately, checksums/spread. Missing counters are unavailable, not zero. Shared exit: zero unexplained differences on implemented paths, compiler agreement, complexity contract, no new >1.1 regression. Cause-closing slices additionally require <=1.0; intermediate slices retain explicit open causes.

### 6.2 Steps

Execution order: **P01 → P02 → P03 → P04 → P05 → P05M**, all source/apparatus work before language extensions. P01 chooses F1 versus packed-u16 versus ASCII-8 by matched ablations, with uncertainty and complete-operation costs. P05M linearises MCP with existing inout writes. Then **P06–P13, P16, P17a–c**, followed by **P18–P35** in dependency order. P10H first establishes the synchronous host/query mechanism needed by P19/P24/P30/P31; its budget gate can pass independently while req's full grid stays blocked. P14/P15 are deferred, only promoted by a profile showing a concrete L2 need. P36 is whole-batch closure after §7, not permission for independent product integration.

The old 36 one-hour rows are superseded. This table has **38 required packages plus 2 deferred L2 packages**: P01–P36 excluding P14/P15, with P17 replaced by three packages and P05M/P10H added. At 3–5 implementation substeps per required package, plan **114–190 substeps**, not 36 hours; this is an unverified planning envelope, not a promise. P01/P02/run-only closure may need fewer code edits but still their stated evidence. Optional promotions add explicit substeps. Split the P10H signatures and P32 adapters per operation rather than hiding integration in a single step.

Paths use the citation prefixes. Future edits only; C/P refer to §6.1. Each substep runs its relevant targeted gates; complete package exit includes all stated evidence. No compiler-only slice is required to fabricate a product Q result.

| Package | Files / scope | Gate and closure evidence |
|---|---|---|
| P01 | B/PERF_HEADROOM.md, m7 driver, m8 hand: matched boundary and key ablations, equal-semantics hand | Apparatus: A/B/C + validation-only + F1/packed-u16/ASCII-8 at fixed hash/call structure; separate layout/hash/inline factors, retained/raw keys and host encode/check. Q1/Q2 Unicode/numeric controls; full-result S/M; no assumed 100–220-instruction gain. |
| P02 | B/q.py,test_impl.py,perf_budget.py,runners: native/hand lanes, argv/profiles | Apparatus controls, spaced paths, compiler/backend metadata; no oracle-only Pruv success. |
| P03 | New B/bud_keys.pruv or bud_bytes.pruv, leaf.rs,m34_test: selected source key packing/iterator | P: key codec negatives, Q2/Q6/Q8 touched cases; NUL/tails/non-ASCII/lone surrogates, length/order and canonicality. No language extension. |
| P04 | B/bud_map.pruv,key helpers: packed hash/equality with existing stored hash | P: Q2/Q3 map/collision/order, M4 S/M; no new interner; record threat model/probe budget. |
| P05 | B/bud_store.pruv,bud_world.pruv,legacy bridge: selected key layout in existing flat store | P: Q1/Q2 import/churn/grow/reuse and complete operations; no SoA entry or new opaque-language dependency. |
| P05M | bench/l3/mcp/packet.pruv,root.pruv: size accepted output then inout writes into one allocation | P: pages/cap/fingerprint/cut, 0/1/10/50/100/250/500 accepted tools; count visits/copied bytes linear in records+payload. Size from stable admitted descriptors/source spans, then write once; keep page storage alive through copying, caps/rejection order and pair-safe cuts. No extra host queries or dependency on G2/G3/new ownership features. |
| P06 | Both contract frontends: expand I0 header facts; manifest raw-entry visibility option | C: expanded requires/ensures acceptance/rejection and visibility misuse cases; no opaque syntax feature. |
| P07 | pruvc/verify.py and targeted sources: I0 preservation at constructor/mutator/grow exits | C proof positives/negatives, changed frames and refusal exits; no I2 graph gate. |
| P08 | RP/symbolic.rs,lib.rs: mirror P07 and agreement | C: obligation/acceptance agreement and solver CPU, not product Q substitutes. |
| P09 | Both Rust codegens/bindings + private adapter: visibility, trust-callers and poison | C ABI: arbitrary-buffer safe access rejected, partial-write/alloc/host panic injection, no poisoned observation/reuse, normal clear and safe drop. P: narrow Q1/Q2 M4 trial after C passes. |
| P10 | Both LLVM emitters/foreign bindings: single-pointer opaque adapter handle | C ABI/provenance/lifetime/no-unwind probes; fixed pointer count, no registry/SoA public entries. Measure poison and remaining overlap costs; preliminary boundary results pending P17c. |
| P10H | Budget and req concrete G6 host-fact signatures and adapters | C existing-foreign ABI regression/signature gates; P query order/result validation, host failure/reentry refusal. Measure budget synchronous path before dependent operations; req fresh-query/420k-call complete grid must pass before req unblocks. No decision logic in Rust. |
| P11 | B/bud_store.pruv,bud_map.pruv: import I0+I1 | P: malformed headers/offsets/links/keys, linear admission counts; I2 model tests separate from safety proof. |
| P12 | Same: local I1 mutator frames and guard removal one accessor at a time | C proof negatives if verifier changed; P Q2/Q3 churn/stale handles. Keep total accessors/step budgets until each local bound proved. |
| P13 | Same + B/host.rs: reserve/grow/failure | P Q2/Q3 growth, no replay of effects/queries, old state valid or terminal poison; byte/copy accounting. |
| P14 (deferred) | Both checkers/verifiers: aggregate/field mutation, only if residual profile requires L2 | New C manifest and estimates before activation; alias/frame proof cannot be replaced by Q. |
| P15 (deferred) | Both codegens/ABI: P14 lowering | C four-pair ownership/ABI probes, then touched P gates; no assumed budget gain. |
| P16 | Both verifiers: eq_range content/frames or explicit loop lemmas | C proof positives/false equality/frame/empty/alias negatives and solver cost; preserve bounds. |
| P17a | Both LLVM emitters: equality lowering/verified loop | C frontend/codegen/runtime agreement, empty/alias/range cases; explicit unsupported guard until complete. |
| P17b | Budget milestone/scenario drivers and bindings: LLVM build/transport/result lane | C ABI lifetime/layout probes plus P narrow Q on all four pairs; separately label old py,rs defaults. |
| P17c | Budget perf runners: LLVM matched A/B/C, direct and adapter lane | P S/M core/boundary/complete timing, wrapper poison/overlap/shim cost and full consumption. No four-pair parity inference from Rust numbers. |
| P18 | B/bud_lifecycle.pruv: current/isHalted/reopen | P Q1+B, lookup S/M <=1.0; retain dynamic identity checks. |
| P19 | Same: OPEN/attach/replace/CLOSE/maybeClose | P Q1/Q4+B/G, synchronous query/effect order after P10H; close <=1.0. |
| P20 | B/bud_charge.pruv,m8_test: counters/sums/bounds | P Q2+C/D, equal safe-number baseline; negative/boundary atomic refusal and charge/failure <=1.0. |
| P21 | B/bud_tools.pruv,key helpers: stored hash/dedup | P Q2+D; dup/new/forget separately <=1.0, missing-budget dedup. Interning only as measured follow-up. |
| P22 | B/bud_clock.pruv: runs/refresh/begin/settle/arm | P Q3+E/F, active-count functional model, R/timer cycle; do not treat I2 as an unchecked proof assumption. |
| P23 | Same+m9_test: fire/drop/close/stale timers | P Q3/Q4+F/G, timer order/deletion bounds and <=1.0. |
| P24 | B/bud_halt.pruv,m10_test: synchronous halt/effects | P Q4/Q5+G including derived cuts; actual query trace after SETTLE and query-before-SETTLE mutant must fail; no prefetch proof or D². |
| P25 | B/bud_card.pruv: validation/projection | P Q6+K, UTF-16 lengths/raw Number/refusal <=1.0; separate card proof-speed experiment only with hand assembly evidence. |
| P26 | Same+m11_test: resident setCard/hasCard | P Q6+K, B/C optional replacement/identity <=1.0. |
| P27 | B/bud_text.pruv,bud_num.pruv,bud_date.pruv,m6_test | P Q8+N/K, exact text/number/date, bounded source-byte outputs (not F1 payloads), helper grid <=1.0. |
| P28 | New B/bud_auto.pruv,M12 driver: clear/record/stop | P Q7+L, owner lookup/record sizes; actual semantics, no stub pass. |
| P29 | New B/bud_cleanup.pruv: restore/forget/dispose | P Q7+M, ordered timers/markers/linear reclaim; C/R/A axes. |
| P30 | New B/bud_decide.pruv: raise | P Q5+H, synchronous reached queries, limits/R; new M10b coverage. |
| P31 | Same: decide extend/end | P Q5+I, releaseHeld at reached position, save/refresh/wake and atomic bounds. |
| P32 | B/host.rs,roots/bindings,bud_test_abi.pruv,m34/m5–m11 shims | P split per operation/narrow family; direct/adapter grid, malformed admission, one decision engine. |
| P33 | Both codegens/hot contracts: residual L4/L5 only | C inline attribute/size heuristic and local proof gates; P touched families, M11 compile CPU/binary size and parity. |
| P34 | One measured §4 opportunity per subpackage | P touched families/collisions and raw/admitted complete operations; saving ledger, same-structure hand (ii), assembly evidence for any proof win. |
| P35 | B/compare.py,check_oracle.py,future judge,notes | P focused KD/final-observation checks; missing/unsupported paths remain open; full grid inventory ready. |
| P36 | Whole-batch results after §7 | Full Q four pairs ×2000 where generated, existing fixture suites, all final grids and selected stage-two targets; every cell <=1.0, req gate and LLVM complete. |

P36 wall time is unverified; monitor long campaigns in resumable <=60-minute sessions without restarting the corpus. On failure retain artifacts, repair using narrow regressions and rerun affected closure evidence; record every run, not a fictitious once-only success. This design task makes no commit, build, test or Q run.

## 7. Applying the framework to earlier pieces

### 7.1 One batch, shared correctness and measurement gates

L3-1..L3-9 are reworked together before real-Lair integration, with budget using the same facilities. Priority follows §1.3; shared G1–G4 prerequisites may precede individual consumers. Keep historical fixtures, seeds, intent comparisons, errors, ordered traces and malformed-input tests. Legacy packet tests exercise an adapter to the one typed engine; internal proofs replace re-checks, never external admission. After any lib/package change, directly run every affected consumer, including req/entry for ent_* and the manifest consumers for pkgs (`bench/l3/lib/README.md:5-12`; `bench/l3/pkgs/README.md:26-30`). Ordinary aggregate CI is not evidence that those piece suites ran.

For each piece, confirm **<=1.0x natural safe hand Rust per operation and size** on Python/Rust compilers × Rust/LLVM backends. Existing TS/Node lanes are additional comparisons, with <=1.0x required for matched applicable cells; no wall-time Q ratio or oracle-only run qualifies. Where no natural hand implementation exists below, writing it is a prerequisite, not a presumed pass. Preserve archived historical runs and clearly mark new runner extensions as proposed.

Use the §6 paired protocol: equivalent admission, validation, effects, allocation/output ownership, full result consumption, warmed alternating >=7 rounds, medians/spread and enough repetitions to resolve noise. An interval crossing 1.0 is inconclusive; profile/repeat it, do not round it into success. Report core, boundary/admission and complete operation separately, including both host encoding and Pruv canonicality passes where required. Count host calls, seals/checks, records visited, allocated/copied bytes and peak memory; unavailable counters are not zero. Include cold imports/growth, timeout/failure paths, raw keys and admitted handles. Keep process startup, filesystem/child I/O, hashing and rendering equal within a comparison; compare natural cores separately from packet-compatible adapters. Q execution durations are correctness campaign costs only.

Cross-piece complexity gates: for req, let M be items, S the sessions in each fresh observation and K candidates per session. Bound-search work is O(M·S·K) candidate visits for list and O(S·K) per open search, **plus actual compared key bytes, fresh snapshot admission, external observations and output**. Logical query count/order remains part of the contract; reducing physical boundary cost does not prove fewer semantic observations. Entry resume is O(new completion bytes + newly reached work + emitted effects), with no unconditional previous-input/full-answer-log replay term; an operation that semantically requests a new scan must still pay for it. Entry bound-index construction is expected O(total candidates + hashed/compared text units) at bounded load; adversarial collision work must be reported and bounded by the implemented fallback/refusal policy, never assumed constant. For json, require linear scan/publication work in input bytes+records and a forward cursor proportional to visited children; key lookup includes examined text. MCP requires linear visits and copy volume in admitted records+payload, not a quadratic accepted-tool prefix. For each remaining operation, record admission, visited records, key comparisons, copies, output and external-call bounds alongside its §7.3 grid before closing its cause; empirical ratios alone do not establish complexity.

### 7.2 Per-piece changes and tests that must keep passing

Here and in §7.3, paths abbreviated by a piece name are under `bench/l3/<piece>/`; all listed runners already exist unless explicitly marked **new/extend**. Named test programs are regression requirements, not assertions that this design task executed them.

| Order / piece | Apply the framework | Preserve existing tests and observations |
|---|---|---|
| 1 budget | G1 Store, G2 generation-checked Budget/Clock/Run handles, G3 record/effect moves, F1 selected compact keys and G4 source-byte results, direct operation ABI; F1–F4/P01–P36 retain safe-number semantics and ordered synchronous queries (§2.4). | All M1/M2 scenario/JV equivalence including lone surrogates, M3 malformed codec, M4 order/churn/grow, M5 effects, M6 exact text, M7 q/derive/same, M8 q/acct, M9 timers, M10 q/derive/same, M11 pairs, and missing M10b/M12 completion tests in §8.1. |
| 2 entry / lib ent_* | G1 EntryState carries current instruction/progress rather than replaying the full item and answer log. G2 input/record/answer invariants survive calls. G3 move newly admitted completion/delta effects, borrow existing log; G4 compact text spans. Use direct synchronous calls where available; only actual async completion boundaries use typed resume with effect-once and saved-fact validation (§2.4). | `entry/test_impl.py`, `q.py`, `forge_test.py`, `judge_test.py`, `q_mutants.py`; fixtures, pinned oracle and four-pair comparisons. Historical 155 fixtures/291 variants, integration10 and forge/control coverage (`RESULTS.md:2294-2308`). Run req too; forge coverage is not already complete for every entry opcode. |
| 3 json / pkgs | G2 verified Document/NodeId and forward child cursor; prove every local fact used by publication/access before removing the corresponding checks; unrelated global functional properties remain tests. G3 move source/tape/result; G4 owned/borrowed byte results with encoding-aware text. G1 needed for consumers' resident documents, not an artificial mutable parser store. Remove intermediate host serialization, retain external UTF-8/JSON/limit validation. | `json/run_a0.py` through `run_a5.py`, `run_q.py`, `run_q_mutants.py`; invalid tape/ID controls at import adapter, exact error positions, depth/record/byte limits, duplicate keys, UTF-16/surrogates and accessor behavior. Run every pkgs consumer and legacy lib frontend row (`pkgs/README.md:26-30`); package migration is not permission to drop those tests. |
| early source / 4 remaining mcp | G1 per-request transport/result state; G2 forward NodeId cursor avoids repeated array_at, verified ToolRecord. First P05M pre-sizes output and writes accepted tools via existing inout, independent of G2/G3. Later remove measured residual handoffs; G4 raw schema/source spans stay UTF-8, decoded strings use §2.1 and pair-safe unit cuts. Remove internal packet/host handoffs while keeping child exchange and fingerprint semantics explicit. | `mcp/test_impl.py`, `q.py`, `q_mutants.py`; normal/pages/many_pages/ping/fallback/cap/cut/fingerprint/numeric_id/flood (`test_impl.py:36`). Cap fixture has 499 invalid objects then one valid tool (`fixtures/server.py:36`); it does not cover 500 accepted-tool copying. |
| 5 qwen | Retain existing hash index. G1 owned import window/index, G2 document/chain handles, G3 move selected turn spans, G4 byte output; direct stage calls without re-encoding parsed facts. Keep actual file/window boundaries. | `qwen/test_impl.py`, `q.py`, `q_mutants.py`; branch/cycle/duplicate/leaf-parent-duplicate, trimming/clipping/dropped-adjacent-chain, names/list starvation/host-policy (`test_impl.py:53-98`). Collision equality and first-parent behavior remain. |
| 6 grok | G1 resident framing/import state, G2 validated event/document spans, G3 move windows/results, G4 compact event payload bytes. Evaluate event cursor instead of expanded event array; batch internal stages but preserve ordered reads and timeout observations. | `grok/test_impl.py` including slow categories, `q.py`, `q_mutants.py`; listing sort/concurrency, 200/201 turns, 2MiB suffix and 8MiB edges, copy-chunk-boundaries (`fixtures/listing.json:13-25,75`; `fixtures/budgets.json:2-39`; `fixtures/copy.json:24`). |
| 7 opencode | G1 admitted config/resident export state; G2 env/key and parsed-record invariants remove per-step geometry re-check. G3 move export/collector data, G4 bytes API, direct internal stages. Initial uniqueness validation stays; optimize its algorithm separately if measured. | `opencode/test_impl.py --slow`, `q.py`, `q_mutants.py`; F07 env-inherit/full/empty and large stdout, F08 lifecycle/deadlines, F09 parser, F13 tools, F14 plans, F15 collector; 200 fixed cases (`RESULTS.md:2262`). |
| 8 claude | G1 owned turn ring/window, G2 admitted node/span/turn handles, G3 consuming publication/output, G4 byte results. Reduce measured small-input allocation and internal stage handoffs; do not claim already-fixed deep clones as a new win. | `claude/test_a0.py`–`test_a4.py`, `q.py`, `q_mutants.py`; text/block, ring wrap, clipping, malformed input and depth limits. Keep all six `q_measure.py:123-124` input families. |
| 9 codex | G1 owned read/transport state and held-turn arena, G2 admitted responses/turns, G3 move held text into output, G4 byte API. Size buffers by demand if proven; direct typed stages, preserve real deadline/cancel/child boundaries. | `codex/test_impl.py`, `q.py`, `q_mutants.py`; fixed136, read_pages_40/end, exact200/byte budget and scalar clipping, shared transport deadline regressions (`RESULTS.md:2250-2252`; `fixtures/cases.json:1485,1526,2588,2660,2841`). |
| 10 supervisor | G1 opaque drain/ring state, G2 chunk/tail spans, G3 buffer ownership transfer, G4 non-widening bytes; remove internal transport/repack only, never child lifecycle/deadline observations. | `supervisor/test_a0.py`–`test_a3.py`, `q.py`, `mutants.py`; A3 fixed35/four pairs and Q29 fixtures, child/timeout/cancel/error/tail behavior (`RESULTS.md:2046-2049`). |
| 11 req (blocked on host gate) | G1 RequestStore + synchronous trusted host-fact calls as primary; direct Pruv decisions instead of world-seal/digest exchange. G2 admitted session/item handles; G3 move answers/results; G4 source-byte spans. Need/Resume only for a proved async host; req parity blocked until P10H passes §3.2's fresh-query/order and timing contract. Do not replace per-item session observations with one cached index. | `req/test_impl.py`, `q.py`, `forge_test.py`, `test_compare.py`, `q_mutants.py`; 160 fixed fixtures/four pairs, integration and stale/forged packet controls (`RESULTS.md:2329-2340`). Q3 snapshot changes and Q7 concurrency/session changes/retry stay (`req/Q_README.md:18-25`). |

### 7.3 How each piece confirms 1.0x

Each row must satisfy §7.1 on its entire listed grid; S/M cells are early feedback, not closure. **New** grids/runners below are specifications for future work, not supported current CLI flags. Baseline authoring includes semantic checks against existing oracles; do not teach hand Rust to copy/revalidate packets unnecessarily to make Pruv win.

| Piece | Baseline and runner | Sizes / separate lanes required |
|---|---|---|
| req (blocked until P10H) | Existing `perf/run_ab.py --reps 3`: Node reference vs B/C; `perf/d/run_d.py --reps 5`: same-layout safe hand `d/hand.rs`, JS and Pruv kernel. `req/perf_req.py` is whole-process Node comparison. **Extend** with natural safe hand request engine and matched typed/end-to-end >=7-round lanes. | A–C/D S=2000/20000/50000, M=10 (`perf/REQ_THREEWAY.md:109-116`); req grid S=1000/5000/20000, M=1/10/50, list/open, none/first/last match (`req/perf_req.py:143-176`). Use 1000/5000 for S/M. S=50000 currently hits Pruv call bound while Node succeeds: error mismatch is not a speed win (:44-46). Leg D remains a control, never full-request parity. |
| budget | Existing M4 `budget/perf_headroom/host/run.py --only m4`, M7/M8/M9 milestone perf runners and hand binaries, M11 runner/hand; TS lanes retained. P01 adds equal numeric/UTF-16 semantics; §6 adds missing hand/TS and explicit LLVM bring-up/perf lanes P17a–c. Current py,rs defaults do not establish four-pair support. | M4/M7/M8 100/500/1000; M9/M10 10/50/100; M11 A/B/C and helper grids from §6.1, full final grid there. All op/size rows <=1.0, adapters against same-work hand plus natural core separately. |
| entry/lib | `entry/perf_wb.py` compares current vs old harness, **not hand Rust** (`:25-39`). **Extend** with safe typed hand entry engine and Node pinned oracle lane; keep old/current regression and add resident progress/replay measurements and continuations only for actual async hosts. | Existing 500/1000/2000; tables and keys modes; extend 5000/10000/20000 as historical table sweep (`entry/IMPL_NOTES.md:717-725`). **New** replay completions 1/10/50/100 at fixed input size and vice versa. S/M=500/1000, answers=10/50; check no full-history work per resume. |
| json/pkgs | `json/run_q_measure.py`: four pairs vs Node JSON.parse (`:13-57,60-75`). **Extend** with natural safe hand parse/document/access API, matching error/Unicode policy; Node parse+consume lane for equal requested results. Keep historical parser/tape ratio labeled separately. | Existing plain8MiB, escapes4MiB, object100000, array500000, depth128/129, hostile depth1M, bytes16777216/16777217; text decode lanes. **New** S/M=64KiB/1MiB and accessor chains 1/10/100 per document. Limits/refusals checked, not compared to Node success as equivalent. Package acceptance also needs all consumer grids below. |
| mcp | **New** `mcp/perf.py` with safe hand typed ToolRecord/JSON result collector and pinned Node reference, matching order/fingerprint semantics. Q runner is not perf. | **New** accepted tools 0/1/10/50/100/250/500, pages1/5; S/M=10/50. Hold per-tool payload fixed then vary title/description/schema bytes. Time pure parse/collect, Node fingerprint/render and real-child end-to-end separately. Verify linear visits/copied bytes and matching payload, including 500 accepted tools. |
| qwen | `qwen/q_measure.py:19-29` existing rs/llvm single cold-call vs TS readQwen. **Extend** to four pairs, repeated matching file/window scope and natural safe hand import/index/turn API. | Existing 2/4/8MiB, 256/512/1024 chain entries (8192-byte padding); S/M=2/4MiB. **New** dense chains, duplicate/branch/cycle and adversarial hash collisions at fixed bytes; raw/window admission and final output included. |
| grok | **New** `grok/perf.py`: safe hand listing/framing/history/copy implementation and pinned TS reference; exclude intentional Q stalls from throughput. | **New** candidates50/100/500/1000; history64KiB/2/4/8MiB, ordinary vs dense short lines; copy64KiB boundaries and 2/8MiB. S/M=100/500 candidates, 64KiB/2MiB. Compare peak event-array memory and each operation, not just aggregate. |
| opencode | **New** `opencode/perf.py`: safe hand config/export/collector engine, pinned Node reference with equal child/SQL work; separate core and external effects. | **New** env entries0/10/100/500/1000; SQL rows/session candidates10/100/1000; export64KiB/2/8/16MiB. S/M=10/100 env and64KiB/2MiB. Time begin uniqueness separately from repeated step geometry; preserve lifecycle timeout tests outside steady throughput. |
| claude | Existing `claude/q_measure.py:78-137` four pairs and `claude/handrust/src/main.rs:10-18` hand loop. **Extend** orchestration to run equal-semantics hand and Node for every cell with §7.1 repetitions; historical TS timing was manual. | Six existing inputs: small10 turns, window8MiB, long_text, block_arrays, ring_wrap, depth_limit. **New** small turns0/1/10/16/17/200 for growth; S/M=10/17. Match parsing, validation, output ownership and trimming rather than assuming existing hand semantics suffice. |
| codex | **New** `codex/perf.py`: safe hand resident reader/collector/transport and pinned TS reference with identical scripted replies; separate process-I/O lane. No Q wall-time parity claim. | **New** turns0/1/10/50/200, pages1/5/40, byte-budget below/at/above limits and8MiB line boundary; S/M=10/50 turns,1/5 pages. Measure small-input allocation, held-arena and collector copy volume. |
| supervisor | `supervisor/measure.py:20-61` existing median5 vs TS; **extend** four pairs, natural safe hand ring/child driver, matched startup/hash work and >=7 rounds. | Existing64MiB output/64KiB chunks; **new** bytes0/64KiB/1/16/64MiB, chunks4/64KiB, tail_chars=1/4096/65536 UTF-16 units (0 is a separate invalid-admission case, not a successful perf cell; supervisor/supervise.pruv:119); S/M=1/16MiB. Core drain/tail lane excludes startup equally; whole-process lane includes the same child, hashing and final output on both sides. 1.11x historical TS is still open. |

### 7.4 Execution packages and gates

Follow §6.1's distinct compiler (C) and product (P) gates. Each earlier-piece package has **A baseline/counters**, **B one state/local-invariant root**, **C one ownership/source-byte handoff**, and **D narrow composition+S/M closure**. A is apparatus work; B/C are split per named root/handoff and use touched Q families/fixed negatives, not full-suite Q on every edit. Compiler extensions get their own frontend/proof/ABI manifests before product adoption. D does not defer the earlier substeps' relevant checks. Full Q and complete grids run at whole-batch closure.

Plan at least **3–5 substeps for each B or C package**, plus A and D: **8–12 substeps per non-budget piece**, or **80–120 across the ten earlier-piece groups below**, before extra roots, baseline authoring and whole-batch integration. This is a preliminary lower planning envelope, not a one-hour promise. Budget uses §6's 114–190 estimate without duplicate counting. These ranges are not additive: P05M is the early part of MCP's C work and P10H overlaps req host apparatus; shared substeps must be counted once in the implementation manifest, not as extra savings or second implementations. Extend estimates for additional opcodes/host signatures; half-day/day work units plus proof/build time replace the old 45–55 minute feature caps. Long campaigns retain artifacts and use <=60-minute monitoring sessions.

Queue order is explicit; R IDs identify packages and need not sort numerically. Start R01's source-key work, then R00/P05M before language work, then finish R01. Req's mechanism experiment P10H runs early, but the req product package stays last and blocked until its gate passes.

| Queue | A / B / C scope | D closure |
|---|---|---|
| R01 budget | A: P01–P02; B/C: P03–P35 in §6 order, with P14/P15 deferred and no duplicate edits | C1–C10 per-op parity, surrogate/external-input rules; LLVM P17a–c explicit. |
| R00 mcp early source part | A: accepted-tool copy/visit counters; C: P05M pre-sized output and inout writes | 0–500 accepted tools, linear records+payload copy slope; no G2/G3 dependency. R04 continues later. |
| R02 entry/lib | A: replay/copy counters and hand lane; B: one owned progress root; C: completion/effect handoff using source spans | entry + req consumer regressions; at-most-once effects; only actual async boundaries need continuation tests. |
| R03 json/pkgs | A: parse/publication/access/copy split; B: local Document/NodeId bounds; C: source/tape byte handoff | Narrow A0–A5/Q/mutant families and affected consumers, external tape/ID admission; full suites at closure. |
| R04 mcp remaining | A: rebaseline after R00; B: cursor/ToolRecord local bounds; C: any remaining measured byte handoff | pages/fingerprint/cap/pair-safe cut and 500 accepted tools; no double credit for P05M. |
| R05 qwen | A: matched IO/window/density/collision grid; B: one document/chain transition; C: selected-turn span handoff | Existing index, branch/cycle/duplicate semantics and collision bounds preserved. |
| R06 grok | A: dense-event memory/list/history; B: one framing/event-span root; C: one output handoff | Slow fixtures/read-timeout ordering and memory/complete grids. |
| R07 opencode | A: begin vs repeated env checks; B: one admitted config/export root; C: export/collector handoff | Touched F07–F15 and lifecycle cases; initial uniqueness validation stays. |
| R08 claude | A: equal-semantics small/growth hand comparison; B: ring/turn spans; C: one allocation/output handoff | Six perf families/growth edges, exact required unit semantics; assembly evidence for any proof-speed claim. |
| R09 codex | A: empty/small/40-page allocation/copy counts; B: transport/read-state bounds; C: held-turn/output handoff | fixed136, budget/scalar clipping and shared deadlines, source UTF-8 retained. |
| R10 supervisor | A: matched startup/hash lanes; B: ring/chunk local bounds; C: byte tail handoff | Child/cancel/deadline cases, chunk/tail complete grid <=1.0. |
| R11 req (blocked) | A: equal-semantics full hand lane + P10H query/call/byte counts; B: one RequestStore root with synchronous facts; C: world/answer handoff | Fresh Q3/Q7 observations and forge controls; 420k-call overhead and full list/open grids <=1.0 before unblocking. Async-only host requires new design, not assumed continuation parity. |
| R12 whole batch | Audit dependency/intent ledger; close missing hand lanes and unresolved causes with new packages, never waivers; schedule P36 | Full four-pair Q ×2000 where generated, existing fixture suites and all §7.3 grids/selected §4 goals. No real-Lair hookup while any baseline/backend/cause or req gate is incomplete. |

## 8. Test migration and open assumptions

### 8.1 Compatibility plan

Keep scenario JSON/JV, fixtures/seeds/oracle traces and test directories. Relative path assumptions are in `B/m34_test/run.py:20-24,55`, `B/m7_test/run_m7.py:25-33,54`, `B/m8_test/run_m8.py:27-32,52`, `B/m11_test/run_m11.py:29-42,63-69`. Test roots adapt to the product; product memory does not preserve old offsets.

| Existing tests | Layout dependency and shim | Retain / retire |
|---|---|---|
| M1/M2 q_gen.py,q.py,compare.py,check_oracle.py,test_impl.py | Mostly scenario/JV-independent; preserve u16/num/undef and ordered observations (`B/compare.py:8-27,62-69`). q --impl needs scenario adapter. test_impl --group accepts lists; check_oracle --group one group. | Keep Node intent, fixtures/full observations, reference-trace comparison and KD judge; no broad normalization. |
| M3 m34_test/root_m3.pruv,bud_api.pruv,bud_test_abi.pruv | i64 tags/length/status/rejection order pinned by `B/ABI.md:14-77`. Decode once to typed args, encode results. | Retain exact codec/malformed tests; retire packets from product only. |
| M4 m34_test/root_m4.pruv,drivers/model | Header/slot/entry/blob/grow assumptions (`B/bud_store.pruv:4-18`). Logical map adapter plus test-only old snapshot import/export. | Keep order/churn/reuse/grow/complexity; retire product 16-word-header/20-word-entry assertions, replace with abstract invariants. |
| M5 m5_test,host.rs,leaf.rs,harness.rs | Oracle-derived effect programs test mechanics, not budget semantics. Typed Core facade retains byte index/JV. `B/m5_test/run_m5.py:43-44` unquoted paths are shlex-split (`B/q.py:49`); use argv/quoting. | Keep effect-once/storage identity; retire stand-in as semantic completion evidence. |
| M6 m6_test/compiler perf | Expected unit arrays; byte output converts losslessly at test edge, generator owns contracts. | Keep exact Node text/date/Number and stack/perf controls. |
| M7 m7_test | Packed key/record and current [4,n,off] (`B/ABI.md:85-91`). Test-only BudgetHandle projection before mutation. | Keep q/derive/same/legacy fixtures; retire exposed offsets/cached packet identity from product. |
| M8 m8_test | 15-word cv/10-word what (`B/bud_charge.pruv:5-11`) and accounting PLACEHOLDER | Typed CounterRecord/ChargeArgs shim; keep q/acct, atomic/negative/P6. Retire placeholder product state and duplicate decisions. |
| M9 m9_test | State/view offsets and timer/result codes | Typed clock/result views; keep q/same, stale timers, overwrite/wait/order; scheduler mechanical. |
| M10a m10_test | Halt -2 tail/ID lists (`B/M10_NOTES.md:11-25`), effect/prefetch markers | Legacy codec for archived tails, synchronous Halt/effects product; actual query trace after SETTLE and query-before-SETTLE mutant (§2.4). q/derive/same keeps counts/between-request events; archived prefetch markers cannot establish live timing; M10b new work. |
| M11 m11_test | V trees, replacement stand-in, A/B/C protocols | Convert raw values once; preserve IEEE card numbers. Real resident B, C separately labelled; identity/optional replacement checked. |
| M12/integration | No bud_auto.pruv,bud_cleanup.pruv/dedicated M12 directory here. Q7/L/M define required coverage (`docs/REQUEST_BUDGET_SPEC.md:1094`; `B/M10_NOTES.md:1-8`). | Add real auto/cleanup entries/scenario tests; cannot claim absent implementation tests pass again. |

M7–M10 prepare cases/oracle, then q, supported derive and same against saved reference observations. Derived cases need their own oracle/artifacts. Keep explicit cut reason/prefix coverage: M10 is 78 exact/222 unsupported originally versus 300 derived halt-only exact (`B/M10_NOTES.md:60-75`), not 300 complete Q4/Q5 successes. M11 records 213 exact/87 prefix-only (`B/M11_NOTES.md:70-81`); integration must close those. Python/Rust same-output checks cannot replace Node intent checks.

Retire only product word layout/dispatch, repeated validation after proven ownership, JSON-tail rewrites and obsolete stand-ins. Keep one authoritative legacy bridge for archived fixtures, measured against same-work hand Rust. Snapshot conversion is admission/export, never hidden per-transition work. New malformed tests target constructors/external refs/generation reuse/type mismatch, poison/reentry and host-result validation; resume sequences only for a proved async host, not fabricated resident offsets.

### 8.2 Unverified assumptions

- Static reads/document edits only: no builds, tests, Q, runtime surrogate/poison probes or benchmark reruns. Gains, counts, durations and stage-two targets are hypotheses; archived ratios are not new measurements.
- Most earlier pieces lack matched equal-semantics hand baselines. Historical Apple-silicon runs were loaded and some helpers specialized (B/perf_headroom/COMPILER.md:8-10,70-82); rebaseline before exact ns acceptance. Claude's fallback and M8 numeric/key differences need the §4.1 row audit.
- Minimum resident-contract expansion/visibility, adapter poison and LLVM handle plumbing still need implementation validation. Full opaque-language types and L2 are deferred, not prerequisites. LLVM equality/driver/perf bring-up P17a–c remains necessary before four-pair closure.
- I0 removes valid only; I1 proofs are needed for each dependent guard. I2 functional properties remain tests, not a safety-proof gate. Import validation, dynamic generation checks, alias exclusion and panic/allocation handling remain required.
- Synchronous host-fact signatures, live req observation compatibility and net call costs are unverified. Req parity stays blocked until P10H and its full grid pass. Need/Resume is only for proved async hosts and needs a separate design/measurement.
- M10b/M12, integrated scenario execution, KD judge and native/hand/LLVM perf lanes remain completion dependencies. M10 lacks hand/TS baselines; replacing archived prefetch needs live query-order evidence and the SETTLE mutant.
- Source-only MCP linearisation is proposed, not measured. JSON 6.3x is one parser cell; Q wall times are not throughput. Key layout selection, seed/probe/refusal policy and any conditional interning need measured lifecycle/complete-operation costs.
- Fixed UUID fast paths require exact form/spelling. Snapshot versioning, source-span lifetimes, escaped-string materialization/unit lengths, and host allocation/destructor contracts need integration evidence without weakening external validation or lone-surrogate equivalence.
