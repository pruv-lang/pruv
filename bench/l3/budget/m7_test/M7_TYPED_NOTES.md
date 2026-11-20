# M7 typed resident lane

Session ids are `String` keys and budgets are the shared `Budget` record. The time strings and extra attributes of the
old text arena are kept in `String` fields of the record. A separate insertion ordinal map keeps the observation order;
a new session returns status 2 before its ordinal reaches 2^40.

`bud_tlife::view` handles current, isHalted and reopen; `bud_tlife::maybe_close` handles maybeClose. attach and
replaceForUserMessage update the budget in the typed root through `map_get_mut` and use the shared `bounded`, `settle`,
`disarm` and `emit`. `bud_tdecide` is imported as a shared module; decide calls belong to M10. Rust reads the same Q
wire (packed UTF-16 and text), passes the UUID and isQuiet inputs, and records effects and results.

Run from `bench/l3/budget`. With `M7_WORK` set to an absolute path, generated sources and perf JSON go to that folder.

```sh
python3 m7_test/run_m7.py q
python3 m7_test/run_m7.py --py q
python3 m7_test/run_m7.py --typed q
python3 m7_test/run_m7.py --typed --py q
python3 m7_test/run_m7.py --typed same
python3 m7_test/run_m7.py --typed perf --profile SM
python3 m7_test/run_m7.py perf --profile SM
```

On both compilers the flat and typed Q totals over 300 cases are the same: 113 exact, 187 stopped at a later milestone,
0 known differences and 0 failures. The generated Rust is identical from both compilers (64,446 bytes).

Typed perf uses pre-decoded inputs. close restores state outside the timed region; replace restores the halted record
inside the timed region, as the hand baseline does. Perf logs and JSON are in `work/p31t7/w3/` (not committed).

A `map_get_mut` record binder can only update fields, so a new budget is initialized field by field. Values of the
String-keyed ordinal map are wrapped in an `Ordinal` record. Both paths use existing language features.
