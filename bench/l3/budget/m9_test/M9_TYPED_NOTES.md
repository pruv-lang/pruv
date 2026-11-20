# Typed M9
The host retains one generated St per case and preserves raw UTF-16 keys. Changes span root_m9_typed.pruv, driver_m9_typed.rs, driver_m9.rs, run_m9.py, and this note.
Flat rs, typed rs, and typed py each had 26 exact Q matches, 274 matches through the existing cutoff, 0 failures, and 313/313 DROP/CLOSE passes.
Run `python3 bench/l3/budget/m9_test/run_m9.py q --typed` (add `--py` for Python) or `perf --typed`. Five-round medians are in ns/call (typed rs / flat rs / hand Rust), at 200,000 repetitions per workload.

| Workload | Size 10 | Size 50 | Size 100 |
| --- | --- | --- | --- |
| Refresh | 191.0 / 111.6 / 139.1 | 191.8 / 112.9 / 138.1 | 191.5 / 114.4 / 141.3 |
| Drop | 403.5 / 146.1 / 180.6 | 1003.8 / 272.2 / 505.8 | 1814.9 / 412.9 / 941.0 |
| Close | 404.3 / 151.4 / 226.6 | 1027.2 / 273.1 / 550.7 | 1867.8 / 424.6 / 982.0 |

At size 1, cycle took 221.5 / 117.9 / 158.0, fire 197.0 / 130.2 / 204.3, stale 143.7 / 69.2 / 63.3, and staleu 148.4 / 73.3 / 66.5 (measured after compaction).
Run and clock order compact above `2 * live + 16` after removal, preserving live order and waiting flags. Verify with `run_m9.py churn --typed` (optionally `--py`).
R-6e-6 (SPEC.md:509, root_m9_typed.pruv:8). Both compilers reject `pub struct Run { waiting: bool } pub struct Clock { runs: HashMap<String, Run> } pub struct St { clocks: HashMap<String, Clock> } pub fn count(inout st: St) -> i64 { return len(st.clocks); }` with `type/foreign_owned_type: String-keyed maps require i64, bool or owned aggregate values`. This lane uses M4's scalar run values. Proposed root row: permit owned records without nested collections as values in these nested maps, with in-place mutation and cross-compiler coverage.
