# Typed M8

The driver holds one generated `St` per case. Sessions own budget records, and turns own activity maps.
Order vectors preserve insertion order. Forgotten turn slots are skipped by ordinal, including after reinsertion.
Turn slots compact above `2 * live + 16` after forgetting a turn, which also drops its activity vector. Check with `run_m8.py churn --typed [--py]`.
The wire codec preserves every UTF-16 unit, including lone surrogates.
The observe reply uses `Vec<Key>` for R-6f-3. Halt dispatch still cuts the Q case at the M10 boundary.

`run_m8.py q --typed [--py]` uses the same cases and oracle as the flat lane.
`run_m8.py perf --typed` checks final-state checksums and reports five-round medians at 100 and 500 sessions.
All three perf lanes time packed-wire input, including the typed driver's key decoding.
