# Typed M8

The driver holds one generated `St` per case. Sessions own budget records, and turns own activity maps.
Each map assigns insertion ordinals from a monotonic counter and refuses new entries at 2^40.
Replacing a budget keeps its ordinal. Forgetting a turn removes its map entry and activities, and reinserting it assigns a new ordinal.
The churn check covers repeated forget and reinsert, wire order, and ordinal exhaustion.
The wire codec preserves every UTF-16 unit, including lone surrogates.
Observation iterates maps directly and returns `Vec<String>` keys with ordinals and record data in `words`.
Only the wire formatter sorts by ordinal. The key encoding adds no work to accounting entries.
Halt dispatch still cuts the Q case at the M10 boundary.

`run_m8.py q --typed [--py]` uses the same cases and oracle as the flat lane.
Use `--pypruvc PATH` to select a Python compiler script explicitly.
`run_m8.py perf --typed` checks final-state checksums and reports five-round medians at 100 and 500 sessions.
All three perf lanes time packed-wire input, including the typed driver's key decoding.
