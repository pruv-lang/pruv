# M12 typed lane

Sessions live in `HashMap<String, Budget>` and the per-instance run clocks in `ClockState`.
Lifecycle decisions go through `bud_tlife` and `bud_tstate`. Pruv uses the instance's
insertion order when forgetting a session or disposing of the instance.
The typed documents keep field order, duplicate keys and UTF-16 units. Auto-continue edits
their fields in place, and the V-word codec produces protocol replies and snapshots.
`maybeClose`, `raise` and `decide` are outside M12's operation list.

`run_m12.py --typed` reads the existing Q corpus and writes builds under `m12_test/target/p31t9/typed`.
`--typed --perf` uses the same lifecycle timing loop as the flat lane. Setup, cloning,
request decoding, observation and checksums run before or after timing. The call and
scheduler apply run inside it.
