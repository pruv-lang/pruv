# M12 typed lane

Sessions live in `HashMap<String, Budget>` and the per-instance run clocks in `ClockState`.
Lifecycle decisions go through `bud_tlife` and `bud_tstate`; a clock is removed with one
`map_take`. Insertion ordinals use the same 2^40 cap as M10.
The V-word codec keeps field order, duplicate keys and UTF-16 units. Auto-continue loads the
referenced sessions into a temporary Pruv arena, calls `bud_auto`, and writes the edit back to
the typed map. `maybeClose`, `raise` and `decide` are not in the M12 protocol's op list.

`run_m12.py --typed` runs the existing Q corpus in `work/p29/m12/typed`.
`--typed --perf` uses the same lifecycle timing loop as the flat lane: setup, cloning,
observation and checksums run outside the timer; the call and the scheduler apply run inside it.
