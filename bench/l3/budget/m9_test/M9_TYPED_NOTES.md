# Typed M9

The resident session, clock, and per-clock run maps assign monotonic insertion ordinals. A new entry at the `2^40` ordinal limit returns status 2. A run stores its ordinal and waiting flag in a `Run` record. DROP and CLOSE remove the clock once, releasing its run table and run keys.

Observation and I2 recount walk the live maps. Observation returns `Vec<String>` keys alongside fixed-layout scalar words and ordinals. The driver sorts those ordinals when formatting Q output. Calls use the existing effect reply.

Generated Rust and hand Rust on the hot path:

| Operation | Generated Rust | Hand Rust (`hand_m9.rs`) |
| --- | --- | --- |
| Find a run for mutation | `match c.runs.get_mut(key) {` | `match c.runs.get_mut(key) {` |
| Insert a new run | `c.runs.insert(pruv_m_f_1_5_owned(key), (Run { ordinal: ordinal, waiting: false }));` | `c.runs.insert(key.to_string(), false);` |
| Remove a clock and release its run table | `let removed: bool = st.clocks.remove(bid).is_some();` | `if self.clocks.remove(&r.budget_id).is_some_and(\|clock\| clock.timer.is_some()) {` |

Typed and flat Q share fixture resolution. With `--py`, `--pruvc` selects the Python script explicitly. Typed DC checks retained session and clock table capacity across cycles and applies the flat lane's `2^32` entry bound to the reported counts.

The typed compilers produced identical Rust for the new root. On each compiler, the Q wire output was byte-identical before and after the change. Each reported 26 exact cases, 274 milestone matches, zero failures, and 1,544 I2 checks with zero mismatches. Churn and all 313 DROP/CLOSE cases passed on both compilers.

The single perf smoke reported 2,876.4 ns/call for typed DROP and 2,341.0 ns/call for typed CLOSE at 100 runs. See [raw runner output](work/p31t6/m9_perf_smoke.txt).
