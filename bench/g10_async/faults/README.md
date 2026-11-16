# G10 async Q compiler faults

Run from the repository root:

```sh
/Users/spica4345/Projects/pruv/.venv/bin/python bench/g10_async/faults/check.py --compiler both --backend both
```

Both selectors also accept `both`; the backend selector accepts `llvm`. `--generate-only` regenerates the seven checked-in patches against
the current sources, without applying them. Normal runs require those patches to match. `--only NAME` (repeatable) runs a subset.

Each diff contains the equivalent Python and Rust compiler edit. Each is applied alone
with `patch -p1` to a disposable compiler source copy under `bench/g10_async/.work`.
The working compiler sources and tests are never edited. Rust builds use an offline,
shared build cache under `.work/fault-target`; the first unseeded build can be cold.
The clean and faulty Rust compilers are rebuilt from their respective source copies.
Unselected compiler/backend rows from an existing result file are retained, allowing
an interrupted compiler batch to be resumed without repeating completed probes.

| Fault | Named probe | Clean oracle | Faulty oracle |
|---|---|---|---|
| `f1_must_consume.diff` | `missing_close.pruv` | `type/async_unconsumed` | accepts forgotten Loop at return |
| `f2_read_kind.diff` | `read_listener.pruv` | `verify/call_requires` | accepts socket read on listener slot |
| `f3_remove_frame.diff` | `remove_frame.pruv` | verifies count decrement and other-slot kind frame | fails count postcondition |
| `f4_close_tag.diff` | `close_tag.pruv` | empty-loop close returns `Ok(0)` at runtime | inverted tag fails runtime assertion/contract |
| `f5_wait_for_blocks.diff` | `tests/runtime/g10a_r23_wait_for` | driver exits 0 with the fixture child | `ev_proc_wait_for` ignores its deadline; a 20 ms wait frees the exited child and the driver asserts |
| `f6_write_for_fails_slot.diff` | `tests/runtime/g10a_r24_write_for` | driver exits 0 with the fixture child | a write deadline marks the slot failed instead of half-closing stdin; the driver asserts |
| `f7_wait_for_frame.diff` | `tests/g10_async/proof/ev_proc_wait_for.pruv` | verifies | the kept branch of the encoding drops `ev_exit` preservation; the postcondition fails |

The `proc` probes (f5, f6) compile the tests/runtime program with `--no-verify`, build its driver and run it
with `PRUV_PROC_CHILD` set to the fixture child built from `tests/g10_proc/child.rs`. The fourth fault changes the runtime Result constructor shared by Rust source generation
and the LLVM Rust shim, rather than changing the verifier's abstract contract. It requires
no network permission. A timeout, failure to build/start, or internal diagnostic never
counts as a caught fault. Full clean/faulty diagnostics and patch results are written to
`../results/faults.json`; every compiler/backend/fault pair gets its own row.

## Recorded run (2026-11-17)

All seven faults are caught on both compilers and both backends with clean baselines:
28/28 rows `caught` in `../results/faults.json` (f1-f7 × py/rs × rust/llvm).
