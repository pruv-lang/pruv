# M10b budget raise

`bud_decide.pruv` owns the decision, validation, capacity checks, record changes and clock transitions. The driver supplies time and unit answers, runs timer effects, and records the Pruv result. Its query log places `E unit` at the effect-word position passed by Pruv. The comparison checks the five unit values and their position among effects, as well as M10's request and final-state observations.

The numeric path preflights room before querying unit when the clock might need refreshing. It applies any rehash only once the answer is valid and a refresh is needed. This keeps Grow retries query-free and preserves the store for invalid answers. The reservation allows a text activeSince rewrite and the fixed part of a nested halt. The halt uses the existing running-id answer and terminal status contract, with one spare effect word for the outer changed notification. Unlimited clears clock field 16 and preserves the interval and record activeSince.

## Complexity by flow

Let S and B be the work of the existing session and clock lookups, including hashing packed keys, collision probes and key comparisons. Let O be the packed owner-key comparison, W the record length in words, D the number of stopped delegation IDs, and I their total packed words. The five budget keys and the unit frame are fixed size. These bounds cover Pruv decision and effect emission. Host timer handling uses the M10 scheduler.

| Flow | Work and space |
|---|---|
| R1 missing or closed | O(S) time and O(1) additional space |
| R2 already unlimited | O(S + B) time and O(1) space |
| R3 newly unlimited | O(S + B) time and O(1) space, up to two effect words |
| R4 size one | O(S) time and O(1) space |
| R5 per-key maximum | O(S + B) time and O(1) space, including a single fixed-size unit answer |
| R6 numeric no-op | O(S + B) time and O(1) space, including the read-only room plan when applicable |
| R7 settle and refresh | O(S + B + O) in the ordinary case and O(1) space. A text rewrite adds O(W) time and space. A nested halt adds O(W + D + I) time and space plus another lookup and owner comparison |
| R8 final notification | O(1) time and space after the selected path |
| R9 halted budget | R5/R6 or R7 settlement costs, with the halted state retained |
| R10 numeric on unlimited | O(S + B) time and O(1) space with an append for changed when a limit rises |

Reserve may trigger the shared index rehash. It scans slots and entries and reinserts live entries, so probes depend on hash spread. A Grow retry copies the store through the shared growth path. R7 does not scan run entries. Refresh uses the maintained active count.

## Changed files

- `bench/l3/budget/bud_decide.pruv`
- `bench/l3/budget/m7_test/bud_facts.rs`
- `bench/l3/budget/m10b_test/root_m10b.pruv`
- `bench/l3/budget/m10b_test/run_m10b.py`
- `bench/l3/budget/m10b_test/driver_m10b.rs`
- `bench/l3/budget/m10b_test/hand_m10b.rs`
- `bench/l3/budget/m10b_test/M10B_NOTES.md`

## Classified differences

- KD-P30-1. Only Boolean true is treated as unlimited. Truthy non-Boolean input is outside the generated corpus.
- KD-P30-2. A scaled unit above 2^53−1 returns status 2 before record or effect writes.
- KD-P30-3. A malformed unit answer returns status 2 before record or effect writes.
- KD-P30-4. An unsupported size code on a current, non-closed budget returns status 2.
- R10 intentionally retains numeric limit updates on unlimited budgets. R15 retains the shared clock owner rule.

## Verification

| Check | Result |
|---|---|
| M10b derive | 300 cases, 428 raise calls |
| M10b q, Rust and Python pruvc | Each 300 exact, 0 cut, 0 fail, 428 raise calls compared |
| M10b h, Rust and Python pruvc | Each H01–H10 exact, 14 raise calls compared, 6 rejected-input controls and 1 Grow/time-halt retry passed |
| M10b same | Byte-identical, 339,635 bytes each |
| M7 q | 113 exact, 187 cut, 0 fail |
| M8 q (regenerated Q2 at seed 20261117) | 63 exact, 237 cut, 0 fail |
| M9 q | 26 exact, 274 cut, 0 fail, 1,835 active-count checks with 0 failures |
| M10 q | 78 exact, 222 cut, 0 fail |
| M10 cases_halt q | 300 exact, 0 cut, 0 fail |
| M11 q, both compilers | Each 213 exact, 87 matching prefixes, 0 fail, identical pair output |
| M12 q, both compilers | Each Q7 300, Q7P 300, L 10, crafted 8, lifecycle 7 exact, 0 fail |

