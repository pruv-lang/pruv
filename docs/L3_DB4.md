# L3 DB4: entry op 9 RSS, await without blob, compact done pool (perf debt bundle, part 4)

Goal: close D-DB3-4 (= D-DB2-2 = D-R02-1) and what is needed of D-DB3-1..3 so entry RSS M (1.249x) and L (1.366x) reach hand Rust 1.0x, time cells kept at or below 1.0x.
Fixed decision: no host JSON parsing that differs from hand (no streaming parse); the fix is on the Pruv representation side.

Result: the await no longer copies the blob and the done pool is compact, but RSS did not move measurably (L 1.366x → 1.378x, M 1.249x → 1.234x).
The byte pool (step 1) did not land. The goal is not reached; see Remaining debt.

## Where the RSS goes (L, 4000 sessions, getrusage probes, 6 repeated runs, probes removed)

| phase | Pruv max RSS |
|---|---|
| before `sol.begin` (JSON `Value` + `Enc` + input packet) | 23.36–23.42 MB |
| `index_begin` returned (await, no blob) | 23.45 MB |
| `index_step` returned (done packet) | 24.41–24.50 MB |
| done decoded, result body formatted (`run_one`) | 24.42–24.59 MB |
| whole process, probed binary | 25.62–25.80 MB |
| whole process, clean binary (perf cell) | 27.79 MB |
| hand Rust whole run | 20.17 MB |

Before DB4 the begin phase added +3.59 MB (DB3 table). It now adds +0.07 MB, and step adds about +1 MB.
The share before `begin` (23.4 MB) already exceeds the whole hand run (20.2 MB). Only a smaller host encode (`Enc` + input packet, one `i64` per UTF-16 unit) can close that, i.e. the byte pool.
The last 1–3 MB is reached after the body is formatted (output write and teardown). It also moves with the allocation pattern: adding probes lowered the process peak by 2 MB.
The reason is macOS malloc. Memory freed from the `serde_json::Value` tree (tiny/small zones) is not reused for the large packet `Vec`s, so lowering what is live at one phase does not lower the process peak.

## What landed

- `entry_api.pruv` `index_begin_checked` (`entry_api.pruv:90`): the op 9 await state is 14 words and holds no blob copy.
  - `st+12` = checked pool high water `hi`, `st+13` = raw input length.
  - The await request pool stays compact (DB3).
- `entry_api.pruv` `index_step` (`entry_api.pruv:180`) takes the input packet, which the adapter keeps.
  - It checks `ls == 14` and `emitted[st+13] == len(input)`.
  - It uses the input directly, or repeats `ent_state::repack(input, hi)` when begin repacked.
  - Then it runs `index_step_on` (`entry_api.pruv:202`).
  - The DB2 progress-root proof is unchanged: step still checks the emitted await against prev and reads the session table from the await.
- `ent_bound.pruv` `index_complete` (`ent_bound.pruv:292`) builds a fresh compact done pool.
  - A pre-pass sums the value string lengths (`vsum`, boundary error past 2^24).
  - The kept value strings are copied first, then the key texts, instead of copying the whole input pool.
- `harness.rs` `Index` keeps `input` and passes it to step. `begin` takes the input by value (`Vec<i64>`).
- `host.rs` `Sol::begin` takes `Vec<i64>`, and `drive` hands over `first` (`host.rs:1954`).
- `root.pruv` forwards the new `index_step` signature. `forge.rs` tamper keeps the op 9 state unchanged (14 words).
- A trial that reused the pool `Vec` in `mk_packet_owned` raised RSS and was reverted.
- Step 1 (byte pool) did not land: D-DB4-1. The wire design is fixed and written down (below).

## Byte pool design (not implemented)

The text pool stays section 0 of the `[i64]` packet, but holds UTF-8 bytes, 8 per word.
- Byte k lives in word k/8, little endian.
- Strings start on a word boundary and are zero-padded.
- A slot is `[6, start_word, byte_len]`.
- Bytes are WTF-8 (a lone surrogate is the 3-byte form, a pair is always one 4-byte scalar). The strict decoder rejects overlong forms, values > U+10FFFF, truncation, stray continuation bytes, surrogate pairs split into two 3-byte forms, and spans past the pool (the same error path as today).
- The Pruv side decodes into the existing `ent_u16` unit buffers, so no `ent_u16` signature changes.
- Equality may compare words. Order must go through units.
- About 216 read and write sites (entry `ent_packet` readers, host `encode_utf16`/`mk_packet`/decode, and the req consumer) change together.

## Tests

