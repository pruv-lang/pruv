# q.py: seeded generator

Default `--seed 20261118 --count 2000`. Case ids are `Q0000`-style (`Q%04d`, 1-based position in the generated list).

## Design
- PRNG: xorshift32. Uniform choices use rejection sampling and always consume a word. The family multiset is
  Fisher-Yates shuffled with the same PRNG, so the same seed and count give the same list.
- Families and default sizes: G1 400 (import_sources, plus account_home about 20%), G2 300 (codex_account_folder),
  G3 200 (find_source / route), G4 300 (titles), G5 300 (errors), G6 200 (session_threads),
  G7 200 (excluded_from_bound / bound_session_index), G8 100 (corrupted packets and probes).
  Other counts scale by largest remainder, so only the default count has the exact split and numbering differs across counts.
- `--families` and `--ops` (1 import_sources, 2 find_source, 3 list_external, 4 read_external, 5 account_home,
  6 codex_account_folder, 7 session_threads, 8 excluded_from_bound, 9 bound_session_index) filter after generation,
  so case numbers stay stable.
- G7 tags about 3% of cases `excluded_ok` (a bound engine given as a ref), compared with excluded=True.
- G8 probes derive from the oracle trace. Only import_sources has several step boundaries; the other ops send their
  requests in one await, so only at=0 is a boundary (op 9 emits all callbacks in one batch). `duplicate` is reachable
  only through import_sources (others use `version`). Clamp for import_sources: at = min(at, max(n-1, 0)); duplicate:
  at = max(1, min(at, n-1)), needs n >= 2. exact_limit expects the oracle; reentry expects InvalidPacket with the full
  trace; other corruptions expect InvalidPacket (InputTooLarge for oversize) with trace[:at].

## Run and report
Oracle once per case (no probes), each selected pair, cross-pair comparison. A crash is bisected (chunks of 250) to the
failing case. Shrinking is greedy over args and script (`--shrink-max`).
Classes: exact, P5, P6, engine-policy (always 0 for Q), excluded-domain, unexplained. A crash, harness error, missing
result, oracle refusal, InternalInvariant or cross-pair mismatch is always unexplained. P6 is counted only when the oracle
value contains InputTooLarge, so oversize probes are not counted there.
Output: work/q/report.json (counts, per_family, per_pair, unexplained_cases) and work/q/fail/<n>.json (input, oracle,
pruv per pair, first differing field, shrunk input). Exit 1 on any unexplained case or build error.
