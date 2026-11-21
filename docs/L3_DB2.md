# L3 DB2: entry op 9 resident progress (perf debt bundle, part 2)

Scope: close D-R02-1..3 and G-R02-1 (`docs/L3_RESTRUCTURE_SPEC.md` §7.5) for the entry M/L cells of
`bench/l3/entry/perf_r02.py` (op 9 `bound_session_index`). Target: ≤ 1.0x of hand Rust on time and RSS.

## What landed

- Resident op 9 path. `index_begin(inout progress, input)` (`bench/l3/lib/entry_api.pruv:71`) checks the input once,
  records the positions of predicate sessions in `progress` (`ent_bound.pruv:235` `index_collect`) and emits the await.
  The Rust adapter (`bench/l3/entry/harness.rs:9-54`) stores only `progress` and the emitted await packet.
- `index_step(inout progress, prev, emitted, completion)` (`entry_api.pruv:141`) rejects any `prev` that differs from
  `emitted`. It checks the state section bounds, the request row count (`emitted` request table == `progress[0]`), and
  every stored position (`ent_bound.pruv:259` `index_progress_ok`: strictly increasing, whole 30-word row inside the
  blob's session table, predicate true when read from the blob). Then it validates the completion and writes the done
  packet directly (`ent_bound.pruv:292` `index_complete`). There is no `input_check`/`log_ok` re-run and no replay from
  pos 0. v1 `entry_begin`/`entry_step` reject op 9.
- G-R02-1 workaround: offset-based readers (`new_meta_at`, `meta_start`, `str_start_at`, `key_put_at`) read the blob
  in place instead of moving it into a `Ctx` field.
- `host.rs`: copy/scan cuts with byte-identical output. `pair_table` decodes both slots of a row before rendering, as
  in HEAD (keeps the error priority). `encode_index` takes its arguments by value.
- Forgery: a new A/B swap case (`forge.rs` `index_input`, `forge_test.py` C9) pairs `progress` from begin(A) with
  `emitted`/`prev` from begin(B). It is rejected.

## Tests (fragment)

| suite | result |
|---|---|
| entry `test_impl.py` | fixed 1,164 pass |
| entry `q.py` | random 1,996/2,000 (4 excluded by spec) |
| entry `forge_test.py` | 23/23 forgeries rejected |
| entry `judge_test.py` / `q_mutants.py` | 8/8, 16/16 |
| req `test_impl.py` / `q.py --impl work/dev/harness` | 640 fixed, 0 unexplained / 2,000/2,000 |
| budget `check_oracle.py` | 140/140 |

## Perf (perf_r02.py, 7 rounds, final run once)

| cell | before pruv / hand | before x | after pruv / hand | after x |
|---|---|---:|---|---:|
| time M | 8.07 / 4.36 ms | 1.850x | 5.16 / 5.24 ms | 0.985x |
| time L | 25.88 / 13.48 ms | 1.920x | 14.45 / 14.76 ms | 0.979x |
| RSS M | 19.87 / 6.59 MB | 3.017x | 8.54 / 6.59 MB | 1.296x |
| RSS L | 72.40 / 20.15 MB | 3.593x | 28.62 / 20.15 MB | 1.420x |

`tools/perf_gate.py`: time passes, RSS M/L fail. Req `perf_req.py --no-node --sizes 1000,5000`: the wire is unchanged.
The times match before within single-run noise (e.g. list M=50 none S=5000: 11.02 s before, 9.99 s after).

## Debt

| id | cells | cause (hand vs generated) |
|---|---|---|
| D-DB2-1 | RSS M 1.296x, L 1.420x | Hand holds the JSON as bytes. Pruv still holds text as one `i64` per UTF-16 unit (D-R02-1 not done), so the L input is 438,723 words (3.5 MB). Each resident copy costs 8 bytes per unit: the input, the await (`entry_api.pruv:106-108` allocates `30 + lp + lb + lt`, and `lb` is the whole blob, so the pool is in the await twice: request section and state blob), `prev` read back from the driver, and the done packet (`ent_bound.pruv:292`), which is allocated while the input and await are still held. Generated Rust mirrors this one-to-one (`sol.rs` `index_begin_checked`: `vec![0; total]`). `harness.rs:29` also sizes `progress` at `input.len() / 30 + 1` words. Fix: the byte pool (D-DB2-2). Also emit the await without the second pool copy, and size `progress` from the session count. |
| D-DB2-2 | RSS (see D-DB2-1) | D-R02-1 is not done. A `[u8]` UTF-8 pool with spans changes the packet shared by entry and req (`host.rs` `encode_utf16`/`mk_packet`, `bench/l3/req` consumer, `ent_packet` readers). It also needs a lone-surrogate encoding (WTF-8) to keep v1 semantics. That is a wire change across two lanes, larger than one fix try in the 2 h cap. |

## Gaps

None new. G-R02-1 is still a compiler limit, worked around by the offset readers above.

## Status

Status: DB2: D-R02-1 open (byte pool not done; RSS 1.296x/1.420x carried as D-DB2-1, D-DB2-2)
Status: DB2: D-R02-2 closed (no input_check/log_ok re-run, no replay; time M 0.985x, L 0.979x)
Status: DB2: D-R02-3 closed (one owned progress root checked once at begin; forged state rejected, forge 23/23)
Status: DB2: G-R02-1 closed (workaround: offset readers, no blob move into Ctx; compiler unchanged)
