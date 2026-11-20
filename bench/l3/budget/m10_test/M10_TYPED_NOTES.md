# M10 typed lane

`St` in `root_m10_typed.pruv` stores sessions and clocks under String keys. Replacing a session keeps its insertion
ordinal; a new session whose ordinal would reach 2^40 returns status 2. The shared modules manage the clock and run
insertion ordinals. `t_put`, `t_del`, `t_get` and `t_observe` live in the typed root. Read entries pick the record to
serialize and the Rust driver writes it in the Q format. Q UTF-16 keys are carried in String through the shared codec,
which keeps lone surrogates.

The lane reuses `bud_tlife::view` (current, isHalted, reopen), `maybe_close`, `run_event` (including FIRE) and
`explicit_halt`. charge combines `bud_tstate::charge`, `settle` and `bud_tlife::halt` and builds the refusal reason as a
String. Halt reason codes in the shared Budget keep the existing encoding. `bud_tdecide` is imported, but decide and
raise belong to M10b. Rust only stores the state, does I/O, answers facts and runs timer effects.

Run from `bench/l3/budget`:

| item | command |
|---|---|
| flat Q | `python3 m10_test/run_m10.py q`, `python3 m10_test/run_m10.py --py q` |
| typed Q | `python3 m10_test/run_m10.py --typed q`, `python3 m10_test/run_m10.py --typed --py q` |
| perf | `python3 m10_test/run_m10.py --typed perf`, `python3 m10_test/run_m10.py perf` |

To compare generated Rust, give each typed Q run its own `--out` and diff the two `gen.rs`. Both lanes use the same Q
cases and oracle. Flat Q also checks arena Grow. Typed perf measures putback on a prepared Budget; halt timing includes
preparing the fact answers, running the effects and the putback. The running-ids fact is staged as its wire words,
cloned per call as the hand lane clones its `Vec<String>`; Pruv (`bud_tlife::running_valid`) admits the wire and the
facts decode each admitted frame (`unit_text`), as in the Q run. Raw logs are in `work/p31t7/w4/` (not committed).

Language gaps: a Rust host entry cannot take an owned String parameter, and a record borrowed with `map_get_mut` cannot
be assigned as a whole through the dereference. Typed putback therefore refills the borrowed Budget field by field, keeping
equal String fields, when the incoming `stopped` list is empty, and inserts a copy otherwise. Repros: `work/p31t7/w4/repro_*.pruv`; see Appendix P31T (attempt 7) in
docs/L3_RESTRUCTURE_SPEC.md.
