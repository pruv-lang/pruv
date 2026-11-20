# M12 typed lane (partial)

The lane keeps sessions in `HashMap<String, Budget>` from `bud_tstate`. Pruv assigns session ordinals
with the M10 cap. The V-word codec preserves field order, duplicate keys, UTF-16 units and numeric
payloads. Queries and setCard use the stored Budget. Auto-continue stages the referenced sessions
in a temporary Pruv arena for the shared `bud_auto` editor and commits edits back to the typed map.

`run_m12.py --typed` uses `work/p29/m12/typed` and runs the existing Q corpus. Typed
lifecycle and timer firing still need a resident clock map wired to `bud_tlife` and `bud_tdecide`.
The typed perf path covers the auto-continue rows with the existing timer boundaries.
Lifecycle perf remains unavailable.
