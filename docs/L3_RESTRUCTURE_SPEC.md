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

**Proof time is a budget too.** A proof-only change must not buy run-time speed with unbounded verifier cost. When an invariant is preserved through many calls, keep its predicate opaque (`opaque fn`, Appendix P13o) and reveal the body only on the paths that chain it, rather than expanding it at every contract instantiation. Report proof CPU next to every such change (Appendix P13a.4 is the current reference).

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
| P13o | Both checkers/verifiers: `opaque fn` and `reveal F;` (Appendix P13o) | C: positive reveal/congruence/loop-scope cases; negatives for a missing reveal, false facts through the opaque symbol, totality, signature and reveal-target misuse; framework rows equal on both compilers. Unblocks P13a step 2 (Appendix P13a.3), which lands separately. |
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
- Opaque predicates (P13o) are sound by construction (Appendix P13o.2) but their proof-time gain on bud_map/bud_world is not yet shown: the first P13a step-2 attempt with `kept`/`dyn_ok` opaque still needed `dyn_ok`'s body in the header-writing mutators and timed out in `put_at`/`drop_entry` (Appendix P13o.7).
- Fixed UUID fast paths require exact form/spelling. Snapshot versioning, source-span lifetimes, escaped-string materialization/unit lengths, and host allocation/destructor contracts need integration evidence without weakening external validation or lone-surrogate equivalence.

## Appendix P06. Contract expansion and raw-entry visibility (design note for P07–P09)

P06 lets a contract use an I0 header fact such as `valid(st)` (bench/l3/budget/bud_store.pruv) without opaque types (D9) and records the raw-entry visibility choice. User rules are in SPEC.md §6 ("User functions in contracts") and §6f; the manifest key is docs/G4B_SPEC.md §3 rule 7. P07–P09 reuse the representation below unchanged.

### P06.1 Representation

| Item | Representation (pruvc/check.py `x_*`, pruvc-rs check.rs `x_*`, same rules and messages) |
|---|---|
| Expandable function | Not generic, no `inout`, no `requires`/`decreases`/`cost`; params in {i64, bool, [i64], [u8], [bool]}; returns bool or i64. Body: immutable i64/bool lets without refinement, `if`, `return`; only bool functions branch; every branch returns. Expressions: literals, names, unary/binary, index, calls of `len`, bit builtins, `load_be` and other expandable user functions. |
| Expansion | At check time a contract `Call` of a user function is replaced **in place** by the expansion (py swaps `__class__`/`__dict__`, rs overwrites the node). No new AST node: verify, both codegens and the runtime requires check see an ordinary expression. Every expanded node carries the call-site file/line/col. |
| Lets and branches | Lets are substituted lazily at each use (their domain is needed only where used). `if c { return a; } rest` becomes `(c && a) \|\| (!c && rest)` (`x_join`), so the guard of an `if` guards later reads in contract-domain order. Arguments are copied at each parameter use (pure, so meaning is unchanged; runtime checks evaluate them more than once). |
| Limits | Expansion size at most 4096 nodes (`X_MAX`). Recursion is impossible because a self/mutual call needs `decreases`, which already disqualifies; the "it is recursive" reason is a defensive guard. |
| Errors | `type/call_in_contract` "function F cannot be expanded in a contract: REASON"; a call in a `cost` clause keeps the v0.1 message. A `pub` function's `requires`/`ensures`/`decreases`/`cost` that directly calls a private function is `type/public_interface` "public function P exposes the private function Q in its contract". |
| Meaning | Contract arithmetic is mathematical. The callee's own body is verified with no assumptions (no `requires` allowed), so every code operation in it is total and its contract value equals its run-time value. Under `--no-verify` this is trusted, like every other contract. |
| Raw entries | Manifest `raw_entries`: `"crate"` (pub(crate) behind a host adapter) or `"unsafe"` (documented `pub unsafe fn`); public safe values are rejected; only the program package may set it. Carried as `Program.raw_entries` (py attribute / rs `Option<String>`); missing means today's output. P06 does not change codegen. |

### P06.2 Evidence manifest (C)

Both compilers give the same acceptance, rejection, stage, code, line, col and message (run_all `--compiler py` and rs default), and the same `.rs` for every positive (diff_rust group `l3`).

| Category | Files | Notes |
|---|---|---|
| Frontend positives (24) | tests/l3/frontend/*.pruv, run_all group `l3_frontend` | requires/ensures/invariant/assert/decreases, i64 and bool callees, nested and chained calls, let chains, if-join, short circuit, Euclidean `/` `%`, bit builtins, `load_be`, bool/u8 arrays, private callee in private fn, full bud_store `valid(st)` and a caller relying on it. |
| Frontend rejects (40) | tests/must_reject/l3_cx_*.pruv | 26 `call_in_contract` reasons (generic, inout, requires, decreases, cost, param/ret types, let/statement/expression shapes, i64 branch, missing return, eq_range, too large, nested reasons, cost-clause call) plus type/arity/shadow/unknown/effect around expansion and 7 verify-stage meaning checks (false ensures, let/branch/header meaning, domain of `/` and index, caller precondition). |
| Visibility misuse (5) | tests/must_reject/l3_vis_*/ (multi-file projects) | private callee in a pub `requires`, `ensures`, `decreases`, nested inside a pub callee argument, and in the root module. |
| Manifest | tests/g4b/check_pkg.py | `raw_entries` crate/unsafe accepted; pub/public/safe, other strings, non-strings, and the key in a dependency rejected with exact messages. |

Applicability mapping for categories P06 does not change:

| C category | Why not new in P06 | Existing coverage / owner |
|---|---|---|
| Proof positives/negatives (24/20) | verify.py and RP unchanged; expansion yields ordinary expressions | Verify-stage rejects above; all g1–g10 proof groups. New I0 preservation proofs are P07, mirror P08. |
| Codegen/visibility snapshots | Generated code is unchanged except for expanded requires checks | Identical `.rs` from both compilers (diff_rust `l3`, `edge`, `must_reject`). Visibility in codegen is P09. |
| Runtime 24 / ABI 16 | No boundary changed; `raw_entries` is recorded only | P09 (adapter, trust-callers, poison), P10 (LLVM handle). |
| Effects, shadowing, arity, types, unknown names in contracts | Rules unchanged, checked before expansion | check_edge `call_in_contract`, `effect_in_contract`; must_reject type groups; the l3_cx type rows confirm order. |
| Recursion | Unreachable (decreases disqualifies) | l3_cx_mutual, l3_cx_decreases. |

### P06.3 Notes for P07–P09

| Package | Note |
|---|---|
| P07 | A code-level call does not know the callee body: `if small(x) { x + y }` does not bound `x`. Expose the body as an automatic ensures (`ensures result == <expansion>`) of expandable functions, or state facts with explicit ensures. Do not add a second expansion path; reuse the existing one (py `x_expand`, rs `x_template`/`x_instantiate`). |
| P07 | Contract-domain rejects can be false when a guard sits behind `band` on an element (solver sees `band` as opaque); P07 frame/mutator proofs should state the needed bound directly. |
| P07/P08 | Solver cost: nonlinear products with `band` and `result * 2 == st[4]`-style facts timed out in trials; keep header facts linear (offsets as sums/compares) and measure solver CPU for P08. |
| P09 | Runtime requires checks print and evaluate the expanded expression with duplicated arguments; acceptable for O(1) headers, but P09 should keep raw-entry requires checks at the adapter only once. |
| P09 | Apply `Program.raw_entries` in both Rust codegens and bindings: `"crate"` emits `pub(crate)` raw functions plus the adapter, `"unsafe"` emits documented `pub unsafe fn`. Consider making `--trust-callers` require one of them. No public safe arbitrary-Vec entry in any mode. |
| P09 | Visibility checks only direct calls: a pub contract may expand pub `q` whose body calls private `r`, so `r`'s body is part of the public contract meaning. This is an interface-stability policy, not soundness (expansion happens in the defining package; other packages see an ordinary expression). Tighten to the whole expansion only if P09 publishes contracts across packages. |
| Later | i64 functions cannot branch (no conditional expression in contracts); add one only if a header fact needs it. |

## Appendix P07. Header facts as contract facts (P07 Python, P08 Rust mirror)

P07 makes the I0 header fact `valid(st)` (bench/l3/budget/bud_store.pruv) a contract fact that constructors establish and mutators preserve, and lets code calls of expandable functions use their bodies. P08 mirrors the verifier change in pruvc-rs; both compilers give the same acceptance, rejections (stage, code, line, col, message) and `.rs`. No I2 graph gate, no new syntax, no opaque types. User rule: SPEC.md §6 "Code calls know the body too".

### P07.1 Design

| Item | Rule (pruvc/check.py `x_auto`, pruvc-rs check.rs `x_auto`; pruvc/verify.py call path, RP symbolic.rs `call_contract`) |
|---|---|
| Automatic body fact | After the checker's last pass, every expandable function (Appendix P06.1) stores its expansion template in `x_auto`. At a **code** call the verifier assumes `result == <template at the call's arguments>` next to the written `ensures`; the template is evaluated over the arguments before `result` is bound, so a parameter named `result` keeps its argument value. The template is the existing P06 expansion (py `x_expand`, rs `x_template`); there is no second expansion path. Contract calls are already expanded in place and do not use it. |
| Why it is sound | An expandable function has no `requires`, no `inout`, no effects and no loops; its body is verified with no assumptions, so its run-time value equals its mathematical value for every argument, and the arguments are evaluated once before the call (the assumption is over their values). Under `--no-verify` it is trusted like every other contract. |
| Bit builtins | An `i64` function whose template calls a bit builtin gets no `x_auto`: bit-vector terms in every caller's path made existing proofs time out (p2_rt_kernels). `bool` predicates always get it, so `if valid(st)` bounds the reads it guards. Callers that need an `i64` bit helper's value state it with an `ensures` (l3_hf_bit_i64 shows the reject). |
| Header fact | `valid(st)` is linear except the power-of-two test `band(s, s - 1) == 0` on word 4. z3 does not decide that for a symbolic `s`, so the constructors `init_store` and `bud_map::grow_copy` close with one O(1) run-time `valid` check (never false for `s = layout_slots(l)`; once per store or grow, not per call). |
| Store contracts | `ea`/`eb` state the entry offset bounds under `valid(st)`; `fill_entry`, `arena_put`, `entry_alloc`, `entry_free` ensure `valid(old(st)) ==> valid(st)` (fill_entry also keeps words 0..15); `init_store` ensures `result != 0 \|\| valid(st)`; `grow_copy` ensures the same for `nxt`. `c32`, `c40`, `hd`, `set_hd` keep their bodies (calls to bench/l3/lib/ent_u16 `clamp`, `rd`, `wr`) and gain exact `ensures`; `clamp`, `rd`, `wr` gain the matching exact `ensures` (value in range, element read, element written plus a frame over the other elements), so callers know the header words. Refusal exits keep the fact (hf_stack `push` proves it with a whole-array frame). |
| Generated code | The only change in the generated `.rs` is the two constructor checks (root_m4 diff vs HEAD: two lines). Inline attributes are unchanged; see P07.4 for why the helper bodies were not rewritten. |

### P07.2 Evidence (C)

| Category | Files | Result (py and rs identical) |
|---|---|---|
| Proof positives | tests/l3/proof/hf_auto.pruv (i64 and bool callees, `len` helper, guard predicate, bit predicate), hf_stack.pruv (linear header: constructor, push with refusal and normal exits, grow returning a new array, checked read) | run_all group `l3_proof`, default and `--infer`: 4/4 rows on both compilers; diff_rust group `l3` covers tests/l3/proof (same `.rs`). |
| Proof negatives | tests/must_reject/l3_hf_frame (mutator changes a header word: verify/ensures), l3_hf_refusal (refusal exit breaks the fact: verify/ensures), l3_hf_grow (grow forgets a header word: verify/ensures), l3_hf_elem_domain (a `requires` bound on the element at `4 + top`, which the header fact lets reach `len(st)`: verify/contract_domain), l3_hf_bit_i64 (i64 bit helper gives no body fact: verify/index_oob), l3_hf_contract_only (predicate guarding a different index: verify/index_oob), l3_hf_result_param (a parameter named `result` keeps its argument value in the body fact; audit finding: verify/index_oob) | 7 rejects; same stage, code, line, col and message on both compilers. |
| Budget roots | m34_test/root_keys, root_m3, root_m4, m8_test/root_m8, m9_test/root_m9 | Verify on both compilers; `.rs` identical. Slices json a0–a5, mcp, qwen, supervisor a0–a3, budget m6, m7 q, m10 q, m11 pass. |

Compiler CPU (user seconds, idle machine, HEAD compiler on HEAD sources vs new compiler on new sources; z3 is most of it):

| Root | rs HEAD | rs new | py HEAD | py new |
|---|---|---|---|---|
| root_keys | 2.88 | 2.86 | 3.34 | 3.85 |
| root_m3 | 0.44 | 0.38 | 0.72 | 0.64 |
| root_m4 | 4.20 | 11.25 | 5.56 | 13.15 |
| root_m8 | 5.23 | 12.03 | 7.06 | 14.31 |
| root_m9 | 4.55 | 12.16 | 6.67 | 13.85 |

The added ~6–7 s per budget root is the new store obligations; the largest single ones (py) are `entry_alloc` ensures 0.76 s, `fill_entry` ensures 0.81 s and `entry_alloc` contract_domain 0.47 s. All are far below the 8 s query timeout on an idle machine.

### P07.3 Notes for P09–P13

| Package | Note |
|---|---|
| P09 | The adapter can now rely on `valid(st)` after `init_store`/`grow_copy` and across the store mutators; per-call `valid` at raw entries is still needed until the handle owns the store. |
| P11 | The bud_map header writers (`sh`/`sf` at bud_map.pruv lines ~301, 408, 440, 442 and the entry/link writes) and the mutators in bud_clock, bud_lifecycle, bud_world, bud_tools, bud_halt do not carry `valid(old(st)) ==> valid(st)` yet. Add them one writer at a time, stating the touched word's bound directly (header words 7, 8, 10, 12–14). |
| P12/P13 | Once grow_copy's copy loop preserves the fact, its closing run-time `valid` check can go; the constructor check stays until the power-of-two fact has a non-`band` form (for example, carry `s == layout_slots(l)` or a log2 word in the header). |
| P08+ | Solver cost: keep new header facts linear. Each `valid(old(st)) ==> valid(st)` ensures costs about 0.3–0.8 s; prefer frame facts on the header words (`forall k in 0..16`) over re-proving `valid` from scratch. |
| Perf | See P07.4. |

### P07.4 Perf (D)

Same machine, same run, new compiler on new sources vs HEAD compiler (87e142b) on HEAD sources; ns per call, ratio to hand Rust.

| Run | Cell | HEAD | New | Note |
|---|---|---|---|---|
| m7 perf (`run_m7.py perf`) | attach, closereopen, current, ishalted, replace, n = 100/500/1000 | Pruv instructions 684, 800, 650, 644, 2951 | 683–685, 800–801, 649–650, 644, 2951–2952 | Instruction counts equal; result checks 15 cells, 0 mismatches on both. Wire ratios (Pruv/hand16w) 0.17–0.86 on both. |
| m7 perf | typed attach / closereopen (Pruv typed/hand16) | 1.09–1.11 / 1.06–1.09 | 1.11–1.13 / 1.06–1.09 | Unchanged cells, already open with a written reason in PERF_HEADROOM.md; not caused by P07. |
| M4 perf (`perf_headroom/host/run.py --only m4`) | sessions n = 100/500/1000 | 0.85 / 0.78 / 0.79 | 0.87 / 0.80 / 0.80 | |
| M4 perf | tools | 0.57 / 0.62 / 0.61 | 0.56 / 0.62 / 0.60 | |
| M4 perf | clocks | 0.65 / 0.61 / 0.62 | 0.66 / 0.61 / 0.62 | |

Lesson: an earlier P07 draft wrote `c32`, `c40`, `hd`, `set_hd` inline instead of calling ent_u16. That changed helper body sizes, so the text-size inline policy (codegen `inline_policy`, INLINE_SMALL 8, INLINE_BUDGET 1500) moved `#[inline(always)]` from `bud_world::put_blob` to `make_room`, and M4 sessions rose to 1.03x at n = 100 (from 0.89x). Keeping the bodies and putting the exact `ensures` on the shared lib helpers kept the generated code equal to HEAD except the two constructor checks. Contract-only changes should not rewrite bodies on hot paths.

## Appendix P09. Raw-entry visibility and the poison store

P09 applies the manifest key `raw_entries` (docs/G4B_SPEC.md §3 rule 7, recorded as `Program.raw_entries` in P06) in both Rust-producing paths of both compilers: the Rust backend (pruvc/codegen.py, pruvc-rs codegen.rs) and the safe wrappers of the LLVM backend's Rust bindings (pruvc/llvmgen.py `gen_bindings`, pruvc-rs bindings.rs). It closes D6 (§2.3) for Rust hosts: resident state lives in a store that refuses every reuse after a panic. With the key missing, every output is byte-identical to before (diff_rust, g8 compatibility hashes and all `.rs` snapshots unchanged). User rules: SPEC.md "Raw entries (manifest `raw_entries`)".

### P09.1 Design

