# L3 DB8: entry wire byte pool (D-DB4-1 step 1)

## What landed

- Step 1 landed at the entry Sol boundary. Section 0 (the string pool) of every entry packet that crosses the host/Pruv boundary is now a WTF-8 byte stream packed 7 bytes per word. Steps 2 (the ~216 reader sites read bytes directly) and 3 (D-DB4-2 beyond the cheap host drops) did not land.
- `bench/l3/lib/ent_wire.pruv` (new): `wire_in` is the strict decoder (host to Pruv). `wire_out` is the encoder (Pruv to host). `bench/l3/entry/root.pruv` wraps every exported function as `wire_out(entry_api::f(wire_in(..)))`.
- `bench/l3/entry/host.rs`: `Enc` keeps WTF-8 bytes plus a unit counter (`push_units` joins a high surrogate at the end of the stream with a low surrogate at the start of the next string into one 4-byte form). `mk_packet`/`mk_packet_owned` write the wire pool. `parse_pkt` decodes it strictly and calls `ie()` on anything `wire_out` cannot produce. `mk_packet_units`, `parse_pkt_units` and `Enc::units()` keep the old unit format for req.
- `bench/l3/req/leaf.rs`: req's Pruv calls `entry_api` directly, so every entry packet inside req stays in the unit format (`mk_packet_units`, `parse_pkt_units`, `Enc::units()`). req output is unchanged (q 2000/2000, test_impl 0 unexplained).

## Wire format (section 0)

`[nu, W_1..W_m]`, where `nu` is the number of UTF-16 units of the logical pool P. B is the WTF-8 bytes of P: a high surrogate followed by a low surrogate is one 4-byte form, and a lone surrogate is its 3-byte form. W_{i+1} = sum_{j=0..6} B[7i+j] * 256^(6-j). Bytes past the end count as 0, so m = ceil(len(B)/7) and every W is in 0..2^56-1. An empty pool is `[0]`. Slots stay `[6, unit_off, unit_len]` in units of P.

- `wire_in` rejects overlong forms, scalars above U+10FFFF, truncation, stray continuation bytes, a 3-byte high surrogate followed by a 3-byte low surrogate (that pair must be the 4-byte form), a unit count other than `nu`, a nonzero byte after the data and a word outside 0..2^56-1. A rejected pool becomes the one-unit pool `[-1]`, which `env_check` rejects. A malformed section table (bad `nsec` or section lengths) and `w[4] != n - 5` pass through unchanged, so `entry_api` gives the same verdict as before.
- Whole zero words after the data are accepted on input. The host's exact-limit probe (`pad_to`, `host.rs`) pads section 0 with zero words to reach the limit. `wire_out` never writes them, so the wire form is canonical on output but not on input (D-DB8-4).

## Deviation from the DB4 design

DB4 planned the byte pool inside `entry_api` and its readers. `entry_api.pruv` and the other lib modules are outside this lane, so the conversion sits at the entry root instead. The judgments still live in Pruv (`wire_in` is Pruv). The host only encodes and decodes. Inside Pruv the pool is still one `i64` per unit, so the Pruv-side RSS cannot drop until step 2 lands.

## Audit

One review of the decoder (Codex gpt-6.2-astra, ultra) found three must-fix items and one should-fix. All are fixed:

- `poison` with `nsec > 1000000`: `env_check` reads the pool at `7 + min(nsec, 1000000)`, so it missed the `[-1]` pool. `poison` now also clears the version word in that case.
- `host.rs` `corrupt(.., "unit")` wrote the unit count 65536, which is valid for a 65536-unit pool. It now writes -1.
- `host.rs` test `corrupt_and_pad` expected the strict host output decoder to accept padding and a bad unit count. It now expects both to fail (`cargo test`: 7 passed).
- should-fix: `wire_in` rejects `nu > 7 * (l0 - 1)` before it allocates the unit pool.

## Tests (whole list, after the last change)

- entry: `dev_build.sh` ok, `forge_build.sh` ok. `test_impl.py`: 1164 runs, 0 unexplained. `q.py`: 1996 exact + 4 excluded, 0 unexplained. `forge_test.py`: 0 failures. `judge_test.py`: 0 failures. `q_mutants.py`: 16/16 killed.
- req: `dev_build.sh` ok. `test_impl.py`: 640 runs, 0 unexplained. `q_gen.py` ok, `q.py --impl work/dev/harness`: 2000 cases, 0 problems.
- budget (build `ent_*`, run before the audit fixes; no budget module imports `ent_wire`, and the fixes touch only `ent_wire` and `host.rs`): m7 q 113 exact, m9 q 26 exact, m10 q 78 exact, m11 213 exact + 87 prefix-ok, m12 crafted 8 + lifecycle 7 exact, m34 m3/m4 ok. m8 was not run (D-DB8-3). No budget module imports `ent_wire`.

## Perf (perf_r02.py, 7 rounds, load 4.1-4.4, before the audit fixes, which add one compare per packet)

| cell | DB4 | DB8 |
|---|---|---|
| time M | 1.042x | 1.046x |
| RSS M | 1.234x | 1.354x (8962048 vs hand 6619136) |
| time L | 1.051x | 1.042x |
| RSS L | 1.378x | 1.419x (28573696 vs hand 20135936) |

RSS got worse. Each boundary now holds two copies of the packet at once inside Pruv (`wire_in`'s unit packet next to the wire input, and `wire_out`'s wire packet next to the unit output), and the host holds the decoded unit pool next to the wire words. Streaming `wire_out` (no byte array) did not move the cells. req `perf_req.py --no-node --sizes 1000,5000` ran clean with no SUSPECT-QUADRATIC rows.

## Debt

| id | cells | debt |
|---|---|---|
| D-DB8-1 | RSS M, L | Step 2 not done: `entry_api` and its ~216 reader sites still read one `i64` per unit, so `wire_in`/`wire_out` must expand and pack whole packets at the root. The byte pool only shrinks the wire, not the Pruv working set. |
| D-DB8-2 | RSS M, L | RSS regressed 1.234x→1.354x (M) and 1.378x→1.419x (L). Cause: the boundary copy in Pruv (input and expanded packet live together, output and packed packet live together) plus the host's decoded `Pkt.pool` next to the wire section. It goes away with D-DB8-1 (readers on bytes, no expansion) and host readers that decode slots lazily. |
| D-DB8-3 | m8 | `budget` m8 q/derive need `work/m8/cases.jsonl`, which exists neither in this worktree nor in the main checkout. Not run. m8 does not import `ent_wire`. |
| D-DB8-4 | wire canonicity | Input accepts whole zero words after the data (exact-limit padding by `pad_to`). Two wire inputs can therefore decode to the same pool. Remove when `pad_to` pads somewhere that is not section 0. |

## Gaps

| id | gap | workaround |
|---|---|---|
| G-DB8-1 | No bitwise operators: byte packing and UTF-8 classification use `/`, `%` and powers of 256 (`pw`). | Works, at some cost per byte. Not measured separately. |

Status: DB8: D-DB4-1 partial (wire byte pool at the entry boundary with a strict Pruv decoder; Pruv readers still on units, carried as D-DB8-1)
Status: DB8: D-DB4-2 partial (host drops the serde Value and Enc before sol.begin and mk_packet_owned drops the pool early; RSS still above, carried as D-DB8-2)
Status: DB8: D-DB4-3 open (not worked; L peak now 28.6 MB, carried with D-DB8-2)
Status: DB8: D-DB4-4 partial (7 rounds at load 4.1-4.4: M 1.046x, L 1.042x; still above 1.0x)
