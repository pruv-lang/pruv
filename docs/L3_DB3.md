# L3 DB3: entry op 9 RSS (perf debt bundle, part 3)

Goal: close D-DB2-1 and D-DB2-2 (= D-R02-1): entry op 9 RSS M and L at or below hand Rust 1.0x, time cells kept.

## Where the RSS goes (L, 4000 sessions, measured once with getrusage probes, probes removed)

| phase | Pruv max RSS | delta |
|---|---|---|
| JSON parsed to `serde_json::Value` (host.rs `run_one`) | 18.35 MB | — |
| `encode_index` done (`Enc` pool + rows + refs) | 22.00 MB | +3.65 |
| `mk_packet` (input built while `enc` still held) | 24.51 MB | +2.51 |
| `index_begin` returned (await built while input held) | 28.10 MB | +3.59 |
| `index_step` returned (done packet) | 28.61 MB | +0.51 |
| hand Rust whole run | 20.14 MB | parse + 1.79 |

The parse share (18.35 MB) is the same `serde_json::Value` tree in both lanes. Hand adds 1.8 MB on top of it, Pruv adds 10.3 MB.
The L input packet is 301,353 words, of which 125,340 are pool (one `i64` per UTF-16 unit). The await is 438,723 words.
`prev` is not a copy: the driver passes the emitted `Rc` back (`host.rs` `drive`), and the input is dropped right after `begin`.
So the "`prev` read back from the driver" share named in D-DB2-1 does not exist.

## What landed (step 2 of the brief)

- `entry_api.pruv` `index_begin_checked`: the await request pool (section 0) now holds only the id texts referenced by the table.
  Each `[6, start, len]` slot is rewritten to the compact offset. The full blob still sits in the state section, and `index_complete` reads it there.
  L await: 438,723 → 332,273 words (−106,450 words, −0.85 MB).
- `index_begin_checked` rejects `len(progress) <= ns` (session count) instead of `len(progress) <= len(blob) / 30`.
  `ent_bound.pruv` `index_collect` takes `ns < len(progress)` as a precondition.
- `harness.rs` `index_progress_len` sizes `progress` from the record's session count, clamped to 559,240 and to `input.len() / 30`, plus 1.
  The forge path uses the same rule.
- `host.rs` `mk_packet_owned` (`host.rs:345`, used at `host.rs:2087`) consumes `Enc` and drops each buffer after copying it.
- Step 1 (byte pool) did not land: see D-DB3-4.
- The DB2 progress-root proof is unchanged.

## Tests

| fragment | test | result |
|---|---|---|
| entry | `dev_build.sh`, `forge_build.sh` | built |
| entry | `test_impl.py` | 1164 runs, unexplained 0 |
| entry | `q.py` | 2000 cases: 1996 exact, 4 excluded-domain, 0 unexplained |
| entry | `forge_test.py` | failures 0 (23 forged cases rejected, C9 included) |
| entry | `judge_test.py` | failures 0 |
| entry | `q_mutants.py` | 16/16 killed |
| req | `dev_build.sh`, `test_impl.py` | built; 640 runs, unexplained 0 |
| req | `q.py --impl work/dev/harness` | 2000/2000 equal |

The budget build was not rerun because no `ent_u16` signature changed.

## Perf

`perf_r02.py`, 7 alternating rounds, medians, wait4. Raw cells: `work/db3/perf_cells.json`.

| cell | before (DB2) | after (DB3) |
|---|---|---|
| M time | 0.985x | 1.039x (11.9 ms vs 11.5 ms) |
| L time | 0.979x | 1.160x (20.1 ms vs 17.3 ms) |
| M RSS | 1.296x | 1.249x (8.22 MB vs 6.59 MB) |
| L RSS | 1.420x | 1.366x (27.56 MB vs 20.17 MB) |

The run ran under load average 57 (other sessions on the host).
A follow-up 15-round L time check on the same binaries gave 0.986x (31.2 ms vs 31.7 ms).
So the L time cell is not confirmed as a regression. It has to be re-measured on a quiet host (D-DB3-5).

req `perf_req.py --no-node --sizes 1000,5000` (one run per cell): before `work/db3/req_perf_before.md`, after `work/db3/req_perf_after.md`.
- RSS is unchanged within 1 MB on every row (list 5000/50/last: 300 → 305 MB).
- Times are within noise on the list rows. The open rows and list 5000/50/last are slower in this single run (9.68 → 15.15 s) under the same host load.
- req does not use op 9. Its only path touched by this change is `mk_packet_owned`.

## Remaining debt

| id | cell | line-level cause (hand vs generated Rust) |
|---|---|---|
| D-DB3-1 | RSS M, L | `host.rs:1269` `encode_index` builds `Enc` from the `serde_json::Value` (+3.65 MB at L). Its pool is one `i64` per UTF-16 unit (125,340 words at L). Hand reads the `Value` directly and has no encode layer. |
| D-DB3-2 | RSS M, L | `host.rs:2087` the input packet (301,353 words, 2.4 MB at L) stays alive across `sol.begin` (`host.rs:1937` `drive` takes it by value). `mk_packet_owned` frees `Enc` buffers as it copies, but the packet itself is still one full copy of `Enc`. |
| D-DB3-3 | RSS M, L | `entry_api.pruv:149` copies the whole blob into the await state section while the input is still held, so two blob copies are live at the peak. Generated Rust mirrors this (`vec![0; total]` + copy). Hand keeps no state between calls. Removing it needs step to read the blob from somewhere other than the await, which changes the DB2 progress-root proof (the await is what step checks). |
| D-DB3-4 | RSS M, L | Step 1 not done (D-R02-1). A `[u8]` pool with spans and WTF-8 for lone surrogates would cut every pool share above by 8x. It changes the wire shared by entry and req (`host.rs` `encode_utf16`/`mk_packet`, the `bench/l3/req` consumer, `ent_packet` readers). That is more than one fix try in the 2 h cap. |
| D-DB3-5 | time M, L | The gate cells (1.039x, 1.160x) were measured under load average 57. A 15-round recheck gives L 0.986x. Re-run `perf_r02.py` on a quiet host before the gate reads these cells. |

Parse share: both lanes parse into the same `serde_json::Value` (18.35 MB at L), so it is not part of the gap.
Parsing the sessions into the packet straight from the JSON stream would put Pruv under hand. But that changes the adapter, not the Pruv code, so it is left as a parent decision and not done here.

## Gaps

None. No compiler workaround was needed.

## Status

Status: DB3: D-DB2-1 partial (RSS M 1.296x→1.249x, L 1.420x→1.366x; rest carried as D-DB3-1..3)
Status: DB3: D-DB2-2 open (byte pool not done; carried as D-DB3-4)
Status: DB3: D-R02-1 open (same as D-DB2-2; carried as D-DB3-4)
