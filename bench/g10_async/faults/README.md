# G10 async Q compiler faults

Run from the repository root:

```sh
/Users/spica4345/Projects/pruv/.venv/bin/python bench/g10_async/faults/check.py --compiler both --backend both
```

Both selectors also accept `both`; the backend selector accepts `llvm`. `--generate-only` regenerates the four checked-in patches against
the current sources, without applying them. Normal runs require those patches to match.

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

The fourth fault changes the runtime Result constructor shared by Rust source generation
and the LLVM Rust shim, rather than changing the verifier's abstract contract. It requires
no network permission. A timeout, failure to build/start, or internal diagnostic never
counts as a caught fault. Full clean/faulty diagnostics and patch results are written to
`../results/faults.json`; every compiler/backend/fault pair gets its own row.

## Recorded run (2026-11-16)

All four faults are caught on both compilers and both backends with clean baselines:
16/16 rows `caught` in `../results/faults.json` (f1-f4 × py/rs × rust/llvm).