| Item | Rule |
|---|---|
| Visibility | With `raw_entries` set, every item that was emitted as `pub fn <host entry>` is emitted as `pub(crate) fn <host entry>`: the explicit host entry without `requires`, the checked wrapper of an entry with `requires` (its body `e__pruv_body` and, under `--trust-callers`, `__pruv_chk` stay private), and the safe wrapper in LLVM bindings. Nothing else changes. Non-explicit programs ignore the key (a manifest program is always explicit). |
| Inline policy | `inline_policy` (py and rs) treats a `pub(crate) fn ` header exactly like `pub fn `, so inline hints match the `pub` build. The `pruv_raw` module is appended after `inline_policy` ran and never changes inlining. |
| `pruv_raw` module | Appended at the end of the Rust backend output, and in bindings after the last safe wrapper and before the G6 shim (the shim stays last). It holds `PruvPoisoned` and `PruvStore<T>` (private fields `data`, `poisoned`; `new`, `is_poisoned`, `with`) and, in `"crate"` mode, nothing else; it has no glob import, so a user item named `PruvStore`, `Result` or `entries` cannot collide with it. In `"unsafe"` mode the forwarders live in a private child module `pruv_raw::entries` (`use super::super::*;`, bodies call `super::super::NAME(..)`) re-exported by `pub use self::entries::*;`, so forwarder signatures resolve user types exactly as the entries do. |
| Poison (D6) | `PruvStore::with(f)` refuses with `Err(PruvPoisoned)` if poisoned, otherwise sets the flag, runs `f(&mut data)`, clears the flag and returns `Ok`. The flag is cleared only on normal return: a panic inside `f` (partial write, allocation-size overflow, host panic) unwinds past the clear, so a caught panic leaves the store poisoned and every later `with`, mutating or observing, is refused without running its closure. There is no other accessor to `data`, so poisoned state is never observed or reused. Drop only releases the buffers (`Vec<i64>`/`Vec<u8>` drop never reads element values), so dropping a poisoned store is safe. |
| Trust boundary | The crate is the trust boundary: code in the same crate can call a `pub(crate)` entry with any array, exactly as it could call today's `pub fn`; that code is the adapter and is trusted. What the key removes is safe access from outside the crate (strictly tighter than without the key). |
| Adapter contract | Two rules the types cannot enforce, stated in the `PruvStore` doc comment of both compilers: `T` owns the state (no reference, `Rc` or other shared handle to it outside the store), and the closure given to `with` never catches a panic (a `catch_unwind` inside it would return normally and clear the flag; the same contract shape as std `Mutex` poisoning). |
| `"crate"` | Entries are `pub(crate)`; a host adapter in the same crate (the module next to the generated file, as in bench/l3/budget/host.rs) owns the store and calls entries only inside `with`. Code outside the crate cannot name any entry (E0603). |
| `"unsafe"` | Same, plus one `pub unsafe fn NAME` forwarder per host entry inside `pruv_raw` (program function order, same parameter and return text as the entry), each with a `# Safety` section: the caller upholds the entry's `requires` (checked at run time unless `--trust-callers`), passes only state produced by this program's entries and not changed since, and never passes again a state that was being changed when a panic unwound (keep it in a `PruvStore`). |
| No safe arbitrary-buffer entry | In no mode does the output contain a `pub` safe function that takes a caller-supplied array: the only `pub` safe items are the generic `PruvStore` methods, which never call an entry. Without the key, today's `pub fn` entries remain (G4B compatibility); a program that wants the guarantee sets the key. |
| Requires run once | Without `--trust-callers` the `pub(crate)` checked wrapper checks `requires` once per call; the adapter does not re-check. The P06.3 note about duplicated arguments in expanded requires therefore costs one evaluation per call, at the entry. |
| `--trust-callers` | Does not require `raw_entries`: single-file builds (no manifest) and about 40 tests and benchmarks use it. SPEC.md states that a trust-callers entry is safe only when every caller is trusted; `raw_entries` is how a package makes that enforced by visibility. |
| No unwind into C | The Rust backend emits no `extern "C"` item. LLVM bindings call the C object, which never unwinds (host callbacks abort on allocation failure). A C-facing adapter with `catch_unwind` is P10. |
| Line-559 note (P06.3) | Kept as is: P09 publishes no contract across packages, so visibility of expansions stays an interface-stability policy. |

### P09.2 Evidence (C)

| Category | Files | Result |
|---|---|---|
| Text facts | tests/l3/abi/check_raw_entries.py (a): keys none/`crate`/`unsafe` × flags default/`--trust-callers`/`--no-verify` × backends rust/llvm | No key: `pub fn` entries, no `pruv_raw`. With a key: no top-level `pub fn`, `pub(crate) fn` per entry, `pub mod pruv_raw`, `PruvStore`/`PruvPoisoned`; `unsafe` has exactly one `pub unsafe fn` and one `# Safety` per entry, `crate` none. With `--rs`, every `.rs`/`.ll` is byte-identical to the Python compiler's. |
| Compile-time rejection | (b): generated code built as an rlib, consumer crates compiled against it | Calling an entry from outside: `crate`/`unsafe` E0603 (no key: compiles); `pruv_raw::put` without `unsafe`: E0133 (`unsafe`), E0425 (`crate`); inside `unsafe {}`: compiles (`unsafe`), E0425 (`crate`); reading `PruvStore.data`: E0616. |
| Panic injection | (c) tests/l3/abi/raw_host.rs, `crate`, `--no-verify` package with `put_far` (writes `st[0]`, then indexes out of range) | Partial write, allocation failure (capacity overflow; a real OOM aborts and cannot unwind), host panic after an entry wrote: each is caught, the store is poisoned, a later mutating `with` and a later observing `with` are refused without running, the store is dropped. Normal calls on a fresh store never poison. |
| Name clash | (d): a package with a user `pub struct PruvStore { x: i64 }`, an entry `read(v: PruvStore)` without `requires` and an entry with `requires`; keys `crate`/`unsafe` × rust/llvm | Generated code compiles as an rlib in every case; `unsafe` has forwarders `read`, `get` and a consumer calls both (`gen::pruv_raw::read(gen::PruvStore { x: 3 })`). |
| Runner | tests/run_all.py group `l3_abi` (one row; passes the selected compiler as `--rs` or `--py`) | PASS on py and rs; direct `--rs` run: every output identical to py. |
| Rejects | tests/must_reject/l3_p09_result_param (expandable callee with a parameter `result: i64`; the automatic body fact keeps the argument value, so `a[bump(1)]` is index 2 of a length-2 array), l3_p09_old_param (the same with a parameter named `old`), l3_p09_nested_call (`a[inc(inc(1))]`, length 3: each nested call carries its own body fact) | verify/index_oob at 8:11 on both compilers; must_reject 852/852 on both. |
| Bench adapter | bench/l3/budget/host.rs (ResidentState poisons on entry and clears on normal exit, `is_poisoned`), m34_test/driver_m4.rs + run.py (`m4raw`: the M4 driver over a `raw_entries = "crate"` package with `pruv_raw::PruvStore`) | m34 keys 5823/5823, m3 300/300, m4 300/300, m4raw 300/300 on both compilers. |

Gates (both compilers unless noted): no-key byte identity against the HEAD Python compiler for every bench/l3/budget root, m34 root and example, rust and llvm (160/160 identical); `cargo test --release`; diff_rust `--fuzz-count 20` all same; `--only g8_frontend` py 27/27, rs 39/39; list-rows/compare-rows; budget roots root_keys, root_m3, root_m4, root_m8, root_m9 verify with identical `.rs`; L3 lib slices json a0–a5, mcp, qwen, supervisor a0–a3; run_m7 q 113 exact / 0 fail, run_m10 q 78 exact / 0 fail, run_m6, run_m11 (213 exact, 87 prefix-ok).

### P09.3 Perf (P)

Rust driver, `perf_headroom/host/run.py --only m4` (load average about 3, so cells move by a few percent between runs). The M4 driver changed in P09: every state call now goes through a store with the `PruvStore` API (m34_test/driver_m4.rs). The default build `m4` uses `Plain<T>`, which has no poison flag and inlines to a direct call, so the first table measures the same work as P07.4. (bench/l3/budget/host.rs `ResidentState`, used by the other budget hosts, also gained a poison flag; M4 does not use it.)

| axis | n | Pruv ns | hand ns | Pruv/hand | P07.4 |
|---|---:|---:|---:|---:|---:|
| sessions | 100 / 500 / 1000 | 12.9 / 12.6 / 13.1 | 14.4 / 15.7 / 15.7 | 0.90 / 0.80 / 0.83 | 0.87 / 0.80 / 0.80 |
| tools | 100 / 500 / 1000 | 30.4 / 33.2 / 32.4 | 52.3 / 52.7 / 53.3 | 0.58 / 0.63 / 0.61 | 0.56 / 0.62 / 0.60 |
| clocks | 100 / 500 / 1000 | 25.9 / 24.6 / 25.1 | 37.5 / 38.9 / 39.2 | 0.69 / 0.63 / 0.64 | 0.66 / 0.61 / 0.62 |

Poison cost (median of 7 alternating rounds; m4 = the plain driver, m4raw = the same calls through a `raw_entries = "crate"` package and one `pruv_raw::PruvStore::with` per call):

| axis | n | hand | m4 | m4raw | m4/hand | m4raw/hand | m4raw/m4 |
|---|---:|---:|---:|---:|---:|---:|---:|
| sessions | 100 | 14.6 | 12.8 | 14.6 | 0.88 | 1.00 | 1.141 |
| sessions | 500 | 15.6 | 12.6 | 14.3 | 0.81 | 0.92 | 1.135 |
| sessions | 1000 | 16.0 | 13.0 | 15.1 | 0.81 | 0.94 | 1.162 |
| tools | 100 | 52.9 | 30.5 | 31.8 | 0.58 | 0.60 | 1.043 |
| tools | 500 | 54.3 | 33.6 | 35.9 | 0.62 | 0.66 | 1.068 |
| tools | 1000 | 54.7 | 32.4 | 35.3 | 0.59 | 0.65 | 1.090 |
| clocks | 100 | 37.6 | 25.5 | 26.7 | 0.68 | 0.71 | 1.047 |
| clocks | 500 | 40.3 | 24.9 | 26.4 | 0.62 | 0.66 | 1.060 |
| clocks | 1000 | 39.2 | 25.2 | 26.9 | 0.64 | 0.69 | 1.067 |

Every cell is at or under 1.0x hand Rust, so no cell above 1.1x needs an explanation. The poison store costs 1.2 to 2.9 ns per call (two flag stores and the `Result` check around one call). It is 13 to 16 % on sessions, whose call is only about 13 ns, and 4 to 9 % elsewhere. Even with that cost the worst cell (sessions n=100) is 1.00x hand. The hand side has no poison store; the m4raw/hand column therefore compares a poisoning Pruv adapter with a non-poisoning hand loop. Correctness of these drivers is m34 m4raw 300/300 (the perf driver only keeps a sink so the work is not optimized away).

### P09.4 Audit

One audit (Codex) found ten items. Fixed: the unsafe-mode glob import that could collide with user types (now the `entries` child module), `raw_items` collected without the key (now only with it, all six sites), the run_all row ignoring a Python compiler path, no test for a no-requires entry or a user `PruvStore` (probe (d)), rustc taken only from ~/.cargo/bin (now PATH first). Documented: the crate as the trust boundary, and the two adapter rules (`T` owns the state; no `catch_unwind` inside `with`). Unchanged by design: an entry that returns a refusal status does not poison the host's store, because a status is a normal return of verified code and its invariants hold on refusal exits; poisoning is for panics. A confirmation audit of the three code fixes (adapter-rule docs, `entries` child module, guarded `raw_items`) found them fixed and no new must-fix item.

### P09.5 Notes for P10/P10H

| Package | Note |
|---|---|
| P10 | The C-facing adapter: an opaque handle that owns a `PruvStore` (or its LLVM equivalent), `extern "C"` functions that wrap every call in `catch_unwind` and return an error code, and a poisoned handle that refuses every call but still frees. Rust-backend output has no `extern "C"` today; keep it that way and put the boundary in the adapter. |
| P10 | LLVM backend: bindings now get `pub(crate)` wrappers and `pruv_raw`; the handle should own the C-side state so that a C caller never passes an arbitrary buffer. Host callbacks must keep aborting (never unwinding) on allocation failure. |
| P10 | Per-call `valid(st)` at raw entries stays until the handle owns the store (P07.3): only then can construction establish the header fact once. |
| P10H | `PruvStore::with` costs one flag store before and after the call (P09.3). If a hot adapter batches many calls, run the batch inside one `with` instead of one `with` per call. |
| P10H | `"unsafe"` forwarders are `#[inline(always)]` and add no code; if P10H exposes a typed handle API, prefer `"crate"` and keep the `unsafe` forwarders for hosts that manage state themselves. |

## Appendix P10. C adapter handle

P10 adds the third value of the manifest key `raw_entries`, `"c"` (pruvc/manifest.py, pruvc-rs manifest.rs, docs/G4B_SPEC.md §3 rule 7). It closes D6/D7 (§2.3) for C hosts: a C caller holds one opaque pointer, never sees or passes the resident state buffer, and no unwind crosses into C. The Rust backend output for `"c"` is byte-identical to `"crate"` and has no `extern "C"`; the C boundary exists only in the LLVM backend's Rust bindings (pruvc/llvmgen.py `c_param_kind`/`c_entries`/`c_module`, pruvc-rs bindings.rs `CKind`/`CEntry`/`c_module`). With the key missing or `"crate"`/`"unsafe"`, every output is unchanged. User rules: SPEC.md "Raw entries (manifest `raw_entries`)".

### P10.1 Design

