# L3-8 engine-import common entry

A Pruv implementation of the common entry of the engine conversation import (ops: import_sources, find_source,
list_external, read_external, account_home, codex_account_folder, session_threads, excluded_from_bound,
bound_session_index). The TypeScript original is pinned in `reference/` and run as the oracle. Design: `DESIGN.md`,
spec: `docs/ENGINE_IMPORT_ENTRY_SPEC.md`. Results are in RESULTS.md section 140 (the lead fills them in).

## Files
- `*.pruv`, `pruv.json`: the Pruv sources (root `root.pruv`). `host.rs`, `host/`, `harness.rs`: the Rust host crate and runner.
- `reference*.mjs`, `reference/`, `oracle.py`: the oracle. `fixtures/`, `gen_fixtures.py`: fixtures and their generator
  (also holds MUTANT_KILLERS). `common.py`: loaders and result comparison.
- `test_impl.py`: fixtures x pairs. `integrate.py`: the I group. `q.py`: seeded generator (Q_README.md).
  `q_mutants.py`: mutant runner. `judge_test.py`: excluded-domain judgement check. `forge.rs`, `forge_build.sh`, `forge_test.py`: forged-replay rejection and deep-climb
  cost tests (R1, R3). `perf_wb.py`: scaling check for large session indexes (R2, R4). `IMPL_NOTES.md`: implementation notes.

## Commands (from this directory; PATH needs ~/.cargo/bin, GROK_TS_TRANSPILER the TypeScript package)
- `./dev_build.sh` : development build.
- `python3 test_impl.py [--pairs rs/rust] [--only GROUP] [--case ID] [--reuse-verified]` : fixtures against expect, oracle and across pairs.
- `python3 integrate.py` : the I group.
- `python3 gen_fixtures.py` : regenerate fixtures.
- `python3 oracle.py cases.jsonl` : run the oracle alone.
- `python3 q.py [--seed N] [--count N] [--families G1,..] [--ops 1,..] [--pairs rs/rust] [--reuse-verified] [--jobs N]` : seeded cases, work/q/report.json.
- `sh forge_build.sh && python3 forge_test.py` : forged replays are rejected, honest ones pass, climb cost is linear.
- `python3 judge_test.py` : the excluded-domain judgement needs the forbidden value to be really reached.
- `python3 perf_wb.py [keys] [S ...]` : wall time of bound_session_index at S sessions.
- `python3 q_mutants.py [--only M08,M09] [--jobs N]` : mutants, work/mut/report.json. Exit 0 only if all selected are killed or verification-rejected.

## Results
In RESULTS.md section 140.