| fragment | test | result |
|---|---|---|
| entry | `dev_build.sh`, `forge_build.sh` | built |
| entry | `test_impl.py` | 1164 runs, unexplained 0 |
| entry | `q.py` | 2000 cases: 1996 exact, 4 excluded-domain, 0 unexplained |
| entry | `forge_test.py` | failures 0 (23 forged cases rejected) |
| entry | `judge_test.py` | failures 0 |
| entry | `q_mutants.py` | 16/16 killed |
| req | `dev_build.sh`, `test_impl.py` | built; 640 runs, unexplained 0 |
| req | `q_gen.py` + `q.py --impl work/dev/harness` | ok; 2000/2000 equal |

The budget build was not rerun because no `ent_u16` signature changed.

## Perf

`perf_r02.py`, 15 alternating rounds (load average 19.4 at start, above 16), medians, wait4. Raw cells: `work/db4/perf_cells.json`.

| cell | before (DB3, 7 rounds, load 57) | after (DB4, 15 rounds, load 19.4) |
|---|---|---|
| M time | 1.039x (11.94 vs 11.50 ms) | 1.042x (13.56 vs 13.02 ms) |
| L time | 1.160x (20.11 vs 17.33 ms) | 1.051x (26.86 vs 25.56 ms) |
| M RSS | 1.249x (8.22 vs 6.59 MB) | 1.234x (8.04 vs 6.52 MB) |
| L RSS | 1.366x (27.56 vs 20.17 MB) | 1.378x (27.79 vs 20.17 MB) |

The time cells are noisy at this load. A 5-run check on the same binaries gave M 0.907–1.11x and L 0.75–0.89x.
RSS is stable across runs (±0.1 MB).

req `perf_req.py --no-node --sizes 1000,5000` (one run per cell, load 13.8 → 9.4): `work/db4/req_perf_after.md`.
- RSS is unchanged on every row.
- Times are equal or faster than DB3 (list 5000/50/last 15.15 → 9.15 s; open 5000/none 0.67 → 0.48 s). The DB3 run had a higher load.

## Remaining debt

| id | cell | line-level cause (hand vs generated Rust) |
|---|---|---|
| D-DB4-1 | RSS M, L | Byte pool not done (the old D-DB3-4 = D-DB2-2 = D-R02-1). The design is above. It changes about 216 sites across the entry readers, `host.rs` `encode_utf16`/`mk_packet`/decode and the `bench/l3/req` consumer. That did not fit the 2 h cap next to steps 2–3. |
| D-DB4-2 | RSS M, L | `host.rs:1270` `encode_index` (`Enc`, one `i64` per UTF-16 unit) and the input packet from `host.rs:346` `mk_packet_owned` are live before `sol.begin`. At L that is 23.4 MB, above the whole hand run (20.2 MB). Hand reads the `Value` directly. D-DB4-1 shrinks both pools 4–8x. |
| D-DB4-3 | RSS L | The process peak (27.8 MB) is reached after `run_one` (`host.rs:2124`) has formatted the body (24.6 MB at that point), in output writing and teardown. It shifts by about 2 MB with the allocation pattern (macOS malloc does not reuse the freed `Value` zones for the large `Vec`s). Not localized further. |
| D-DB4-4 | time M, L | 1.042x / 1.051x at load 19.4 over 15 rounds. Short checks scatter 0.75–1.11x. Re-run `perf_r02.py` on a quiet host (load < 4) before the gate reads these cells. |

## Gaps

| id | gap | workaround |
|---|---|---|
| G-DB4-1 | An `inout` array parameter cannot be reassigned as a whole (`input = repack(input, hi)` is rejected), so step cannot swap the kept input for its repack in place. | The await stores `hi` and the raw length, and step repeats `repack(input, hi)` when `hi` differs from the pool length. This costs one repack per step on repacked inputs, not one stored blob. |

## Status

Status: DB4: D-DB3-1 open (RSS M 1.249x→1.234x, L 1.366x→1.378x; carried as D-DB4-2, D-DB4-3)
Status: DB4: D-DB3-2 partial (the input is no longer copied into the await; the adapter keeps one input across begin/step; host share before begin carried as D-DB4-2)
Status: DB4: D-DB3-3 closed (await holds no blob; step reads the kept input; DB2 progress-root proof kept)
Status: DB4: D-DB3-4 open (byte pool designed, not implemented; carried as D-DB4-1)
Status: DB4: D-DB3-5 partial (15 rounds at load 19.4: M 1.042x, L 1.051x; quiet-host rerun carried as D-DB4-4)