| Item | Rule |
|---|---|
| Module | LLVM bindings with `"c"` end with the `"crate"` output (`pub(crate)` safe wrappers, `pruv_raw`) followed by `pub mod pruv_c`, before the G6 shim. Both compilers emit it from the same text (`C_HEAD` plus one block per entry); the module is appended after `inline_policy`, so it never changes inlining of program code. |
| Handle | `PruvHandle { store: pruv_raw::PruvStore<Vec<i64>> }`, opaque to C (one pointer, `Box::into_raw`). `pruv_handle_new(len)` makes `len` zero words (null for `len < 0` or `len * 8 > isize::MAX`, or if allocation panics); the program's init entry runs next as an ordinary `pruv_h_` call. `pruv_handle_free(h)` drops the box whether or not the store is poisoned (a `Vec<i64>` drop never reads the words); null is ignored. `pruv_handle_poisoned(h)` is 1/0, 2 for null. |
| Entries | Every host entry whose first parameter is `[i64]` (`in` or `inout`; the handle's state), whose other parameters are `i64`, `bool`, `str`, `[i64]`, `[u8]` or `[bool]` (`in` or `inout`), and whose result is `i64`, `bool` or unit gets `pruv_h_NAME(h, …) -> i32`. Arrays are (pointer, length) pairs, `bool` is an `i64` that must be 0 or 1, and an `i64`/`bool` result is written to `*ret` only on `PRUV_OK`. Each function has a fixed parameter count: no registry, no variadic call, no SoA public entry. Entries outside this shape get no C function (they stay reachable from Rust through the `pub(crate)` wrapper). C parameters are named by position (`a0`, `a1`+`a1_len`, …; the handle is `h`, the result pointer `ret`), so no user parameter name is a Rust identifier in the adapter and none can shadow `h`, `ret`, the closure's state or a helper; the doc comment maps each `aN` to the user's name. |
| Rebuild | An entry `(prev: [i64], inout next: [i64]) -> i64` (M4's `m4_grow`) also gets `pruv_hr_NAME(h, new_len, ret)`: inside one `with`, it allocates `new_len` zero words, runs the entry from the state into them, and replaces the state only when the entry returns 0 (the old buffer is freed); any other result leaves the state unchanged. C never holds either buffer, so a grow cannot alias or leak the state. |
| Statuses | 0 `PRUV_OK`; 1 `PRUV_POISONED` (refused right after the null-handle check, before any other argument is read; nothing ran); 2 `PRUV_BAD_ARGS` (refused, nothing ran: null handle or `ret`, misaligned `ret`, a `ret` whose end address overflows, negative length, a length whose byte size exceeds `isize::MAX`, null or misaligned pointer with a nonzero length, a range whose end address overflows, a `bool` other than 0/1, a `[bool]` byte other than 0/1, invalid UTF-8 in `str`, two byte ranges that overlap where at least one is written, including `ret` and the handle object itself; for `pruv_hr_` a negative or oversized `new_len`); 3 `PRUV_PANICKED` (a Rust panic unwound to the adapter; inside the store it also poisoned the handle). A refusal status returned by the program itself is a normal `PRUV_OK` result in `*ret` (P09.4: statuses do not poison). |
| No unwind (D7) | Every `extern "C"` body runs inside `guard` (`catch_unwind` + `AssertUnwindSafe`); a caught payload is `mem::forget`-ed, so its destructor (which could itself panic) never runs. The entry call runs inside `PruvStore::with`, so a Rust panic that unwinds out of it poisons the store before `guard` catches it. `pruv_handle_new` catches allocation panics too. Not every failure is a status: a checked failure inside LLVM-compiled code (range, arithmetic, `requires` at a callee) and a G6 host-callback failure call the runtime's exit path (`exit(101)`), and a real OOM aborts (Rust `handle_alloc_error`, runtime.c host callbacks). Those end the process; they never unwind and never return a status. Status 3 therefore means a recoverable Rust unwind only. |
| Poison (D6) | A poisoned handle refuses every `pruv_h_`/`pruv_hr_` call with status 1 before checking or touching any other argument (a bad `bool`, invalid UTF-8, a null `ret` or a bad `new_len` on a poisoned handle is still 1), without running the entry, and still frees. There is no reset: a C host that sees status 3 or 1 frees the handle and builds a new one. |
| Provenance | Argument slices are built only after every check passed; length 0 never reads the pointer. `ro`/`mt` reject a range whose end address `p + len * size_of::<T>()` overflows (`checked_add`). Every argument buffer and `ret` must also be apart from the handle object (`span(h, 1)`, counted as written), so a C buffer cannot alias the store's `Vec` header or poison flag; the state words live in the handle's own heap buffer, which C never receives. Each byte range (`span`, `saturating_add`) is computed once into a local (`sp_aN`, `sp_ret`, `sp_h`) and `apart` compares two ranges by address; empty ranges never overlap. |
| Caller contract | The adapter checks what it can see: null, sign, alignment, byte size, address overflow and overlap among one call's buffers and the handle. It cannot see allocation size, initialization or provenance. The `pruv_c` module docs state the contract and every `extern "C"` function has a `# Safety` section pointing at it: `h` came from `pruv_handle_new` and was not freed; one handle is not used from two threads at once or reentered from a callback; each (pointer, length) with length > 0 points to `length` initialized elements inside one live allocation, readable (writable for `inout`) and not accessed by anyone else during the call; `ret` points to a writable `int64_t`. |
| Misuse not checked | A double free, a call after free, or a pointer that never came from `pruv_handle_new` is undefined behaviour the adapter cannot detect without a registry (a registry would add a lookup to every call and a lock for threads). A C caller must not call one handle from two threads at once (each call takes `&mut` of the handle; same rule as any unsynchronized C object); different handles are independent. Symbols are global (`pruv_h_NAME`), so one `"c"` program per linked image. |
| Rust backend | `"c"` maps to `"crate"` in both codegens: no `extern "C"`, no `pruv_c`. A Rust host keeps using `PruvStore` directly (P09). |

### P10.2 Evidence (C)

| Category | Files | Result |
|---|---|---|
| Text facts and identity | tests/l3/abi/check_c_handle.py (keys `crate` and `c`) | LLVM bindings: `crate` has no `pruv_c` and no `extern "C" fn pruv_h_`; `c` has `pub mod pruv_c`, `pruv_handle_new/free/poisoned`, one `pruv_h_` per eligible entry, `pruv_hr_grow`, and still `pub mod pruv_raw` (the `//!` caller contract and the `# Safety` sections are emitted but not asserted by the script). Rust backend: the script also compiles the program with `--manifest` for both keys; the `.rs` for `c` equals `crate` and has 0 `extern "C"`. With `--rs`, the LLVM-backend program `.ll` and bindings `.rs` are byte-identical to the Python compiler's; the Rust-backend check is `c` == `crate` within the selected compiler only. |
| C probe | tests/l3/abi/c_probe.c, linked against the bindings staticlib (`rustc --cfg pruv_c_probe`), the program `.ll` object and runtime.c | ABI: every function called from C with its declared prototype; `new(-1)`/`new(INT64_MAX)` null; zero-state call before init. Names: `clash(state, st, h, ret, ro, ro_len)` uses every generated name as a user parameter and returns the exact value built from the outer `st[0]` (not the resident state). Types: `[u8]` sum and `str` length, both with length 0 and a NULL pointer; `inout [bool]` flipped in place. Provenance: in the groups that check them (op, clash, `[u8]`/`str`, the BAD_ARGS table, rebuild), sentinel words before and after the out buffers and `ret` stay unchanged on refusal and `in` buffers do not change. Lifetime: state persists across calls; rebuild with an entry result ≠ 0 keeps the state, with 0 commits it, the state length read back by an entry goes 4 → 8 and the values carry over; a second handle is independent; free of a poisoned handle and of null. BAD_ARGS: null handle/`ret`, misaligned `ret`/arrays, negative lengths, null pointer with length 1, four overlap shapes (in = out, partial, out = `ret`, in = `ret`), adjacent ranges accepted, end-address overflow (`i64`, `u8`, `str`), `i64` length above `INT64_MAX/8`, every buffer and `ret` overlapping the handle object, `bool` 2 and −1, a `[bool]` byte 2 (in and inout), invalid and truncated UTF-8, rebuild `new_len` −1/`INT64_MAX`/`INT64_MIN`/above `INT64_MAX/8` and `ret` null/misaligned/inside the handle; none poisons. Poison: `pruv_handle_probe_panic` panics inside `with` with a payload whose `Drop` panics → status 3 (payload forgotten, no abort), `poisoned()` 1; then a mutating entry, a query, a unit entry and a rebuild each return 1, and so do these bad-argument shapes: bad `bool`/`[bool]`, invalid UTF-8, null `ret`, handle overlap, bad `new_len`, each leaving its buffers untouched. Not probed: a zero-length NULL `i64` array (no eligible entry of the probe takes an `i64` array that may be empty; `ro`/`mt` share the length-0 branch with `[u8]`/`str`). |
| Runner | tests/run_all.py group `l3_abi`, row `l3_abi/c_handle` (passes the selected compiler as `--rs` or `--py`) | PASS on py and rs. |
| Bench adapter | bench/l3/budget/m34_test/driver_m4c.rs + run.py lane `m4c` (the M4 World over `pruv_c` symbols, called through `black_box`-ed function pointers so each call is a real C call) | m34 `m4c` 300/300 (922 grows, 1922/1922 stats checks), same as `m4`, `m4raw`, `m4llvm`, `m4llvmraw`. |

### P10.3 Perf (LLVM path)

M4 host (`bench/l3/budget/perf_headroom/host/run.py --only m4 --rounds 7`), Apple M-series, median of 7 alternating rounds of 200000 reps, ns per op. Every lane reports `total_ns_per_op` (grows included) and consumes the same sink `out[0]+out[1]`. Rust is `rustc -C opt-level=3`, the LLVM object is `clang -O2`. The numbers were taken before the `ret` end-address check (P10.5, confirmation audit) was added; that check is one compare added to both `m4c` and `handc`. Baselines:

- `hand`: hand Rust, state and `exec` called in-process. Matched baseline of every in-process lane.
- `handc`: the same hand Rust behind its own `extern "C"` handle (`hand_h_new/exec/free` in bench/l3/budget/perf_headroom/host/src/bin/m4_hand.rs): null handle, poison flag, `ret` null/alignment/end address overflow, the six pairwise overlap checks of op/out/`ret`/handle, `ro`/`mt` slice checks, `catch_unwind`, called through `black_box`-ed function pointers like `m4c`. Matched baseline of `m4c`.

| lane | backend | raw_entries | per-call `valid` | eq_range | matched to |
|---|---|---|---|---|---|
| `m4` | Rust | none (plain store) | yes | builtin | hand |
| `m4raw` | Rust | `"crate"` (PruvStore) | yes | builtin | hand |
| `m4eq` | Rust | none | yes | loop `eqr_p10` | hand |
| `m4nv` | Rust | none | no | builtin | hand |
| `m4llvm` | LLVM | none | yes | loop | hand |
| `m4llvmraw` | LLVM | `"crate"` | yes | loop | hand |
| `m4c` | LLVM | `"c"`, driver calls `pruv_h_*` through the handle | yes | loop | handc |
| `m4llvmnv` | LLVM | none | no | loop | hand |

The LLVM backend rejects `eq_range` until P17a, so every LLVM lane (and `m4eq`, the like-for-like Rust lane) uses a verified loop in its place.

| axis | n | hand | handc | m4 | m4raw | m4eq | m4nv | m4llvm | m4llvmraw | m4c | m4llvmnv | m4c/handc (matched) | m4c/hand (unmatched, info) |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| sessions | 100 | 14.7 | 16.5 | 12.9 | 14.5 | 11.8 | 11.8 | 20.5 (1.39) | 20.3 (1.38) | 24.7 | 17.4 (1.18) | **1.50** | 1.68 |
| sessions | 500 | 16.1 | 18.1 | 12.6 | 14.2 | 11.4 | 11.7 | 20.3 (1.26) | 20.3 (1.26) | 24.4 | 17.3 (1.07) | **1.35** | 1.52 |
| sessions | 1000 | 16.3 | 18.0 | 13.0 | 14.7 | 11.9 | 12.1 | 20.7 (1.27) | 20.5 (1.26) | 24.9 | 17.8 (1.09) | **1.38** | 1.53 |
| tools | 100 | 52.8 | 55.9 | 29.9 | 31.5 | 27.9 | 29.6 | 40.2 | 40.4 | 44.6 | 37.2 | 0.80 | 0.84 |
| tools | 500 | 53.1 | 55.9 | 33.4 | 34.9 | 31.9 | 32.7 | 43.4 | 43.5 | 47.7 | 40.5 | 0.85 | 0.90 |
| tools | 1000 | 52.8 | 55.7 | 32.3 | 34.2 | 31.2 | 31.5 | 42.6 | 42.5 | 46.9 | 39.6 | 0.84 | 0.89 |
| clocks | 100 | 37.2 | 41.4 | 25.2 | 27.3 | 22.8 | 23.9 | 38.1 (1.02) | 37.3 (1.00) | 42.1 | 34.7 | 1.02 | 1.13 |
| clocks | 500 | 38.7 | 42.0 | 24.5 | 26.5 | 22.5 | 23.0 | 35.6 | 35.6 | 40.5 | 32.9 | 0.96 | 1.05 |
| clocks | 1000 | 38.9 | 41.9 | 25.0 | 27.0 | 21.2 | 23.8 | 38.8 | 38.7 | 43.0 | 36.2 | 1.03 | 1.11 |

Ratios of in-process lanes to `hand` are shown only at or above 1.0x; every Rust-backend lane is below hand (0.53x to 0.99x; `m4raw/m4` 1.04x to 1.13x). Matched cells above 1.1x: `m4c/handc` sessions (1.35x to 1.50x), and in-process `m4llvm`/`m4llvmraw` sessions (1.26x to 1.39x) and `m4llvmnv` sessions n=100 (1.18x). No clocks or tools cell is above 1.1x matched.

**Decomposition of `m4c - handc`.** `m4c - handc = (m4eq - hand) + (m4llvmraw - m4eq) + (m4c - m4llvmraw) - (handc - hand)`: Pruv work on the Rust backend, the LLVM path, the generated C crossing, minus the hand C crossing. Every term is a difference of two table cells, so each row sums exactly:

| axis | n | m4c - handc | m4eq - hand | m4llvmraw - m4eq (LLVM path) | m4c - m4llvmraw (generated crossing) | handc - hand (hand crossing) | generated - hand crossing |
|---|---|---|---|---|---|---|---|
| sessions | 100 | +8.2 | −2.9 | +8.5 | +4.4 | 1.8 | +2.6 |
| sessions | 500 | +6.3 | −4.7 | +8.9 | +4.1 | 2.0 | +2.1 |
| sessions | 1000 | +6.9 | −4.4 | +8.6 | +4.4 | 1.7 | +2.7 |
| tools | 100 | −11.3 | −24.9 | +12.5 | +4.2 | 3.1 | +1.1 |
| tools | 500 | −8.2 | −21.2 | +11.6 | +4.2 | 2.8 | +1.4 |
| tools | 1000 | −8.8 | −21.6 | +11.3 | +4.4 | 2.9 | +1.5 |
| clocks | 100 | +0.7 | −14.4 | +14.5 | +4.8 | 4.2 | +0.6 |
| clocks | 500 | −1.5 | −16.2 | +13.1 | +4.9 | 3.3 | +1.6 |
| clocks | 1000 | +1.1 | −17.7 | +17.5 | +4.3 | 3.0 | +1.3 |

(sessions n=500: −4.7 + 8.9 + 4.1 − 2.0 = 6.3 = 24.4 − 18.1.)

**Sessions `m4c/handc` 1.35x to 1.50x, explained term by term.**

- **LLVM path, +8.5 to +8.9 ns (the largest term; not P10 code).** The same lane without any C adapter, `m4llvmraw`, is already 1.26x to 1.38x of hand on sessions. This term changes three things at once: the code generator (LLVM IR vs Rust), the optimizer (`clang -O2` vs `rustc -C opt-level=3`), and the call boundary (a separate object behind a non-inlinable Rust → C call with each slice as a pointer/length pair, no LTO, where hand Rust and `m4eq` inline into the caller). The split between them is not measured; that the call boundary dominates on sessions (the least work per op, 12 to 16 ns) is a hypothesis. It predates P10; the control that separates it (the same `.ll` linked with `-C linker-plugin-lto`) is P17 work (P10.6).
- **Generated crossing minus hand crossing, +2.1 to +2.7 ns (P10 code).** Both crossings run the same steps in the same order. Hand (`hand_h_exec`) against generated (`pruv_h_m4_op`), line by line:

| step | hand | generated |
|---|---|---|
| panic boundary | `catch_unwind(AssertUnwindSafe(\|\| …))` | `guard(\|\| …)` = the same `catch_unwind`, payload `mem::forget` |
| handle | `if h.is_null() { return 2 }` | `match h.as_mut() { None => return PRUV_BAD_ARGS }` |
| poison | `if (*h).poisoned { return 1 }` | `if hd.store.is_poisoned() { return PRUV_POISONED }` |
| `ret` | `ret.is_null() \|\| ret % 8 != 0 \|\| ret + 8` overflows | same text |
| ranges | `span(p, len)` once per buffer, `+ len.max(0) * 8` saturating | `let (sp_a0, sp_a1, sp_ret, sp_h) = (span(..), ..)`: same formula, `size_of::<T>()` |
| overlap | six `spans_overlap(a, b)` joined by `\|\|` | six `apart(a, b)` (`!spans_overlap`) joined by `&&` |
| slices | `ro(op, op_len)`, `mt(out, out_len)` | `ro(a0, a0_len)`, `mt(a1, a1_len)`: same checks, generic over `T` |
| call | `(*h).poisoned = true; st.exec(op, out); (*h).poisoned = false` | `hd.store.with(\|st\| super::m4_op(&mut st[..], a0, a1))`: same flag writes |
| result | `*ret = status; 0` | `*ret = v; PRUV_OK` |

  The one structural difference is the callee: hand's `exec` is Rust in the same crate, `m4_op` is the LLVM object behind a second non-inlinable call (the same boundary as the LLVM-path term). Variant builds of `m4c` (bench/l3/budget/work/m34/gen_m4c.rs edited and relinked against the same `.o`; median of 7 alternating rounds; sessions n=500 / clocks n=1000, ns):

| variant | sessions 500 | clocks 1000 |
|---|---|---|
| generated as before P10.3 (spans recomputed per pair) | 24.4 | 42.6 |
| `guard` replaced by a plain call (no `catch_unwind`) | 24.3 | 42.5 |
| no overlap checks, no `ro`/`mt` checks (slices straight from the pointers) | 20.4 | 38.8 |
| spans computed once, hand-form `apart` (adopted) | 23.3 | 41.6 |
| `m4llvmraw` (no adapter) | 19.6 | 37.5 |
| `handc` / `handc` without overlap and slice checks | 17.6 / 16.4 | 40.6 / 39.7 |
| same with hand `exec` `#[inline(never)]` | 17.6 / 16.3 | — |

  So the panic boundary costs nothing and the adapter frame without checks costs +0.8 to +1.3 ns over `m4llvmraw`. The no-check rows are not matched: the generated variant also dropped the `ret` check, the hand variant kept it, so "about 3 to 4 ns of checks generated against about 1 ns hand" is indicative only. Recomputing each range per pair was a real inefficiency (`pruv_h_m4_op` was 194 lines of arm64 assembly against 102 for `hand_h_exec`); computing each range once (both emitters, P10.1 Provenance) cut it to 114 lines and about 1 ns. Further forms measured within noise of that: branch-free `&` of `apart`, and validating each array into a `checked_add` range before the overlap test. The remaining 2.1 to 2.7 ns (generated minus hand crossing) has the same checks in the same order as hand; the one structural difference is the LLVM callee, but no measurement attributes the residual to it, so its cause is open.
- **Pruv work, −2.9 to −4.7 ns.** The program itself is faster than hand on the Rust backend on every cell (`m4eq/hand` 0.53x to 0.80x).

Clocks n=100 and n=1000 are 1.13x and 1.11x unmatched (`m4c/hand`) and 1.02x and 1.03x matched; tools is below 1.0x either way.

- **`m4llvm`/`m4llvmraw` sessions (1.26x to 1.39x).** The LLVM-path term above; no P10 code. `m4llvmraw - m4llvm` is −0.8 to +0.2 ns on all nine cells, so the poison store costs nothing measurable on the LLVM path (on the Rust path `m4raw - m4` is +1.5 to +2.1 ns, still below hand).
- **`m4llvmnv` sessions n=100 (1.18x).** The LLVM-path cost minus the per-call `valid` (`m4llvm - m4llvmnv` is 2.6 to 3.4 ns; `m4 - m4nv` on Rust is 0.3 to 1.5 ns; see P10.4).

Per-op work in M4 is 12 to 50 ns, so a fixed per-call cost shows up as a ratio. **Open:** sessions `m4c/handc` 1.35x to 1.50x is above the 1.1x floor. The measured terms are the LLVM-path term (+8.5 to +8.9 ns, backend and call boundary not yet separated) and the generated-over-hand crossing (+2.1 to +2.7 ns, residual not attributed). Both go to P10H/P17 with the controls in P10.6; the C path also benefits from crossing less often (several operations inside one `with`, P10.6).

### P10.4 Item 4: per-call `valid(st)` stays

The spec allowed dropping the per-call `valid(st)` check at raw entries if the handle makes it safe. It does not, so the check stays:

- A handle starts as `len` zero words, which are not `valid` (word 0 is not the magic); it becomes valid only after the program's init entry returns 0. A C caller may call any entry before init, after a failed init, or after a rebuild the program refused. The entry-side check is what makes those calls return the program's own status instead of reading garbage.
- None of the 38 `pub` entries with `inout st` in the budget libs ensures `valid(st)` on exit (bud_map 0/5, bud_world 0/9, bud_clock 0/14, bud_lifecycle 0/7, bud_tools 0/2, bud_halt 0/1). Without that fact (P07.3, P11), the handle cannot carry validity from one call to the next.
- `pruv_hr_` commits any buffer the entry returns 0 for. For M4's `m4_grow` that buffer is already valid: `grow_copy` ensures `result != 0 || valid(nxt)`, so a committed rebuild keeps validity. Rebuild is not the reason the check stays; the two reasons are the ones above: nothing records that init returned 0, and general mutators have no `valid(old(st)) ==> valid(st)` contract.

Cost of keeping it: lanes `m4nv` (Rust backend, no per-call `valid`) and `m4llvmnv` (LLVM, no per-call `valid`) in P10.3. The check can go when P11 adds `valid(old(st)) ==> valid(st)` to every mutator and the handle records "init returned 0" (a word the C side cannot set).

### P10.5 Audit

First audit (Codex gpt-6-astra, ultra): not approvable as first submitted. Both emitters agreed, so every defect was shared. Each finding and its fix:

| Finding | Fix |
|---|---|
| M1 Generated names (`h`, `ret`, `st`, `NAME_len`, `ro`) could collide with user parameter names; `probe(state, st)` passed the resident state instead of the outer array | C parameters are positional (`aN`, `aN_len`) in both emitters; probe entry `clash(state, st, h, ret, ro, ro_len)` (P10.1 Entries, P10.2) |
| M2 A poisoned handle checked `bool`, UTF-8, `new_len` and `ret` before refusing, so it answered 2 instead of 1 | `is_poisoned()` right after the null-handle check in every `pruv_h_`/`pruv_hr_`; the probe repeats every bad-argument shape on a poisoned handle (P10.1 Poison) |
| M3 The spec said a callee failure gives status 3 and poisons, but LLVM range/arithmetic/`requires` failures and G6 callback failures call `exit(101)` | P10.1 No unwind separates a recoverable Rust unwind (status 3) from process exit and abort |
| M4 No end-address overflow check, no overlap check against the handle object, no caller contract | `checked_add` in `ro`/`mt`; every buffer and `ret` must be apart from `span(h, 1)`; `//!` caller contract and `# Safety` on every `extern "C"` function (P10.1 Provenance, Caller contract) |
| M5 m4c above 1.1x of hand with wrong explanation numbers, and the runner dropped a failing requested lane silently | Matched baseline `handc` (hand Rust across the same C boundary), per-cell decomposition with exact deltas, span-once overlap check (P10.3); a requested lane that fails to build or run is a hard error |
| M6 Pruv lanes summed `out[0]+k` and excluded grow time; hand summed `out[0]+out[1]` | Every lane reports `total_ns_per_op` (grow included) with the sink `out[0]+out[1]` |
| S1 P10.2 overclaimed (no Rust `c == crate` compile, no `[u8]`/`str`/`inout [bool]`, zero-length NULL, byte-size overflow, oversized rebuild, panicking payload `Drop`, `ret` sentinels, rebuild length) | Probe and script extended; P10.2 lists what is probed and what is not. The confirmation audit narrowed it again (below) |
| S2 Per-call `valid` rationale cited rebuild, but `grow_copy` already ensures `result != 0 \|\| valid(nxt)` | P10.4 rewritten: no init tracking and no preservation contract on general mutators |

Confirmed OK by the first audit: Python/Rust parity of `c_entries`/`c_entry` and `C_HEAD`; Rust backend `"c"` → `"crate"`; length/alignment/null, `bool` and UTF-8 checks; catchable panic with the payload forgotten; rebuild and free semantics; a real C ABI link; Linux `native-static-libs`; rows and manifest.

Confirmation audit (Codex gpt-6-astra, ultra): not confirmed as submitted. Findings and fixes:

| Finding | Fix |
|---|---|
| M4 `ret` had no end-address overflow check (SPEC.md requires BAD_ARGS for an overflowing range) | `ret` check adds `checked_add(size_of::<i64>())` in every `pruv_h_` and `pruv_hr_` (both emitters, byte-identical); `handc` gets the same compare; c_probe.c passes `UINTPTR_MAX - 7` as `ret` to `pruv_h_op` and `pruv_hr_grow` (BAD_ARGS, state unchanged) |
| M5 Attribution of the sessions cells above 1.1x was asserted, not measured; the no-check variants were unmatched; two ranges were wrong | Ranges corrected (`m4raw/m4` 1.04x to 1.13x, `m4llvmraw - m4llvm` −0.8 to +0.2 ns); the LLVM-path term is stated as backend + optimizer + call boundary, not separated; the no-check rows are marked indicative; sessions `m4c/handc` 1.35x to 1.50x is recorded as open (P10.6, P17) |
| S1 P10.2 still claimed more than the script and probe assert | The `//!`/`# Safety` text is not asserted, `--rs` identity covers the LLVM `.ll` and bindings `.rs` only, sentinel claims are limited to the probed groups |

Confirmed OK by the confirmation audit: the first-audit fixes for M1, M2, M3 and M6, S2, emitter parity, all 54 decomposition deltas exact, and `handc` mirroring the generated checks.

### P10.6 Notes for P10H and P17

- **Crossing cost.** The handle adds a fixed +4.1 to +4.9 ns per call over `"crate"` on the LLVM path (`m4c - m4llvmraw`, P10.3), of which the hand C crossing of the same checks is 1.7 to 4.2 ns. P10H should offer an entry that runs several operations inside one `with` (one crossing, one `catch_unwind`) rather than shaving per-call checks.
- **LLVM call boundary (P17).** `m4llvmraw` is +8.5 to +17.5 ns per call over the same program on the Rust backend (`m4eq`); it is the largest term of the sessions cells above 1.1x of hand, but backend, optimizer and call boundary are not separated yet. Inline or LTO the LLVM object into the Rust host, or measure with `-C linker-plugin-lto`, before comparing backends.
- **`eq_range` lowering (P17a).** On the Rust backend the builtin `eq_range` (`memcmp`) is 1.1 to 3.8 ns slower per op than the verified loop for the short ranges in M4 (`m4 - m4eq`). Lower short ranges to an inline loop.
- **Per-call `valid`.** Costs 2.6 to 3.4 ns per call on LLVM (`m4llvm - m4llvmnv`) and 0.3 to 1.5 ns on Rust (`m4 - m4nv`). It can go once P11 gives every mutator `valid(old(st)) ==> valid(st)` and the handle keeps an "init returned 0" word (P10.4).
- **Global symbols.** `pruv_handle_*` and `pruv_h_*` are unprefixed global C symbols, so two Pruv programs linked into one C binary collide. P10H should add a per-program prefix.
- **Misuse.** Double free, use after free and cross-thread use are not checked (no registry, by design). A C wrapper that wants them checked keeps its own table.

## Appendix P10H: budget and req host facts, batched C entry, symbol prefix

### P10H.1 Host facts (bench/l3/hostfact)

Concrete synchronous G6 host facts for budget and req. Decisions stay in Pruv (`root.pruv`); the host crate
`hostfact_host` only answers facts or does side effects and appends each call to a trace. Signatures, entries and
refusal semantics are in `bench/l3/hostfact/README.md`.

| fact | signature | used by |
|---|---|---|
| `running_ids` | `(session, settled: i64) -> Vec<i64> ! alloc, io`, answer `[status, n, ids...]` | `bh_halt` |
| `is_quiet` | `(session) -> i64 ! io` | `bh_close` |
| `release_held` | `(session) -> i64 ! io` | `bh_release` |
| `next_item`, `is_temporary`, `title_obs` | `(s, m) -> i64`, `(item) -> i64`, `(s) -> i64`, all `! io` | `rq_find` |

Gates (13 per build, 4 builds: py/rs x rust/llvm, all pass): query order (the query sees the settled value), a
query-before-settle mutant caught by the trace, result validation (bad status, bad length, bad id, small `out`: no
write to `out` or the halted flag), close/release answers, re-entry refusal (code 9, guard cleared on unwind), req
first match / invalid answer / no match with exact call sequences, and Pruv == hand lanes on every gate input.

### P10H.2 Trusted shim fix (both emitters)

The first grid run had `rq_find` at 1.27x to 2.60x of hand on the Rust backend. Cause: every trusted-fn helper
(`pruv_m_f_*`) was a separate, non-inlined function, and it set the `PRUV_CTX` thread-local on entry and cleared it on
drop even when the fn has no checked contract (`ensures true`). On macOS each thread-local access is a call. Fix in
`pruvc/foreign_codegen.py` and `pruv-core/src/{foreign_codegen,codegen}.rs`, byte-identical output:

- trusted helpers are `#[inline(always)]`;
- the `pruv_ctx` guard and the budget line are emitted only when at least one requires/ensures check line is emitted.
  `pruv_ud` (only reachable from contract expressions) is the one reader of `PRUV_CTX`, so no message changes.

### P10H.3 Measurements (median ns, ratio = Pruv / hand; hand Rust lane has equal semantics)

Host facts, after the fix (9 rounds; `rq_find`: S=20000, M=10, 420000 fresh per-item `is_temporary` calls, no match):

| build | `rq_find` ratio | ns/call Pruv / hand | `bh_halt` ratio | ns/halt Pruv / hand |
|---|---:|---:|---:|---:|
| py/rust | 1.065 | 2.07 / 1.94 | 0.994 | 29.8 / 29.9 |
| rs/rust | 1.162 | 3.15 / 2.71 | 1.001 | 28.8 / 28.8 |
| py/llvm | 1.568 | 4.44 / 2.83 | 2.341 | 66.5 / 28.4 |
| rs/llvm | 2.006 | 4.58 / 2.28 | 2.293 | 66.2 / 28.9 |

py/rust and rs/rust generate byte-identical `root_gen.rs` (and so do the two llvm builds), so the rs/rust 1.16x is
run noise: three 21-round repeats gave rs 0.71x, 1.25x, 0.96x and py 1.02x, 1.02x, 0.99x on the same machine.
Before the fix: py/rust 1.656x (4.69 / 2.83), rs/rust 2.599x.

Re-run after the audit harness fixes (hand lanes carry the same `requires` checks, both halt lanes `black_box` `st` and
`out` after every call, lanes rotate per round; 9 rounds):

| build | `rq_find` ratio | ns/call Pruv / hand | `bh_halt` ratio | ns/halt Pruv / hand |
|---|---:|---:|---:|---:|
| py/rust | 0.981 | 3.15 / 3.21 | 1.496 | 43.2 / 28.9 |
| rs/rust | 0.897 | 3.09 / 3.44 | 1.603 | 41.4 / 25.8 |
| py/llvm | 1.216 | 3.78 / 3.11 | 2.437 | 68.0 / 27.9 |
| rs/llvm | 1.335 | 2.70 / 2.02 | 2.404 | 64.6 / 26.9 |

The earlier 1.0x `bh_halt` hid real work: with `st` unobserved, the state writes and the guarded calls around them
could be dropped. The Rust-backend `bh_halt` excess (+13 to +17 ns per halt) is the `vec_get` builtin
`pruv_g6_vec_get__i`. It is not `#[inline(always)]` and opens a `pruv_ctx` guard (two thread-local writes) on each of
its 8 calls per halt. In a scratch copy of the generated `root_gen.rs` (no compiler change), dropping only that guard
line and adding `#[inline(always)]` gave Pruv / hand 27.9 / 28.1, 27.7 / 27.7 and 28.0 / 27.6 ns (three 9-round
runs). Also removing the `catch_unwind` around `running_ids` changed nothing.

Fix (both emitters, `pruvc/foreign_codegen.py` `_fn` and `pruv-core/src/foreign_codegen.rs` `fn_text`, byte-identical):
every non-I/O G6 catalog helper (all ops except the `file_`/`proc_`/`ev_`/`net_` families) is `#[inline(always)]`,
and the `pruv_ctx` guard is emitted only when the helper has a `requires` and either an `ensures` or a body call that
reads the ctx (`pruv_ud`, `pruv_g6_charge`, `pruv_g9_tick` and the checked `pruv_c*` runtime helpers; list
`_UD_CALLS` / `UD_CALLS`). Helpers now inlined without the guard: `vec_new`, `vec_push`, `vec_get`, `map_new`,
`map_get`, `map_insert`, `string_new`, `string_push_str`, `string_byte`, `string_to_str`. Their refusal messages name
the op explicitly and do not read the ctx, so no message changes (diff_rust 20 fuzz, all same).

Final run after the fix (one compiler rebuild, 9 rounds, `bench/l3/hostfact/run.py --rounds 9`, all 52 gates pass):

| build | `rq_find` ratio | ns/call Pruv / hand | `bh_halt` ratio | ns/halt Pruv / hand |
|---|---:|---:|---:|---:|
| py/rust | 1.012 | 3.49 / 3.45 | 0.968 | 25.97 / 26.82 |
| rs/rust | 1.013 | 3.16 / 3.11 | 0.942 | 27.87 / 29.59 |
| py/llvm | 1.199 | 4.00 / 3.33 | 2.126 | 60.32 / 28.38 |
| rs/llvm | 1.193 | 3.90 / 3.27 | 2.102 | 60.37 / 28.71 |

**req verdict:** passes on the Rust backend (median 0.90x to 1.07x over all runs, 1.01x in the final run); **blocked on
the LLVM backend** (1.19x to 2.0x, P17).
**budget synchronous path:** passes on the Rust backend (`bh_halt` 0.94x to 0.98x, matching the scratch copy); 2.1x to
2.5x on LLVM (P17).

LLVM decomposition (not separated further; P17): `rq_find` +1.6 to +2.3 ns per host call is one non-inlined
`extern "C"` call from the `.ll` object into `pruv_g6_v1_*` plus its `pruv_g6_guard` frame (the Rust backend inlines
the same shim). `bh_halt` +38 to +44 ns per halt is 11 such crossings per halt with 3 ids (`running_ids`, `len`, 8
`vec_get`, `drop`) plus one heap handle (`pruv_g6_new`, a `Box` around the returned `Vec`) allocated and freed per halt.

M4 adapter lanes (perf_headroom/host `--only m4 --rounds 7`, sessions axis, ns per op; tools and clocks are at or below
1.10x matched):

| n | hand | handc | handcb | m4 | m4llvm | m4c | m4cb | m4c/handc | m4cb/handcb |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 100 | 14.6 | 16.4 | 17.9 | 13.0 | 20.4 | 24.5 | 25.8 | 1.49 | 1.44 |
| 500 | 15.8 | 17.5 | 18.5 | 12.7 | 20.2 | 24.1 | 25.8 | 1.38 | 1.39 |
| 1000 | 15.8 | 17.4 | 18.5 | 13.2 | 20.7 | 24.4 | 26.3 | 1.40 | 1.42 |

Batching did not lower the sessions cell. Exact decomposition of `m4c - handc` = `(m4llvm - hand) + (m4c - m4llvm) -
(handc - hand)`:

| n | m4c - handc | m4llvm - hand (LLVM path) | m4c - m4llvm (Pruv C entry) | handc - hand (hand C entry) |
|---:|---:|---:|---:|---:|
| 100 | +8.1 | +5.8 | +4.1 | +1.8 |
| 500 | +6.6 | +4.4 | +3.9 | +1.7 |
| 1000 | +7.0 | +4.9 | +3.7 | +1.6 |

The largest term is the LLVM path (the same program is 0.80x to 0.89x of hand on the Rust backend, `m4`); the Pruv C
entry costs 2.1 to 2.3 ns more than the hand C entry, because the generated entry calls the LLVM body across an object
boundary while `handc` inlines its body. The crossing itself is not the term: `m4cb` (one crossing, one `catch_unwind`
per get+put) is +1.3 to +1.9 ns slower than `m4c`, and `handcb` is +1.0 to +1.5 ns slower than `handc`, because the
batch adds a per-item loop and per-item checks while the per-call crossing was already cheap. World equivalence:
`m34_test m4cb` and `m4c` 300/300 (seed 1, 922 grows).

### P10H.4 Batched entry and symbol prefix

`@P@` in the C adapter is replaced by `<pkg>__` (the `pruv.json` name), so the C adapter symbols
(`pruv_handle_*`/`pruv_h_*`/`pruv_hb_*`) carry the package name. `<pkg>__pruv_hb_NAME(h, items, n, done)` runs up to
`n` operations inside one `catch_unwind` and one `store.with`, applies the single-entry checks per item, stops at the
first nonzero result and reports `done`. `tests/l3/abi` probes it (positive, poison, bad args, stop-at-first, prefixed
names).

Audit findings, closed:

- **Prefix scope.** For a program with a package name, every global program symbol of the LLVM backend carries
  `<pkg>__`: the `.ll` exports `pruv_abi_<entry>` / `pruv_abi_<entry>__typed`, the G6 imports `pruv_g6_v1_*` and
  `pruv_g6_bridge_v1` with their `#[no_mangle]` definitions in the Rust module, and the C adapter symbols. Root entry
  bodies in the `.ll` were already `define internal`. A program without a package name keeps the old names. Not
  prefixed on purpose: the shared runtime (`pruv_alloc`, `pruv_panic_*`, `pruv_check_*`, linked once) and the
  program-independent `pruv_host_*` hooks the runtime calls by name. Each Rust module still emits those hooks, so a
  host that links two modules keeps one copy (the same holds for `pruv_panic_cost` with cost and `pruv_g7_v1_run`
  with par, SPEC.md: shared definitions are linked once). `tests/l3/abi/check_two_programs.py` (+ `c_two.c`) compiles packages
  `twoa` and `twob` (same entry name, same `vec_*` builtins) with the LLVM backend and links ONE staticlib (both Rust
  modules, the second without its hooks, asserted byte-identical to the first), both `.ll` objects and one runtime
  into one C binary that calls both programs. It also checks that every non-internal `.ll` define and every
  `#[no_mangle]` item is prefixed or a shared hook.
- **`done` on up-front refusal.** An up-front `PRUV_BAD_ARGS` (`n < 0`, null `items`, misaligned or overlapping
  arguments) now writes `*done = 0` when `done` itself is a usable slot (non-null, aligned, no address overflow, not
  overlapping the handle or the `ret` table); an unusable `done` is never written. `c_probe.c` probes `n = -1` with
  `done` preset to 77, null `items` with `n = 1`, and the unusable-`done` cases. SPEC.md states the rule.

### P10H.5 Notes for P11+ and P17

- **P17 (LLVM boundary).** Every LLVM cell above 1.1x here is the `.ll` to Rust call boundary: per G6 host call
  (`rq_find`, `bh_halt`), per Vec element (`vec_get` crossings) and per op (`m4c`). Inline the Rust shims into the
  LLVM object or link with LTO; lower `Vec` results to a borrowed slice instead of a boxed handle with per-element calls.
  req stays blocked on LLVM until then.
- **Open from the P10H confirmation audit (needs a compiler rebuild, both emitters).** (a) `_UD_CALLS` / `UD_CALLS`
  (`pruvc/foreign_codegen.py`, `pruv-core/src/foreign_codegen.rs`) misses runtime helpers that also call `pruv_ud`:
  `pruv_cband`, `pruv_cbor`, `pruv_cbxor`, `pruv_cwrapping_shl`, `pruv_cushr`, `pruv_cwrapping_add`,
  `pruv_cwrapping_mul` (`pruvc/codegen.py`, `codegen.rs`). No current G6 builtin body calls them, so no guard is
  missing today; a future builtin that does would lose its ctx guard. Root fix: derive the list from the runtime
  prelude (every helper whose body reads `pruv_ud`) instead of a hand list. (b) `map_updated` is emitted outside
  `_fn`/`fn_text` and is not `#[inline(always)]`; it is not on a measured path.
- **P11.** The per-call `valid` (P10.4) and `m4raw/m4` (1.12x to 1.13x on sessions, P10.3) are unchanged by P10H.
- **Batch API.** Keep `pruv_hb_` for callers that already hold a batch; it is not a performance lever while the body
  crosses the LLVM boundary.

## Appendix P11 (import admission I0+I1, compiler list fix, writer preservation measured)

### P11.1 What landed

- **Admission (`bench/l3/budget/bud_admit.pruv`, new lib module).** `admit(st, inout cnt)` checks a store handed in
  from outside and returns the live count (roots included) or a negative code: -1 header (`bud_store::valid`, I0),
  -2 roots / top, -3 entry state, -4 owner (live, not a root, below top), -5 key span or padding (offset inside the
  arena, `words(kn) <= key cap`, unused u16 lanes zero), -6 blob span (`len <= cap`, span inside the arena), -7 links
  (`f7..f10` in `-1..top-1`, child count `>= 0`), -8 free list (walk bounded by top, ends at -1, `free + live == top`),
  -9 slot word (entry index in `0..top-1`; a slot naming a free or reused entry or with a stale tag is accepted,
  because removal frees descendants without clearing their slots and `entry_matches_w` re-checks hash, state, owner, owner generation and key),
  -10 counts (word 12 live, word 13 used slots, word 14 in
  `0..at-ab`). Work is linear: one pass over `top` entries, one bounded free walk, one pass over the slots;
  `cnt` returns the three visit counts. It lives in its own module so roots that do not import (`root_m4`, `root_m8`,
  `root_m9`) do not carry its proof.
- **Gate P test (`m34_test/root_admit.pruv`, `driver_admit.rs`, `run.py admit`).** A clean store (10 live, top 11, one
  freed entry, one blob), 17 single-word corruptions (one or more per check above), and 3 valid stores admit must
  accept: a stale slot tag (case 14), entry 3 removed with its children 9, 10 leaving their slots (case 19, live 7),
  and then two keys added (reusing entries 3 and 10, 10 under its stale slot) and entry 3 removed again (case 20,
  live 8). Then the clean store at 1024, 4096
  and 65536 words: entries visited 11 and free steps 1 at every length, slots visited exactly 32 / 128 / 2048. The
  harness checks the exact 23 `(case, len)` rows; 23/23 pass on both compilers.
- **Compiler (both emitters).** `_UD_CALLS` / `UD_CALLS` are derived from the runtime prelude (every helper whose body
  reads `pruv_ud`, plus listed roots for the large PROC/NET/ASYNC preludes); the 7 helpers named in P10H.5 (a) are now
  covered. `map_updated` is `#[inline(always)]` (P10H.5 (b)). `tests/diff_rust.py --fuzz-count 20` all same;
  `g8_frontend` 27/27 (py) and 39/39 (rs).

### P11.2 Writer preservation: measured, not landed

`valid(old(st)) ==> valid(st)` was proved for the bud_map header writers and helpers (`sf`, `sh` for words 13/14 with
`v >= 0`, `write_at` for `o >= 16`, `put_units`, `take_words`, `drop_entry`, `drop_child`, `set_blob`; `set_ef` in
bud_store with header equality). It does not remove any runtime check yet: the per-call `valid` in `m4_op` /
`m4_observe` stays until the big mutators (`put_at`, `put_key_h`, `remove`, `rehash`, `add_absent_w`, and through them
`add_w`, `room`, `make_room`) also preserve it, and those proofs time out at the default limit. The cost lands on every
root that imports bud_map (Python compiler, user CPU s, `root_m8`; HEAD on the same machine 15.0):

| variant | root_m8 | main growth |
|---|---:|---|
| all writers above | 28.7 | `remove` +3.9, `set_blob` +3.0, `drop_entry` +2.8 (callers expand `valid` at each `sf` call) |
| without `sf`, `drop_entry`, `drop_child`, `set_blob` (`put_units` calls `set_ef` directly) | 17.8 | `write_at`, `put_units`, `set_ef`, `sh`, `take_words` ~0.3–0.7 each |
| also without `write_at`, `put_units` | 16.7 | `sh`, `take_words`, `set_ef` |
| `set_ef` ensures only | 15.7 | `set_ef` |

Because P11 forbids proof-time growth and the preservation buys no runtime work yet, bud_map.pruv and bud_store.pruv
are unchanged by P11. The proved variants are kept outside the tree for P12 (see P11.5).

### P11.3 Proof time (user CPU s, one run each, same machine, quiet)

| root | Python HEAD | Python P11 | Rust P11 |
|---|---:|---:|---:|
| m34_test/root_m4 | 13.9 | 14.2 | 23.9 |
| m8_test/root_m8 | 15.0 | 14.7 | 24.6 |
| m9_test/root_m9 | 14.5 | 14.7 | 24.6 |
| m34_test/root_admit (new) | n/a | 14.2 | 24.0 |

The 25–27 s figures taken at the start of P11 were measured under load (real 45 s for 27 s user) and are not used.
The Rust column was measured with the P11 compiler only (the HEAD binary had been rebuilt); bud_map/bud_store are
HEAD, so it is the HEAD proof workload. Rust times above were taken while another proof ran, so they are upper bounds.

### P11.4 Performance (core ns/op, median of 5, 200000 reps, Rust backend)

| axis | n | m4 | m4nv | m4raw | m4raw code, Plain store | m4raw/m4 | m4raw − m4 | of which poison flag |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| sessions | 100 | 13.3 | 12.0 | 15.0 | 14.1 | 1.13 | 1.7 | 0.9 |
| sessions | 500 | 13.0 | 11.9 | 14.7 | 13.0 | 1.13 | 1.7 | 1.7 |
| sessions | 1000 | 13.1 | 12.1 | 15.1 | 13.2 | 1.15 | 2.0 | 1.9 |
| tools | 100 | 30.3 | 29.7 | 31.9 | 30.6 | 1.05 | 1.6 | 1.3 |
| tools | 500 | 33.9 | 33.3 | 35.7 | 34.0 | 1.05 | 1.8 | 1.7 |
| tools | 1000 | 32.7 | 33.0 | 34.4 | 33.3 | 1.05 | 1.7 | 1.1 |
| clocks | 100 | 25.4 | 23.9 | 27.1 | 25.6 | 1.07 | 1.7 | 1.5 |
| clocks | 500 | 24.9 | 23.0 | 26.7 | 24.4 | 1.07 | 1.8 | 2.3 |
| clocks | 1000 | 25.9 | 24.2 | 28.0 | 26.1 | 1.08 | 2.1 | 1.9 |

"m4raw code, Plain store" is the m4raw build (manifest package, `raw_entries "crate"`) with the driver's `Store`
type switched from `sol::pruv_raw::PruvStore` to the local no-poison `Plain`; nothing else differs. The difference
to m4raw is the `PruvStore::with` poison flag (a store before and after each call plus the `Err` branch): 0.9 to 2.3 ns
of the 1.6 to 2.1 ns gap, the rest (0 to 0.9 ns) is run noise at this size. `m4 − m4nv` (0.6 to 1.9 ns) is the
per-call `valid`. Cells above 1.1x: `m4raw/m4` sessions n=100/500/1000 (1.13, 1.13, 1.15), attributed to the poison
flag. `m4` itself is unchanged by P11 (P10H: same cells).

### P11.5 Notes for P12 and P13

- **Big-mutator preservation (P12).** Prove `valid(old(st)) ==> valid(st)` for `put_at`, `put_key_h`, `remove`,
  `rehash`, `add_absent_w` (then `add_w`, `room`, `make_room`) and for the bud_clock / lifecycle / world / tools / halt
  mutators, then drop the per-call `valid` in `m4_op` / `m4_observe` (keep the init-returned-0 word, P10.4). Keep
  proof time flat: state preservation through a helper whose body is not unfolded at each `sf` call (the measured
  cost above is the callers re-expanding `valid` per call), or prove it once per mutator over the words it writes
  (7, 8, 10, 12, 13, 14) instead of through every field writer. Reproduction files from P11:
  `work/p11_preservation/` (not committed; `bud_map_full.pruv`, `bud_map_writers.pruv`, `bud_store_set_ef.pruv`).
- **Poison flag (P12/P13).** The `m4raw/m4` cells above 1.1x are the `PruvStore` poison flag. A proved body cannot
  panic except through the runtime's abort paths; if the adapter can rely on that (panic = abort for raw entries), the
  flag and its two stores go away and `m4raw` meets `m4`.
- **Admission contract (audit).** `admit` returns codes but has no `ensures` giving I0 / I1 on success, so callers
  cannot yet drop checks on its result. P12: state `result >= 0 ==> valid(st) && (forall live e: entry_ok(st, e))`
  (or the subset a raw entry needs) and prove it from the loop invariants. Also: the linearity test grows the buffer only; a case
  that grows top and the free list with it should be added with the contract.
- **I1 vs I2 (audit).** Spans may overlap inside the arena (in bounds, so I1 holds), but a blob write can then change
  another key's padding; preserving I1 under `set_blob` needs a frame / no-alias fact. Excess child count, owner
  generation mismatch, link order and cycles, slots naming roots, duplicate or missing slots are I2 (clamps, dynamic
  compares and step budgets keep them safe) and stay out of admission. `cap = 0` with any offset and the stale
  lengths of free entries are accepted, so no unchecked access may rely on them.
- **Admission at the boundary.** `admit` is not yet called by an entry; wiring it into the import path of the C / raw
  adapter (once per handed-in buffer, not per call) is what lets the per-call `valid` go once P12 lands.

## Appendix P12 (admission ensures and linearity case; big-mutator preservation measured, deferred to P13)

### P12.1 What landed

- `bud_admit::admit` has `ensures result >= 0 ==> (bud_store::valid(st) && result <= 4294967296)`, which gives I0 on
  every success code. `st` is read-only in `admit`, so the success paths carry the `valid(st)` check at the top through
  every loop and no extra loop invariant is needed. The per-entry I1 conjunct (`forall` live e: `entry_ok`) is not
  stated: `entry_ok` reads fields through `ef` and the bound checks, so stating it means quantifying over a function
  with a body in an ensures. That waits for the opaque-helper work in P12.2.
- Two admission linearity cases are added in `m34_test/root_admit.pruv` (`grow`, `driver_admit.rs`, `run.py admit`).
  Cases 21 and 22 run on a 65536-word store. Each adds 2n entries under root 0 (n = 64, 256) and then removes n, so
  top and the free list grow together, not only the buffer. `run.py` checks:
  - the result is live (74, 266);
  - entries visited == top == live + free (135 = 74 + 61, 519 = 266 + 253);
  - free steps ≥ n − 4;
  - slots visited == the slot count (2048).

  The cases build a flat layout because `remove` frees only one level: a chain of nested entries would leave orphans
  whose owner is free (class -4). `admit: 25/25` passes on both compilers.

bud_map.pruv, bud_store.pruv and the runtime adapters are unchanged by P12. P12.2 explains why.

### P12.2 Big-mutator preservation: measured, not landed

The plan was to prove `valid(old(st)) ==> valid(st)` for the mid writers first (`put_units`, `drop_entry`,
`drop_child`, `set_blob`), then for the big mutators and the bud_clock / lifecycle / world / tools / halt mutators. To
keep the per-write cost low, the field writers export ground frames:
- `hdr_same(a, b, w)`: header words 0..14 are equal except w;
- `set_ef`, `sf`, `sh`, the new `slot_wr`, `write_at` and `take_words` say which header words they keep;
- `ea` and the slot helpers return ≥ 16, and `fits_region` takes the clamped offset.

All of this proves, but the time is not flat (Python, root_m8, user CPU s):

| variant | root_m8 | top functions (s) |
|---|---:|---|
| HEAD | 13.25 | `set_blob` ≈0, `drop_entry` 0.19, `remove` 0.89 |
| + `sf` / `set_ef` ground frames | 13.26 | |
| + frames on the small writers (`write_at`, `take_words`, `sh`, `slot_wr`) | 13.52 | |
| + `valid` preservation on the 4 mid writers | 22.8 | `set_blob` 4.3, `drop_entry` 3.4, `remove` 1.7 |
| same, `band(s, s-1)` dropped from `valid` (unsound, experiment only) | 14.6 | |
| same, power of two as a 30-way linear disjunction | 19.9 | `set_blob` 3.7, `drop_entry` 2.7 |

Two facts come out of these runs:
- **The cost is not the frames.** Ground header frames are free (13.26). The cost is that every `valid(...)` in an
  assumed or proved ensures is expanded syntactically (check.py `x_expand`), so each callee's
  `valid(old) ==> valid(st)` puts a full 15-check copy into the caller's query.
- **The bit-vector `band(s, s-1)` is most of that copy.** A linear disjunction removes the bit-vector term but still
  costs about 6.6 s.

The remaining path is a split:
- `valid = shape(st) && dyn_ok(st)`.
- `shape` covers words 0..6, 9 and 11 plus `len`, and is the only part that holds the power-of-two test.
- `dyn_ok` is the linear part over the words the mutators write (7, 8, 10, 12, 13, 14).
- Mutators export a ground "shape words unchanged" plus `dyn_ok(old) ==> dyn_ok(st)`. `valid` is then re-derived only at
  `m4_op` by congruence.

The catch is that the store mutators the mid writers call (`entry_alloc`, `entry_free`, `arena_put`, `fill_entry`)
already state `valid(old) ==> valid(st)` in HEAD. All of them would have to move to the split form first. That is the
bud_store-wide refactor P13 starts with. An alternative is a verifier feature in both compilers: an opaque predicate
whose body is revealed only where it is asked for (AGENTS principle 5). The proved P12 variants are kept outside the
tree: `work/p12_wip_frames.diff` and `work/p12_bud_store_lineardisj.pruv`.

Because the preservation did not land, the per-call `valid` in `m4_op` / `m4_observe` stays. So does the
`PruvStore` poison flag: with `valid` still a runtime check and the mutators not yet proved to keep it, a C or raw
caller can still hit the `Err` path. Relying on panic = abort is deferred together with the preservation.

### P12.3 Proof time (user CPU s, one run each, same machine, quiet)

| root | Python HEAD | Python P12 | Rust HEAD | Rust P12 |
|---|---:|---:|---:|---:|
| m34_test/root_m4 | 12.57 | 12.57 | 10.58 | 10.58 |
| m8_test/root_m8 | 13.25 | 13.30 | 11.56 | 11.56 |
| m9_test/root_m9 | 13.35 | 13.35 | 11.08 | 11.08 |
| m34_test/root_admit | 12.91 | 13.36 | 10.82 | 10.82 |

- root_m4, root_m8 and root_m9 have the same sources at HEAD and P12, so they were measured once (root_m8 Python HEAD
  was also measured separately as 13.25).
- root_admit Python grows by +0.45 s. The new `grow` test function accounts for that: `admit` alone was 12.41 against
  12.36 at HEAD.
- The Rust compiler is the HEAD source (`b67b4ff`), built in the main checkout, and P12 does not change it. The P11
  Rust column (≈24 s) was taken under load.

### P12.4 Performance

No runtime code changed. The new ensures generates no code, and the root_admit cases are test-only, so the P11.4 table
stands as is. The cells above 1.1x are the same: `m4raw/m4` sessions 1.13 to 1.15, from the poison flag (P12.2).

Gates run:
- `run.py admit` 25/25 with both compilers;
- `run.py lanes --count 300`: all lanes 300/300, size and header checks ok.

Gates not run, because their trigger did not apply:
- `cargo test --release` and `diff_rust.py` (no compiler change);
- `g8_frontend` and `--compare-rows` (no codegen or row change);
- the lib user tests (bench/l3/lib unchanged).

The external audit was not run because no new verification encoding landed. The only new contract is the
one-line `admit` ensures above.

### P12.5 Notes for P13

- **valid split (first).** Introduce `shape` / `dyn_ok` in bud_store and move `entry_alloc`, `entry_free`,
  `arena_put` and `fill_entry` to "shape words unchanged + `dyn_ok` preserved". Then reapply `work/p12_wip_frames.diff`
  (the frames and the `fits_region(o)` signature) with the split form on the mid writers. Measure root_m8 against
  13.25 s after each step. Then do the big mutators (`put_at` via `slot_wr`, `put_key_h`, `remove`, `rehash`,
  `add_absent_w`, `add_w`, `room`, `make_room`) and the clock / lifecycle / world / tools / halt mutators. Finally
  drop the per-call `valid` in `m4_op` / `m4_observe`.
- **Poison flag.** Once the per-call `valid` is gone, rely on panic = abort in the raw / C adapter and remove the
  `PruvStore` flag. Target: `m4raw/m4` ≤ 1.1x.
- **Admission I1 ensures.** State the `entry_ok` subset a raw entry needs, as a ground predicate on the entry words.
  Then wire `admit` into the adapter import path (P11.5).
- **`remove` depth (observed, not analysed).** Removing the middle of a 2n-deep chain left a store that admission
  rejects with -4: an owner is not live, so deeper descendants keep a freed owner. Check whether this is intended lazy
  deletion. If it is not, `remove` needs a work-list free before admission can be relied on after removals.

## Appendix P13a (remove frees the subtree; valid split into shape + dyn_ok; big-mutator preservation measured, deferred)

P13a is the work P12 deferred (P12.2, P12.5). The original P13 row (reserve / grow / failure) is P13b. Steps 0 and 1
landed. Step 2 (big mutators) proves but is not flat, so it is kept outside the tree and steps 3 to 5, which depend on
it, were not started (P13a.3).

### P13a.1 `remove` depth: a bug, fixed (step 0)

Removing the middle of a deep chain left live entries whose owner is free. P12 observed this as admission class -4.
It is a `remove` bug, not intended lazy deletion:
- **The lazy part is only the index slots.** `drop_child` leaves a removed descendant's slot in place, and lookups
  re-check hash, state, owner and key (`entry_matches_w`). That is deletion by tombstone-free staleness and costs a
  probe at most.
- **The orphans were live entries.** A grandchild kept state 1 and stayed on the live count (word 12) and on the used
  slot count. `need()` charges word 12 against the capacity, so every orphan leaked one entry of capacity for the life
  of the store, and nothing could reach it again (its owner's links were gone).
- **Depth is reachable through the pub API.** `add_w` takes any live entry as owner. The world sits at most two levels
  below the roots, but `bud_map` is a general map module and the old comment ("one level") stated an assumption the
  API does not enforce.
- **A store the program produced must pass admission.** With orphans it did not (-4).

Fix (bud_map.pruv `remove`, new `drop_desc`): a post-order walk without a stack. The walk:
- descends to the head child while that child is a live entry owned by the current node;
- frees a leaf and moves on to its next sibling;
- when a list ends, climbs to the owner and frees it.

Each entry of the subtree is entered once and freed once. On a valid store the walk takes at most 2 * (subtree size)
+ 1 steps. The budget is 2 * walk_limit + 2, which bounds the walk on any buffer. The words written are the same as one
`drop_entry` per entry, in the same order:
- `drop_desc` folds the deferred word-14 update (`take_words`) as before;
- an entry without a fast base flushes the pending sum and calls `drop_entry`.

The cost is linear in the subtree, and a one-level removal does the same work as before.

Test: case 23 in `m34_test/root_admit.pruv` (`chain`) runs on the case-19 store (live 7) at len 65536. It builds a
128-deep chain under root 0, adds a leaf under entry 64 after its chain child (so the walk climbs from an internal
node to a next sibling), and removes entry 64 with its subtree. `run.py admit` then checks:
- the result is live = 70;
- entries visited == top == live + free (136 = 70 + 66).

`admit: 26/26` passes on both compilers. With the old `remove` the case returns -4, and so does a mutant that drops
the sibling step (`c = un` replaced by `c = -1`).

The guarantee needs well-formed child lists (I2), which every store the module builds has. Admission (I0 + I1) does
not check I2: a store that passes admission but has, say, a cut head link makes `remove` stop early, and the next
admission returns -4. On such buffers `remove` stays in bounds and terminates (the step budget), which is all it
promises there (P13a.7).

### P13a.2 `valid` split (step 1)

bud_store.pruv:
- `valid(st) = shape(st) && dyn_ok(st)`.
  - `shape` covers `len >= 232`, the power-of-two `s` with the layout inequalities (the only `band` term), and words
    0..6, 9 and 11.
  - `dyn_ok` is linear over the words the mutators write: `0 <= st[7] <= st[6]`, `-1 <= st[8] < st[7]`,
    `st[9] <= st[10] <= st[11]`, and `st[12], st[13], st[14] >= 0`. `len >= 15` makes it total.
  - The conjunction is the old `valid` term for term. The old `top <= cap` reads `st[6]` directly, which `shape` pins to
    `cap`.
- Ground frame predicates, all total on any buffer:
  - `hdr_same(a, b, w)`: header words 0..14 are equal except w;
  - `shape_same(a, b)`: same length and the same shape words;
  - `kept(st, o)`: same length, and when `len >= 15` also `shape_same` and `dyn_ok(o) ==> dyn_ok(st)`.

  `shape` reads only the length and the shape words, so `kept(st, o) && valid(o)` gives `valid(st)` by congruence.
- `entry_alloc`, `entry_free` and `arena_put` now ensure:
  - `shape_same` plus the dynamic words they do not write;
  - `dyn_ok(old) ==> dyn_ok(st)`, in place of `valid(old) ==> valid(st)`.

  `fill_entry` and `set_ef` ensure `hdr_same(st, old, -1)`, a header frame.
- **Header-safe guards.** The frames hold on any buffer, not only under `valid(old)`, because the writes that could hit
  the header are guarded at run time:
  - `set_ef` and the `fill_entry` slow path return when `a < 16`;
  - `arena_put` returns -1 when `top < 16 || aend > len || top > aend` (it used to clamp with `c40`);
  - `wr16` writes only when `16 <= p < len`.

  On a valid store none of them fires:
  - entry bases are `>= 16 + s`;
  - the arena range is `16 <= ab <= at <= ae <= len`.

  Each guard is one or two compares, replacing the compares that were there. `wr16` is added for P13a.3 and has no
  caller yet.

Proof time fell by about 40% (P13a.4). The `band` term was being expanded into every caller of the four store mutators.

### P13a.3 Big-mutator preservation: measured, not landed (step 2)

The full step-2 change was written and proves on both compilers. It is kept at `work/p13a/bud_map.step2.pruv` and
`work/p13a/p13a_step2_bud_map.diff`. It contains:
- P12's frames reapplied in the split form: `sf` / `sh` header frames; `wr16` for the slot writes in `put_at`,
  `drop_entry` and `rehash`; `write_at`'s fast path gated on `o >= 16`; `set_blob` in place only at `o >= 16`;
- `kept(st, old(st))` ensures on `write_at`, `put_units`, `add_w`, `put_key_h`, `put_at`, `drop_entry`, `remove`,
  `drop_desc`, `take_words`, `drop_child`, `set_blob`, `rehash`, `make_room`, `room` and `add_absent_w`;
- loop invariants (`kept` or `hdr_same`) wherever a loop body calls an inout function.

It is not flat (user CPU s, one run each):

| state | root_m8 Python | root_m8 Rust | root_m4 Rust |
|---|---:|---:|---:|
| HEAD (reference) | 13.25 (P12), 13.82 (this run) | 11.62 | 11.02 |
| P13a steps 0 + 1 | 8.41 | 7.70 | 6.78 |
| + step 2 in bud_map (bud_world not yet) | 14.65 | 10.33 | 9.56 |

Per-query solver time on root_m8 (Python, timer around `sol.check()`, local experiment only), step 1 → step 2:

| obligation | queries | solver s |
|---|---:|---:|
| `contract_domain` | 73 → 316 | 0.36 → 2.01 |
| `ensures` | 326 → 444 | 0.62 → 1.38 |
| `invariant_preserved` | 256 → 366 | 1.36 → 1.59 |
| all | 2625 → 3109 | 6.3 → 9.0 |

By function, `remove` went from 0.57 to 1.86 s and `put_at` from 0.36 to 1.19 s.

Two findings:
- **The solver is less than half of the increase.** Solver time grows by 2.7 s; the other 3.2 s of the 5.9 s is
  building and asserting terms. Each `kept` expands to about 30 array reads (two `dyn_ok` copies plus `shape_same`),
  and that copy goes into the path condition after every call and into every loop invariant.
- **The domain checks are not the cause.** All 243 new `contract_domain` queries are valid without the path condition.
  An experiment that proved them path-free first removed them and gave 14.51 s: only the solver share moved.

bud_world's 11 mutators would add the same kind of copies on top. Splitting further at the source level does not help,
because the predicate is small. The cost is the syntactic expansion at every use. The remaining path is the verifier
feature P12.2 already named, in both compilers (AGENTS principle 5): an opaque ground predicate, kept as an
uninterpreted application at call sites and loop heads, with its body revealed only where a proof asks for it. The
frame then needs only reflexivity and transitivity of `kept` on top of the store mutators' revealed proofs.

Steps 3 (drop the per-call `valid` in `m4_op` / `m4_observe`), 4 (poison flag) and 5 (I1 ensures, adapter wiring)
need step 2, so they were not started. The `m4raw/m4` sessions cells stay at P11.4 / P12.4 levels, with the same
attribution (P13a.5).

### P13a.4 Proof time (user CPU s, one run each, same machine, quiet)

| root | Python HEAD | Python P13a | Rust HEAD | Rust P13a |
|---|---:|---:|---:|---:|
| m34_test/root_m4 | 13.09 | 7.54 | 11.02 | 6.78 |
| m8_test/root_m8 | 13.82 | 8.41 | 11.62 | 7.70 |
| m9_test/root_m9 | 13.49 | 8.08 | 11.45 | 7.17 |
| m34_test/root_admit | 13.70 | 7.76 | 11.03 | 6.97 |

- The HEAD column was re-measured in this run. It is 3 to 6% above P12.3 (root_m8 Python 13.82 against 13.25),
  which is machine drift between sessions.
- Step 0 alone gave root_m8 Python 13.21 and root_m4 Python 12.46, i.e. flat. The fall comes from step 1.
- The Rust compiler is the HEAD source built in this worktree. P13a does not change it.

### P13a.5 Performance (core ns/op, median of 5, 200000 reps, Rust backend, HEAD and P13a alternated per run)

| axis | n | HEAD m4 | HEAD m4nv | HEAD m4raw | m4 | m4nv | m4raw | m4raw/m4 | m4 / HEAD m4 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| sessions | 100 | 13.5 | 12.0 | 14.7 | 13.5 | 12.1 | 15.0 | 1.11 | 1.00 |
| sessions | 500 | 12.7 | 11.7 | 14.5 | 13.0 | 11.6 | 14.5 | 1.12 | 1.02 |
| sessions | 1000 | 12.9 | 12.1 | 14.4 | 13.2 | 12.0 | 14.9 | 1.13 | 1.02 |
| tools | 100 | 29.6 | 29.4 | 31.8 | 30.1 | 29.1 | 31.5 | 1.05 | 1.02 |
| tools | 500 | 33.4 | 32.6 | 34.7 | 33.1 | 32.2 | 35.4 | 1.07 | 0.99 |
| tools | 1000 | 32.3 | 31.7 | 33.8 | 32.4 | 31.6 | 34.6 | 1.07 | 1.00 |
| clocks | 100 | 24.9 | 23.5 | 26.7 | 24.5 | 23.4 | 26.6 | 1.09 | 0.98 |
| clocks | 500 | 25.0 | 23.8 | 26.8 | 24.9 | 23.6 | 26.7 | 1.07 | 1.00 |
| clocks | 1000 | 25.0 | 23.8 | 27.1 | 24.9 | 23.7 | 26.7 | 1.07 | 1.00 |

- `m4 / HEAD m4` is 0.98 to 1.02, which is run noise. The guards replace compares that were already there, and
  `remove` does the same work on the one-level removals the world makes.
- Cells above 1.1x: `m4raw/m4` sessions n=100/500/1000 (1.11, 1.12, 1.13), the same cells as in P11.4 (HEAD here:
  1.09, 1.14, 1.12). P11.4 attributes them to the `PruvStore` poison flag, 0.9 to 1.9 ns of the 1.7 to 2.0 ns gap measured there. Here the gap is 1.5 to 1.7 ns.
  Removing the flag is step 4, which waits for step 2 (P13a.3).
- `m4 − m4nv`, 0.4 to 1.5 ns, is the per-call `valid` that step 3 removes.

### P13a.6 Gates

Gates run, all on both compilers (Rust: worktree release build; Python: `pruvc/pruvc.py`):
- `run.py admit`: 26/26;
- `run.py m3 --count 300`: 300/300;
- `run.py lanes --count 300`: all 9 lanes 300/300, size and header checks ok.

Gates not run, because their trigger did not apply:
- `cargo test --release`, `diff_rust.py` and `g8_frontend` (no compiler change);
- `--compare-rows` (no row change);
- the lib user tests (bench/l3/lib unchanged).

### P13a.7 Audit

One Codex gpt-6.2-astra review audit, read-only on the source (no z3 run). Verdict: fix needed. Two findings were
REQUIRED. Both are contract statements, not code defects, and were fixed in the text:
- **`remove` needs I2.** Admission checks owner liveness and link bounds, not that child lists are complete, mutually
  linked and acyclic. On an admitted store with a corrupt head link, `remove` stops early and leaves entries, and the
  next admission returns -4. Fixed: the `remove` comment, `m34_test/CONTRACT.md` and P13a.1 now state the guarantee
  for well-formed child lists, and only bounds and termination on other buffers. A check of I2 before the first write
  was not added: it is a walk of the subtree on every removal, for stores the program never produces.
- **Behaviour on corrupt headers changed.** These are intended (P13a.2), and on a valid store none of them fires:
  - `arena_put` now returns -1 when the arena overlaps the header; HEAD wrote at offset 0;
  - `set_ef` and the `fill_entry` slow path skip addresses below 16;
  - a removal candidate whose owner is not the current node ends the list; HEAD skipped it and followed its `next`.
    In a well-formed list every member is owned by the list's node, so the two agree there.

The OPTIONAL findings:
- The deletion order, the budget (at most 2(N-1)+1 steps, N <= walk_limit) and the accounting are correct (words 12,
  13 and 14, the free list and generations). The `valid` split is equivalent to the old `valid` term for term. Neither
  needs a change. The "one level" text in `CONTRACT.md` was stale and is updated.
- `fill_entry` used to ensure that word 15 (pending Need) is kept, and `hdr_same` covers words 0..14 only. The code
  still keeps word 15, and every root proves without that ensures, so no caller uses it. It is left out; it can be added
  back as one ensures if a caller needs it.
- Case 23 did not exercise the climb to a next sibling, since every internal node of a pure chain is an only child.
  Fixed: case 23 adds that sibling, and the mutant above shows the case now catches a wrong sibling step. The audit
  also suggested an exact word-14 check. It was not added: admission checks word 14 only for range, so the check
  would need its own driver output, and `drop_desc` keeps the word-14 update of one `drop_entry` per entry (P13a.1).
- The guards add no new rejection of valid input and no per-iteration work; the measured cells are in P13a.5.

There was no confirmation audit: the REQUIRED fixes are text only. After the case-23 change, `run.py admit` is 26/26 on
both compilers, and root_admit proves in 7.83 s (Python) and 6.88 s (Rust).

### P13a.8 Notes for the next package

- Step 2 is ready to land as soon as the verifier keeps a ground predicate opaque (P13a.3). The archived diff (local, outside the tree) is the
  starting point; bud_world's 11 mutators follow the same pattern. Landed with step 3 in P13a.9.
- Steps 3 to 5 follow, in the original order: drop the per-call `valid` (m4 should meet m4nv), remove the poison flag
  (m4raw/m4 <= 1.1x), then I1 ensures and adapter wiring.
- The reference for proof time is now P13a.4, not 13.25 s.

## Appendix P13o (opaque ground predicates: `opaque fn` and `reveal F;`)

P13a.3 measured why preserving `valid` through bud_map's big mutators is slow (root_m8 Python 8.41 s to 14.65 s):
- **Term building.** Every `kept(st, old(st))` the checker expands inline is about 30 array reads. It is rebuilt and
  re-asserted at each contract instantiation, about 3.2 s of the 5.9 s growth.
- **Domain queries.** Each expansion also brings a contract-domain query for its guarded reads (73 to 316 queries),
  and every one of those is valid with no path condition.

Neither cost buys anything. Most queries in a mutator never look inside `kept`; only the few that chain it
(`kept(c, b) && kept(b, a) ==> kept(c, a)`) need the body. This is a language gap (AGENTS.md principle 5): the user
has no way to say "this predicate is a name here, and its body there". P13o adds it.

### P13o.1 Syntax and meaning

- **`opaque fn` / `pub opaque fn`.** It marks a function that expands in contracts (SPEC §6, the P06 rules) and
  returns `bool`. Its meaning is unchanged: the body. Only the verifier's view changes.
- **Opaque by default.** Wherever the checker would put the body in place (a contract call, or the P07 fact at a
  code call), it keeps the call. The verifier encodes the call as one application of an uninterpreted z3 function
  `opq!F`, applied to the argument encodings. An `i64` or `bool` argument is its own term. An array argument is two
  terms, its contents and its length. Calls with equal arguments are the same term, so congruence still works
  (`kept(st, o)` from an `ensures` matches `kept(st, o)` in a later `requires` without the body).
- **`reveal F;`.** A proof-only statement in a function body, like `assert`. It generates no code. F must be the
  name (a path is allowed) of an opaque function. From that statement on, along the execution path that reaches it,
  F is transparent:
  - every F application already met on the path gets the equation `opq!F(t) == body(t)`;
  - every later F application gets the same equation when it is met.

  A path that does not pass through the `reveal` keeps F opaque. A loop body's reveal ends with that iteration (the
  path after the loop continues from the loop's entry state). A callee's reveal never reaches its callers.
- **Quantified arguments.** An application whose arguments mention a variable bound by `forall`, `exists` or an
  aggregate is not a ground term. It is replaced by its body in place, as before P13o.
- **Totality.** The body of an opaque function must be defined on every argument. The verifier proves its contract
  domain once, at the definition (code `opaque_total`). Only the shape facts that every value of a parameter type
  meets inside a contract are assumed: 0 <= len <= MAX_LEN for an array (and the byte domain for `[u8]`), the same
  bound on a map's size. The i64 range of a scalar is not assumed. Contract arithmetic is unbounded, and an `[i64]`
  element has no i64 fact, so a contract can apply F to `x + 1` or `a[j]` outside i64 (audit, P13o.9). After that,
  an opaque application needs no domain obligation of its own (only its arguments' domains). This removes the
  per-instantiation domain queries of P13a.3. Every predicate in bud_store is already written this way (guarded
  reads).

### P13o.2 Soundness

Fix an opaque F with body B over parameters x. Interpret `opq!F` as the function x ↦ B(x), with B read in z3's
total semantics. This is well defined because B is a closed, non-recursive expression over x. With this choice:
- **Equations.** Every equation the verifier adds, `opq!F(t) == B(t)`, is true.
- **Contract meaning.** Each contract that mentions F means what the pre-P13o expansion meant. Totality makes B's
  z3 value its language value on every argument a contract can supply, including integers outside i64 and arguments
  where a partial read could otherwise be unspecified. So dropping the per-application domain obligation does not admit an undefined contract.
- **Code calls.** At a code call, the verifier assumes `result == opq!F(args)`. This is P07's fact under the same
  interpretation. It is valid because F's code is verified separately with no assumptions (no `requires`), against
  the same body.

Each query asks whether `pc ∧ ¬goal` is unsatisfiable, with `opq!F` free. A proof therefore covers every
interpretation of `opq!F`, including the intended one. So a proof with F opaque is a proof with F expanded, and the
reveal equations are true facts under that interpretation.

What can be lost is completeness, never soundness:
- a goal that needs B and has no reveal fails;
- two arrays that are equal only on the range B reads are different `opq!F` arguments.

### P13o.3 Errors and counterexamples

- **Checker errors** (`type` stage):
  - `opaque_signature`: `opaque` on a function that does not expand in contracts or does not return `bool`. The
    message gives the P06 reason.
  - `reveal_target`: `reveal X;` where X is unknown or not opaque.
- **`opaque_total`** (verify): the body is not defined on some argument. The counterexample gives the parameters.
- **Proof failures with something hidden.** When a proof fails and the path holds an application of an opaque
  function that is not revealed there, the message ends with
  `(opaque F is not revealed here; add reveal F; before this point if the proof needs its body)`, one clause per
  function in name order. The counterexample gets `"_opaque": ["F", ...]`, the same names. The SMT model's
  interpretation of `opq!F` is not printed. It is an arbitrary table, and the hint says what to do about it.
- **Same output on both compilers.** pruvc and pruvc-rs build the same terms in the same order:
  - the application term `opq!F` is built at the call;
  - a reveal equation is built at the `reveal` (for earlier applications, in the order they were met) or at the
    application (for later ones), and is appended to the path condition unguarded.

### P13o.4 What it does not allow

- **Opaque values.** No opaque `i64` functions: the body would still be needed for every arithmetic use. No opaque
  generic, recursive, `inout` or `requires` functions: they do not expand at all (P06).
- **Scoped reveals.** No `reveal` inside a contract, no `reveal` that lasts past the path it is on, no reveal for one
  goal only (Dafny's `reveal` in an `assert … by`) and no `hide`. There is one way: a `reveal` statement, scoped to
  the rest of its path.
- **Unrolling.** No fuel and no partial unfolding. A reveal of F does not reveal opaque functions that F's body
  calls; each needs its own `reveal`.
- **Visibility.** P06's `public_interface` rule still applies. A `pub` function's contract may mention only `pub`
  functions, opaque or not, because the body is still the contract's meaning.

### P13o.5 Prior art

- **Dafny.** An `{:opaque}` function (`opaque function` in Dafny 4) is hidden from the verifier until a `reveal f();`
  statement, a ghost statement like `assert`. The Boogie encoding makes the reveal a call to a generated lemma
  ([Dafny Reference Manual](https://dafny.org/dafny/DafnyRef/DafnyRef),
  [Dafny compilation to Boogie](https://dafny.org/dafny/Compilation/Boogie)).
- **Verus.** `#[verifier::opaque]` keeps a spec function's body from unfolding, and `reveal(f)` makes the body
  available for the rest of the enclosing scope. Verus presents this as solver performance control, not modularity
  ([Modules, hiding, opaque, reveal](https://verus-lang.github.io/verus/guide/opaque.html),
  [reveal, reveal_with_fuel, hide](https://verus-lang.github.io/verus/guide/reference-reveal-hide.html)).
- **F\*.** `[@"opaque_to_smt"]` hides a definition from Z3, and `reveal_opaque` brings it back for the rest of the
  scope
  ([F\* wiki](https://github-wiki-see.page/m/FStarLang/FStar/wiki/Code-pattern-for-hiding-definitions-from-Z3-and-selectively-revealing-them)).

Pruv takes the shared core of all three: an opaque marker, a statement that reveals, and an uninterpreted symbol
with definitional equations. It leaves out fuel, `hide`, and per-assertion reveal. The scope is the execution path
(the verifier has no joins), which in structured code is the rest of the enclosing block plus what follows it on the
same branch.

### P13o.6 Proof time (user CPU s, bench sources unchanged)

The feature costs nothing when unused. HEAD (99bdf62) and P13o compilers, alternated on the same machine in one
session (a busy one: absolute numbers sit 5-15% above P13a.4):

| Root | Py HEAD | Py P13o | Rs HEAD | Rs P13o | P13a.4 Py / Rs |
|---|---|---|---|---|---|
| root_m4 | 8.01 | 8.29 | 7.11, 7.08 | 7.10, 7.08 | 7.54 / 6.78 |
| root_m8 | 9.22, 9.14 | 9.11, 9.21 | 7.77, 7.74 | 7.78, 8.13 | 8.41 / 7.70 |
| root_m9 | 8.51 | 8.39 | 7.43, 7.43 | 7.42, 7.46 | 8.08 / 7.17 |
| root_admit | 8.30 | 8.26 | 7.21, 7.22 | 7.30, 7.55 | 7.83 / 6.88 |

### P13o.7 P13a step 2 with opaque `kept`/`dyn_ok`: tried, not landed

Starting from the archived step-2 diff, `kept` and `dyn_ok` were made `pub opaque fn`, with `reveal dyn_ok;` in
`arena_put`, `entry_alloc` and `entry_free`. Two placements were tried on root_m8:
- **No reveal in bud_map.** Reflexivity (`kept(st, st)` at a loop entry or an early return) and the final chaining
  step need `kept`'s body, so the `kept`-ensuring mutators fail with the hidden-opaque hint. The hint names the right function.
- **`reveal kept;` at the top of the 15 `kept`-ensuring mutators**, `dyn_ok` still opaque. `kept`'s body is
  `shape_same(st, o) && (!dyn_ok(o) || dyn_ok(st))`, so chaining no longer needs `dyn_ok`. But mutators that write
  header words through `sh` (`write_at`, `put_units`, `take_words`, `remove`'s loop entry, and others) need
  `dyn_ok`'s body to show the write keeps it, and `put_at` and `drop_entry` time out. Rust: 23.6 s, failing.

What is left is proof engineering in bud_map, not a verifier gap: a per-writer lemma (`sh` keeps `dyn_ok` when the
written word is not one `dyn_ok` reads, or keeps the bound it checks) with `reveal dyn_ok;` only in `sh`, and
`reveal kept;` moved from the function top to the exits. The attempt is kept locally (outside the tree); the
bench sources are unchanged in this package. bud_world's 11 mutators wait for bud_map. Landed in P13a.9 (exit
reveals after the frame asserts, `shape` through the opaque `shape_w`).

### P13o.8 Gates

- Targeted: tests/l3/proof/opaque_reveal.pruv (positive: a symbol passed along without a reveal, reveal before a
  read, reveal on branches, a code-call fact, a quantified argument, arguments outside i64) and
  tests/must_reject/l3_opq_{no_reveal, false, sym_false, total, range, loop_scope, signature, reveal_target,
  unknown_reveal}. The error JSON is equal on both compilers, including with `--infer` and the LLVM
  backend; the generated Rust is equal.
- Framework rows: tests/run_all_rows.txt regenerated with Rust (`--backend both --list-rows`, 4443 rows, 11 new,
  all backend-null). Rust full run (rust backend) 3088/3088; Python full run with `--compare-rows` against the Rust
  list: 3076 run, missing 0, extra 0, failed 0.
- `cargo test --release` passes. `tests/diff_rust.py --fuzz-count 20`: all same.
- `--only g8_frontend` on both compilers: 27/27 (py), 39/39 (rs).
- m34 `run.py admit` 26/26. `lanes --count 300`: every lane 300/300, header checks 1922/1922.

### P13o.9 Audit

An independent soundness review found one required fix. Contract arithmetic is unbounded, but `opaque_total` was
proven with i64 parameters assumed. So a contract such as `requires p(a, 9223372036854775807 + 1)` applied F where
its body could be undefined, and the inline expansion would have rejected it. A call-site range obligation was tried
first and dropped: it also refuses `forall j in 0..len(a): p(a, a[j])`, because an `[i64]` element has no i64 fact.
The landed fix narrows what totality assumes, to the shape facts only (P13o.1). The repro is
tests/must_reject/l3_opq_range. `shifted` in tests/l3/proof/opaque_reveal.pruv covers out-of-i64 arguments. The
review found no reveal leak across branches, loop iterations or calls, and no gap in the array encoding, the
code-call fact or module visibility.
A confirmation review of the fix found nothing required. Contracts cannot allocate or slice arrays, and `old` and
`inout` keep lengths, so no contract argument breaks the kept array facts. Opaque parameters are limited to `i64`,
`bool`, `[i64]`, `[u8]` and `[bool]`, so the map fact is unused today.

## Appendix P13a.9 (P13a step 2 landed: mutators keep `valid`; per-call `valid` removed from `m4_op`/`m4_observe`)

P13a step 2 (P13a.3, P13o.7) lands with the P13o verifier, without a language change. Every write function of
bud_store, bud_map and the 11 bud_world mutators ensures `kept(st, old(st))`, and `m4_op` ensures
`valid(old(st)) ==> valid(st)`. So step 3 lands in the same package: `m4_op` and `m4_observe` no longer check `valid`
per call.

### P13a.9.1 Method

- **`shape` by congruence.** `shape(st)` is a transparent wrapper, `len(st) >= 232 && shape_w(len(st), st[0], st[1],
  st[2], st[3], st[4], st[5], st[6], st[9], st[11])`, where `shape_w` is an `opaque fn` over i64 words. A mutator that
  shows `shape_same(st, old(st))` (equal length and shape words) gets `shape(st)` from `shape(old(st))` because an
  opaque call is a function symbol: equal arguments, equal result. `shape_w` is revealed only in `ea` (entry address,
  which needs the slot count), and never in a mutator, so the power-of-two layout test never reaches a mutator query.
- **`dyn_ok` revealed at the write exits, after the frame.** `dyn_ok` (words 7, 8, 10, 12, 13, 14 against 6, 9, 11) is
  opaque. Each write exit first asserts the header words it leaves equal (`hdr_same` / `shape_same` plus the single
  words) and the values it wrote (for example `st[7] == top + 1`, `st[12] == live + 1`), and only then says
  `reveal dyn_ok;`. The revealed body then closes over plain equalities instead of store chains. Exits that write
  nothing need no reveal. Reveals: bud_store `wr16`, `set_ef`, `fill_entry`, `arena_put`, `entry_alloc`,
  `entry_free`; bud_map `sh`, `write_at`'s loop, `link_fast`, `set_blob`; none in bud_world or root_m4. P13o.7 revealed at the function top, which is what timed out.
- **`kept` as a proposition.** `kept(st, o) = len(st) == len(o) && (!shape(o) || shape(st)) && (!dyn_ok(o) ||
  dyn_ok(st))`. It is transparent and cheap: reflexivity and chaining through a sequence of calls need no reveal, and
  `valid(o) ==> valid(st)` follows from it with `len(st) >= 232` from `valid`'s own first term.
- **Header writer lemma.** `bud_map::sh` (the one header writer) ensures `wok(old(st), i, v) ==> kept(st, old(st))`.
  `wok` is small and transparent: true off the header and on buffers shorter than 15, the `dyn_ok` bound on words 7, 8,
  10, 12..14, false on the shape words. Callers (`put_units`, `take_words`, `remove`, `rehash`, ...) prove `wok` with
  linear facts and never reveal `dyn_ok`.
- **Writes past the header.** Slot and region writes go through `bud_store::wr16` (`ent_u16::wr` that also skips
  addresses below 16), so they keep the header frame on any buffer. `write_at`'s fast path additionally requires
  `o >= 16`; `make_room`'s fit test uses `o >= 16` (was 15). On a valid store every such address is >= 16 already.
- **`put_at` split.** Its direct-word linking block is now `link_fast(e, n, h, be, bo)` (same writes, same order), so
  `put_at`'s query no longer carries the field writes; `put_at` went from 0.40 s to 0.15 s (Python), `link_fast` 0.13 s.
- **Loops.** The mutating loops of `write_at`, `put_key_h`, `remove`, `set_blob` and `rehash` carry
  `invariant kept(st, old(st))`; the loop body proves one step with the lemmas above.

### P13a.9.2 Lemmas and ensures

| file | functions with `ensures kept(st, old(st))` (or the `valid` form) |
|---|---|
| bud_store | `wr16`, `set_ef`, `fill_entry`, `arena_put`, `entry_alloc`, `entry_free` (+ `shape_w` opaque, `wok`, `kept`, `shape_same`) |
| bud_map | `sf`, `sh` (under `wok`), `write_at`, `link_fast`, `put_at`, `put_key_h`, `put_units`, `take_words`, `room`, `make_room`, `rehash`, `set_blob`, `drop_entry`, `drop_child`, `drop_desc`, `remove`, `add_w`, `add_absent_w` |
| bud_world | `put_blob`, `del_key`, `session_put_w`, `session_del`, `clock_put`, `clock_del`, `clock_set`, `run_set`, `run_del`, `tool_note`, `tool_forget` |
| m34_test/root_m4 | `dec_key`, `dec_put`, `dec_two`, `dec_set`; `m4_op`: `valid(old(st)) ==> valid(st)` |

### P13a.9.3 Contract change

`m4_op` and `m4_observe` take `st` from `m4_init`, `m4_grow` or an earlier `m4_op`. On any other buffer they stay
memory-safe and terminate (the bounds and loop limits are unchanged) but their answers are unspecified; status 2 now
means a bad op encoding only (`m4_grow` still checks `valid(prev)`). m34_test/CONTRACT.md says so. The `nv` lanes of
run.py are now the same code as the default lanes (the driver check accepts that). Behaviour on corrupt buffers
changed in one more way: slot and region writes below address 16 are skipped instead of landing on the header.
P10.4 kept the per-call `valid` until the handle records that init returned 0; here the proof covers every state the
entries produce, so the record is not needed for these two calls. A handle that accepts a foreign buffer still needs
it (the poison-flag step, below).

### P13a.9.4 Proof time (user CPU s, one run each, same machine, same session; load about 3)

HEAD (ab6b291) sources and P13a.9 sources, both with the worktree's compilers (no compiler change):

| root | Python HEAD | Python P13a.9 | Rust HEAD | Rust P13a.9 |
|---|---:|---:|---:|---:|
| m34_test/root_m4 | 7.79 | 9.20 | 6.91 | 5.20 |
| m8_test/root_m8 | 8.87 | 9.23 to 9.83 | 7.57 | 5.61 to 5.94 |
| m9_test/root_m9 | 8.16 | 9.30 | 7.22 | 5.35 |
| m34_test/root_admit | 8.02 | 8.34 | 7.04 | 4.91 |
| m7_test/root | 9.58 | 10.37 | 7.75 | 5.95 |
| m10_test/root_m10 | 10.38 | 11.44 | 8.43 | 6.62 |
| m11_test/root | 14.02 | 14.37 | 10.17 | 8.02 |

Gate (root_m8 <= 1.2x P13a.4: Python 10.09, Rust 9.24): met on both, with Python close (9.23 to 9.83 over three runs).
Rust is faster than HEAD on every root: the hidden `shape_w`/`dyn_ok` bodies leave most queries smaller than the
expanded `valid` of HEAD. Python is 4 to 18% slower than HEAD, mostly in `remove` (below).

Per-write-function time, Python `pruvc` profile on root_m8 (s; `-` = below the profile's top 60, under 0.02 s):

| function | HEAD | P13a.9 | | function | HEAD | P13a.9 |
|---|---:|---:|---|---|---:|---:|
| `bud_store::fill_entry` | 0.85 | 0.34 | | `bud_map::put_at` | 0.40 | 0.15 |
| `bud_store::entry_free` | 0.42 | 0.33 | | `bud_map::link_fast` | - | 0.13 |
| `bud_store::entry_alloc` | 0.26 | 0.24 | | `bud_map::put_key_h` | 0.05 | 0.14 |
| `bud_store::arena_put` | 0.10 | 0.10 | | `bud_map::write_at` | 0.02 | 0.14 |
| `bud_store::set_ef` | 0.06 | 0.05 | | `bud_map::set_blob` | 0.09 | 0.38 |
| `bud_store::wr16` | 0.04 | 0.05 | | `bud_map::rehash` | 0.04 | 0.27 |
| `bud_map::sh` | - | 0.05 | | `bud_map::drop_entry` | 0.18 | 0.17 |
| `bud_map::sf` | - | 0.02 | | `bud_map::remove` | 0.79 | 1.23 |
| `bud_map::put_units` | - | 0.03 | | `bud_map::drop_desc` | - | 0.04 |
| `bud_map::take_words` | - | 0.02 | | `bud_world::put_blob` | - | 0.14 |
| `bud_world::tool_note` | - | 0.13 | | `bud_world::run_set` | - | 0.04 |
| `bud_world::clock_put` | - | 0.03 | | other bud_world mutators | - | 0.02 or less |

Profile totals: root_m8 8.86 (HEAD) and 9.11; root_m4 7.49 and 8.92 (root_m4 adds `dec_*`, `m4_op` at 0.02 to 0.05
each). The largest new cost is `remove`'s loop (+0.44 s): its invariant carries `kept`, and each iteration chains
`kept` through `drop_child`/`drop_entry` and the pending word-14 write.

### P13a.9.5 Performance (Rust backend)

M4 (`perf_headroom/host/run.py --only m4 --lanes m4nv --rounds 5`, ns per op, median of 7 alternating rounds for the
adapter table):

| axis | n | hand | m4 | m4raw | m4nv | m4/hand | m4raw/hand | m4raw/m4 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| sessions | 100 | 14.6 | 11.6 | 13.1 | 11.6 | 0.79 | 0.90 | 1.13 |
| sessions | 500 | 15.5 | 11.3 | 12.7 | 11.3 | 0.73 | 0.82 | 1.12 |
| sessions | 1000 | 15.3 | 11.7 | 13.1 | 11.8 | 0.76 | 0.86 | 1.12 |
| tools | 100 | 51.8 | 29.7 | 31.4 | 29.9 | 0.57 | 0.61 | 1.06 |
| tools | 500 | 52.2 | 32.8 | 34.9 | 32.8 | 0.63 | 0.67 | 1.06 |
| tools | 1000 | 52.9 | 32.1 | 34.1 | 31.8 | 0.61 | 0.64 | 1.06 |
| clocks | 100 | 37.7 | 24.2 | 29.2 | 24.3 | 0.64 | 0.77 | 1.21 |
| clocks | 500 | 38.9 | 23.7 | 27.3 | 23.8 | 0.61 | 0.70 | 1.15 |
| clocks | 1000 | 38.9 | 24.3 | 28.3 | 24.8 | 0.62 | 0.73 | 1.17 |

- `m4` now equals `m4nv` (P13a.5 had `m4 - m4nv` of 0.4 to 1.5 ns, the per-call `valid`).
- Every cell is at or under 0.90x hand Rust.
- `m4raw/m4` above 1.1x (sessions 1.12 to 1.13, clocks 1.15 to 1.21) is Pruv against Pruv, not a hand ratio: `m4raw`
  goes through the `PruvStore` poison flag (P11.4, P13a.5; 1.4 to 1.5 ns on sessions here). The clocks gap
  (3.6 to 5.0 ns) is wider than in P13a.5 (1.8 to 2.2 ns), and `m4raw` clocks has the widest round spread of the
  table (27.5 to 31.7 ns at n=100); it is still 0.70 to 0.77x hand. Removing the flag is step 4.

M8 (`m8_test/run_m8.py perf --rounds 5 --no-instr`, HEAD and P13a.9 generated sources run alternately twice,
`--gen`), Pruv / hand16 ns: charge 20.1 to 21.8 against 21.0 to 25.1, failure 16.7 to 20.4 against 16.2 to 18.9,
tooldup 23.8 to 26.8 against 25.1 to 28.5, toolgrow 48.7 to 53.5 against 90.7 to 97.9. P13a.9 equals HEAD in every
cell within noise. `failure` n=500 (1.08 to 1.15x hand16) is above 1.0x on HEAD as well; it is not from this package.

M7 (`m7_test/run_m7.py perf --rounds 5 --profile SM --no-instr`, HEAD and P13a.9 alternated twice), Pruv typed /
hand16: `attach` 1.10 to 1.14 and `closereopen` 1.05 to 1.12 on both HEAD and P13a.9 (pre-existing). `ishalted` reads
1.03 to 1.12 on P13a.9 against 0.95 to 1.02 on HEAD, but `c_is_halted` reaches none of the functions this package
changes (its 36 generated functions are identical text on both), so the difference is code placement in the driver
binary, not Pruv work. M9 (`run_m9.py perf --rounds 5`): 0.73 to 0.85x hand. M10 and M11 perf were not run; they
use the same changed writers, whose generated code differs only as listed in P13a.9.3.

### P13a.9.6 Gates

Both compilers (Rust: worktree release build; Python: `pruvc/pruvc.py`):
- `run.py admit`: 26/26;
- `run.py lanes --count 300`: all 9 lanes 300/300;
- all seven budget roots above prove.

Not run, because their trigger did not apply: `cargo test --release`, `diff_rust.py`, `g8_frontend` (no compiler
change), the bench/l3/lib user tests (bench/l3/lib unchanged). No audit: the package adds no language semantics; the
contract change is the removed per-call check, stated in P13a.9.3.

### P13a.9.7 Remaining work

- Step 4: remove the `PruvStore` poison flag (`m4raw/m4` sessions and clocks above 1.1x). A handle that may receive a
  buffer the entries did not produce needs the "init returned 0" record of P10.4 (or a validated ingress) first.
- Step 5: I1 ensures and adapter wiring (P13a.8).
- Python proof time on `remove` (1.23 s, HEAD 0.79 s): moving the loop body into a helper with its own `kept`
  ensures would shrink the loop query.
- M7 `attach`/`closereopen` and M8 `failure` n=500 are above 1.0x hand on HEAD too; they need their own row audit
  (§4.1).

## Appendix P13a.10 (P13a steps 4 and 5: no poison flag on a no-unwind store; admit proves I1, `m4_import`)

### P13a.10.1 Method

Step 4 (no poison flag). The `PruvStore` poison flag (P11.4) exists because an entry that panics halfway through its
writes leaves the state half-updated. In a verified Rust-backend program that is impossible except at one place:
every check inside a body is erased (Pruv callers call `f__pruv_body`, the caller proved `requires`, no cost counters),
and every operation the verifier accepted is total on the proved domain (unchecked indexing, plain arithmetic, lengths
bounded by 2^40). The only panic left is the entry's own `requires` check, which runs before the body writes. So the
judgment "the state is the result of complete entry calls" is a consequence of the proof, and the host keeps only the
side effect (owning `T`). Both compilers compute the same predicate, `raw_unwind_free`:

    verified && !printed && !g6 && !g10 && !proc && !net && !ev && par callees empty

and then emit `RAW_STORE_FREE` instead of `RAW_STORE`: the same API (`new`, `with`, `is_poisoned`, `PruvPoisoned`), so
adapters compile unchanged, but `with` is `Ok(f(&mut self.data))` and `is_poisoned` is `false`. Excluded, keeping the
flag: `--no-verify` (bodies keep checks), `print` (`println!` panics when stdout fails, mid-body), foreign code (G6
boundary checks and trusted Rust may panic), the G10
file / process / socket / event runtimes, `par` groups (threads, locks), and the LLVM backend and the C adapter (their
store is a separate text; unchanged in this package). Allocation failure and stack overflow abort and do not unwind.
SPEC.md "No-unwind store" states the rule, and one adapter rule the types cannot enforce: the `with` closure changes
`T` only through the entries (a host write to `T` followed by a host panic inside `with` is no longer caught). No
language change, so no must_reject rows.

Step 5 (I1 as ensures). P13a.8 left admit's I1 facts as runtime checks without a stated contract. They are now a
ground predicate on raw words:

- `ent_i1(st, e, top, ab, at)` (`pub opaque fn`, bool, no requires) states for entry e, at
  `st[c32(st[5]) + 20 * e + f]`, bud_store::ea's address (a valid store has `st[5] = 16 + slots <= 2^32 + 16`; the base
  is clamped to 2^32 as `c32` clamps it; an opaque body allows only immutable lets, so the two base cases call the
  plain bool `ent_w(st, b, ...)`): state 0 or 1; roots 0..2 live with owner -1; key span
  (words 4, 18) and blob span (12, 19) inside the arena `[ab, at)`; a free entry's next link -1 or below top; a live
  non-root entry's owner below top; key length in `0..2^30` and `kn <= 4 * kcap` (`words(kn) <= kcap`); blob length
  within its cap; links 7..10 -1 or below top; word 11 >= 0.
- `span_w` replaces `span_ok`: the same test without `requires` (it tests the arena bounds first), because a function
  with `requires` is not expanded into a contract.
- `entry_ok` reads the words directly instead of through `ef`/`words` (both have `requires`, so their bodies are
  invisible to the ensures), returns the same codes in the same order (-3, -2, -5, -6, -8, -4, -5, -6, -7), and
  `ensures result >= 0 ==> ent_i1(st, e, top, ab, at)`.
- admit tests the header words it bounds by as plain words (`0 <= st[5] <= 2^32 + 16`, `top = st[7] <= 2^32`,
  `0 <= st[9] <= st[10] <= 2^40`; every valid store passes, else -1) and its entry loop carries
  `forall j in 0..e: ent_i1(st, j, top, ab, at)`.

One encoding fact matters here (P13o): an opaque call is the uninterpreted `opq!F` symbol, but under a quantifier
whose bound variable reaches an argument it is expanded to its body. entry_ok's ensures gives the symbol at `e`; the
invariant's forall needs the body at `j = e`. The loop therefore says `reveal ent_i1;` after the entry check, which
ties the two. Callers of admit see only the symbol form unless they reveal it.

Adapter wiring (P11.5): `root_m4.pruv` gets `m4_import(st, inout cnt) -> i64`,
`ensures result == 0 ==> bud_store::valid(st)`, for a buffer handed in from outside the entries (2 on any admit
failure). The Rust-store drivers (`m4`, `m4raw`, `m4nv`, `m4eq`, `m4llvm*`) load it as an adapter would load a host
buffer: right before the program's `observe` they copy the state into a fresh store, admit it (must return 0 with at
least the three roots visited), check that its observation equals the old store's, and swap it in, so the observation
checked against the model runs on the imported store; a copy whose entry 0 state word is 7 must return 2 and is never
served from. The C-handle lanes (`m4c`, `m4cb`) have no host buffer and admit the handle's own state (last field
`-`). `run.py` requires exactly one `import` line per program with those values and prints
`import admission checks: N/N programs`.

### P13a.10.2 Changed contracts

| function | before | after |
|---|---|---|
| `bud_admit::admit` | `result >= 0 ==> valid(st) && result <= 2^32` | adds the header bounds above and `forall e in 0..st[7]: ent_i1(st, e, st[7], st[9], st[10])`; header words out of those bounds now return -1 (no valid store has them) |
| `bud_admit::entry_ok` | none | `requires 0 <= ab <= at <= 2^40`, `0 <= e < top <= 2^32`; `ensures result <= 1`, `result >= 0 ==> ent_i1(...)` |
| `bud_admit::ent_i1` | (new) | opaque predicate, above |
| `bud_admit::ent_w` | (new) | ent_i1's word tests at a given base, no requires |
| `bud_admit::span_ok` | span test with requires | removed; `span_w` (no requires) |
| `root_m4::m4_import` | (new) | `result == 0 ==> valid(st)` |
| `PruvStore` (Rust backend, raw entries) | poison flag always | no flag when `raw_unwind_free`; same API |

Not stated by `ent_i1` (entry_ok still checks them at run time): the owner's live state and the key padding
(`key_pad_ok` reads through `ent_u16::rd`, which returns i64 through a branch and cannot appear in a contract).

### P13a.10.3 Proof time (wall s, `/usr/bin/time -p`, two runs each, same machine and session)

HEAD (4a1018b) bench sources against this package's final sources (after the audit fixes, P13a.10.7), both with the
worktree compilers (the compiler change is codegen only):

| root | Rust HEAD | Rust P13a.10 | ratio | Python HEAD | Python P13a.10 | ratio |
|---|---:|---:|---:|---:|---:|---:|
| m34_test/root_admit | 5.21, 5.21 | 5.75, 5.58 | 1.07 to 1.10 | 8.61, 8.56 | 10.17, 10.26 | 1.19 |
| m34_test/root_m4 | 5.34, 5.34 | 6.16, 5.93 | 1.11 to 1.15 | 9.48, 9.58 | 10.56, 10.58 | 1.11 |
| m8_test/root_m8 | 5.99, 5.97 | 6.07, 6.04 | 1.01 | 9.83, 9.91 | 9.93, 10.24 | 1.01 to 1.04 |

Gate (root_m8 <= 1.2x of Rust 6.0 s, Python 9.8 s): met, 1.01x Rust and 1.04x Python at most (root_m8 does not import
bud_admit, so its spread is noise). The added cost on root_admit and root_m4 is admit's entry loop (the per-entry
forall and its reveal), entry_ok's ensures and the two base cases of ent_i1 (the c32 clamp); before the clamp it was
1.03 to 1.09x.

### P13a.10.4 Performance (Rust backend)

`bench/l3/budget/perf_headroom/host/run.py --only m4 --lanes m4nv --rounds 5`, ns per op, median of 7 alternating
rounds, after step 5:

| axis | n | hand | m4 | m4raw | m4nv | m4/hand | m4raw/hand | m4raw/m4 (P13a.9) | m4raw/m4 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| sessions | 100 | 14.7 | 11.6 | 11.8 | 12.1 | 0.79 | 0.80 | 1.13 | 1.02 |
| sessions | 500 | 15.7 | 11.3 | 11.4 | 11.3 | 0.72 | 0.73 | 1.12 | 1.01 |
| sessions | 1000 | 15.5 | 11.9 | 11.8 | 11.7 | 0.77 | 0.76 | 1.12 | 0.99 |
| tools | 100 | 52.6 | 29.9 | 30.0 | 30.3 | 0.57 | 0.57 | 1.06 | 1.00 |
| tools | 500 | 53.8 | 33.3 | 33.2 | 32.9 | 0.62 | 0.62 | 1.06 | 1.00 |
| tools | 1000 | 53.1 | 32.1 | 32.4 | 32.4 | 0.60 | 0.61 | 1.06 | 1.01 |
| clocks | 100 | 37.8 | 24.6 | 24.4 | 24.4 | 0.65 | 0.65 | 1.21 | 0.99 |
| clocks | 500 | 39.0 | 23.6 | 23.7 | 24.4 | 0.61 | 0.61 | 1.15 | 1.00 |
| clocks | 1000 | 39.2 | 24.8 | 24.8 | 24.6 | 0.63 | 0.63 | 1.17 | 1.00 |

Gate (`m4raw/m4 <= 1.1x`): met, 0.99 to 1.02x (P13a.9: 1.06 to 1.21x). A run after step 4 alone gave 0.98 to 1.02x.
Every cell is at or under 0.80x hand Rust. `m4_import` is not on the op path.

### P13a.10.5 Gates

Both compilers (Rust: worktree release build; Python: `pruvc/pruvc.py`):
- `run.py admit`: 26/26 (codes unchanged);
- `run.py m3 --count 300`: 300/300;
- `run.py lanes --count 300`: all 9 lanes 300/300, stats checks 1922/1922, import admission checks 300/300;
- `cargo test --release`, `tests/diff_rust.py --fuzz-count 20` (all same), `run_all.py --only g8_frontend` (27/27
  Python, 39/39 Rust) and `--only l3_abi` (2/2) on both compilers (check_raw_entries asserts the flag is dropped
  exactly when verified on the Rust backend and kept for `--no-verify`, LLVM, a foreign program and a printing
  program), then the full `run_all.py` on both compilers.

### P13a.10.6 Remaining work

- The LLVM backend store and the C adapter keep the flag; the same rule could drop it there.
- host.rs `ResidentState` keeps its own flag (not generated code).
- A handle that receives a buffer the entries did not produce must go through `m4_import` first; nothing in the types
  forces it (op-before-init and foreign buffers are adapter rules). The C adapter has no import of a host buffer;
  its `m4_import` admits the handle's own state.
- `ent_i1` omits the owner's live state and the key padding; stating them needs an i64 word read usable in contracts.
- A quantified opaque call is expanded, so a loop invariant over an opaque predicate needs `reveal`; a quantifier
  form that keeps the symbol would avoid it.
- Python proof time on `remove` (P13a.9.7): the helper with its own `kept` ensures was not done in this package.
- check_raw_entries has no `par` or G10 runtime program case (the predicate excludes them; only foreign and print are
  exercised).
- `m4_import` forwards only `valid(st)`; it could also forward admit's `forall ... ent_i1` for a caller that reveals it.

### P13a.10.7 Audit

One review (Codex, delegate role review) of the package diff. Required findings and what was done:

1. `print` lowers to `println!`, which panics when stdout fails, after body writes: a verified printing program could
   leave a half-written state under the flag-free store. Fixed: `printed` joins the predicate in both compilers;
   check_raw_entries gains a printing program that must keep the flag.
2. A host closure that writes `T` and then panics inside `with` is no longer caught. Not a code change: such a write
   already breaks the adapter rule (state changes only through the entries) and the forwarders' safety text; SPEC.md
   now states the rule next to the store.
3. At the largest layout (`st[5] = 2^32 + 16`) admit returned -1 where HEAD returned -2, and ent_i1's base differed
   from `ea`'s clamped base. Fixed: the bound is `2^32 + 16` and the base is `c32(st[5])` in ent_i1 and entry_ok.
4. `m4_import` was called on the final state only, so no op or observation ran on an imported store. Fixed: the Rust
   drivers import a copy before `observe`, compare and swap it in (P13a.10.1).
5. run.py passed a program with no `import` line and accepted `-` on every lane. Fixed: exactly one line, `2` on the
   Rust lanes and `-` only on the C-handle lanes.

Optional findings (left as remaining work above): par / runtime cases in check_raw_entries; forwarding the I1 ensures
through `m4_import`. A confirmation review of the five fixes found 1, 2, 3 and 5 done; on 4, a second `init` in the
same `q` session kept the first world's import result. Fixed: `init` resets it, so each world is imported before its
own `observe` (checked by hand with `init 232, observe, init 232, observe`; generated programs have one `init`). Findings and decisions: `work/p13a45/audit_findings.md` (not committed).