Q5-20261117-26-raise exposed a display mismatch on both compilers. The M10 decoder applied a stale opaque-rest `unlimited: undefined` after reading word 2 as true. M10b uses header precedence for this observation. Both final comparisons and request observations use the same projection. The case is included in the exact count.

## Performance

Five interleaved rounds, 10,000 measured raises per lane per round, after 1,000 warmups. D is the number of clock runs and staged delegation IDs per lane. One run consumes time. Each sample includes the public raise and its host timer effects. Budget reset and the pause/resume that re-enters the interval run outside the measurement. Both lanes do the same host work in the sample: one public call on a frame prepared outside the sample (only the time word is written) and a single timer slot that holds the armed resume timer with the same expiry before each sample (`perf_raise` in the driver, `Host::perform` in the hand lane). The M10 scheduler with its logging, audit and per-budget timer index runs only in q mode and in the setup calls.

The table shows medians across rounds. Peak RSS came from `getrusage(RUSAGE_CHILDREN)` on a fresh wrapper per sample. In this sandbox `/usr/bin/time -l` fails its `kern.clockrate` query before reporting RSS.

| Path | D | Pruv ns/op | Hand ns/op | Pruv alloc/op | Hand alloc/op | Pruv RSS bytes | Hand RSS bytes | Hold |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| Time | 10 | 130.6 | 82.0 | 1.00 | 1.00 | 2195456 | 1622016 | No (overlap) |
| Time | 50 | 85.5 | 62.4 | 1.00 | 1.00 | 2277376 | 1654784 | Yes |
| Time | 100 | 85.7 | 60.2 | 1.00 | 1.00 | 2326528 | 1671168 | Yes |
| Limits | 10 | 65.4 | 56.0 | 1.00 | 1.00 | 2195456 | 1622016 | Yes |
| Limits | 50 | 66.3 | 56.7 | 1.00 | 1.00 | 2293760 | 1638400 | Yes |
| Limits | 100 | 67.6 | 55.8 | 1.00 | 1.00 | 2342912 | 1671168 | Yes |
| Unlimited | 10 | 35.3 | 32.9 | 0.00 | 0.00 | 2162688 | 1605632 | No (overlap) |
| Unlimited | 50 | 35.0 | 33.1 | 0.00 | 0.00 | 2244608 | 1622016 | Yes |
| Unlimited | 100 | 34.4 | 33.8 | 0.00 | 0.00 | 2310144 | 1654784 | No (overlap) |

Every cell is O(1) in D and well under 1 ms, so none is data-proportional. The remaining gap is fixed per call:

- All paths, about 2 to 12 ns. `t_call` decodes the op frame, `current_entry` hashes and compares packed keys, and `clock_of` does a second lookup. Every state access goes through bounds-checked `ent_u16::rd`/`wr`. The hand lane reads a typed struct.
- Time path, about 20 ns more. `bud_halt::rec_ok_in` validates the whole stored record frame (`text_frame`, O(W)) before the refresh writes. `room_plan` probes the store before the unit query. `view_in` copies the header into `bv`, and three words are written back after settle and refresh. The hand lane has a typed record that needs none of these.
- RSS, about 0.5 to 0.65 MB. The Pruv binary carries the whole M10 world: the state arena, 4096-word `fx` and `out` buffers, the text arena and a larger text segment.

The first version of the harness timed `World::call`. That added the M10 scheduler's owned key copies (three allocations per timer set), its timer index and its logging checks, which the hand lane does not do. It measured 240 ns and 4 allocations on the time path. The review then found that the Pruv sample still copied the op frame with its packed keys and started with an empty timer slot; both were fixed (the frame is built once, the slot holds the armed timer with the hand lane's expiry), which moved unlimited from 45 to 35 ns.

## Gaps

`work/p30/gaps.md` records three proposed follow-ups. The clock could expose a disarm-only operation to encapsulate field 16. A shared record decoder could use the header precedence already tested in M10b. Nested halt inherits status 6 for an answer larger than the reserved arena or effect frame, pending the existing in-call growth follow-up.
