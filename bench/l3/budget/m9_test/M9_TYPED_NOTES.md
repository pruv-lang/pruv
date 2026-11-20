# Typed M9

The host retains one generated St per case and preserves raw UTF-16 keys. Changes span root_m9_typed.pruv, driver_m9_typed.rs, driver_m9.rs, run_m9.py, and this note.
Flat rs, typed rs, and typed py each had 26 exact Q matches, 274 matches through the existing cutoff, 0 failures, and 313/313 DROP/CLOSE passes.
Run `python3 bench/l3/budget/m9_test/run_m9.py q --typed` (add `--py` for Python) or `perf --typed`. Five-round medians are in ns/call (typed rs / flat rs / hand Rust), at 200,000 repetitions per workload.

| Workload | Size 10 | Size 50 | Size 100 |
| --- | --- | --- | --- |
| Refresh | 200.8 / 115.2 / 144.3 | 201.5 / 116.4 / 145.0 | 203.7 / 121.5 / 143.3 |
| Drop | 423.7 / 152.0 / 188.4 | 1085.1 / 279.0 / 522.3 | 1915.2 / 471.8 / 970.1 |
| Close | 420.2 / 158.5 / 231.1 | 1077.4 / 289.6 / 577.3 | 1924.2 / 464.6 / 1016.2 |

At size 1, cycle took 235.6 / 122.7 / 164.3, fire 202.9 / 138.7 / 210.8, stale 145.7 / 71.7 / 65.5, and staleu 148.7 / 75.7 / 69.2.
R-6e-6 (SPEC.md:509, root_m9_typed.pruv:8). Both compilers reject `pub struct Run { waiting: bool } pub struct Clock { runs: HashMap<String, Run> } pub struct St { clocks: HashMap<String, Clock> } pub fn count(inout st: St) -> i64 { return len(st.clocks); }` with `type/foreign_owned_type: String-keyed maps require i64, bool or owned aggregate values`. This lane uses M4's scalar run values. Proposed root row: permit owned records without nested collections as values in these nested maps, with in-place mutation and cross-compiler coverage.
