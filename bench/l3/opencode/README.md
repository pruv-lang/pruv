# L3-7 OpenCode import (bench/l3/opencode)

Pruv port of Lair's OpenCode conversation import (`../grok/reference/{opencode,tail,plan,stubs}.ts`, pinned at Lair `744d14af`).
Spec: `docs/OPENCODE_IMPORT_SPEC.md`; design: `DESIGN.md`; Pruv ABI notes: `IMPL_NOTES.md`. Pruv makes every decision; the Rust host runs the
primitives and applies bank patches. This file covers the reference/oracle/Q/fixture side (package F) only.

**Status: nothing in this package has been run.** The files were written without a shell. Where results would go, this document says
"not yet run".

| Path | What |
|---|---|
| `reference.mjs reference-hooks.mjs reference-primitives.mjs` | run the pinned TypeScript exports on Node 26.4.0 with the same primitive schedule/trace language as the Rust host |
| `oracle.py` | exact comparison, with only the OpenCode policies P1-P13 (no Grok classifier is imported) |
| `q.py`, `q_mutants.py`, `Q_README.md` | seeded generator (`--cases 2000 --seed 20261117`) and the 16 source mutants |
| `fixtures/_defaults.json`, `fixtures/F01_paths.json` ... `F16_utilities.json` | recipes and named cases (DESIGN section 7) |
| `common.py test_impl.py fake_exporter.py host.rs host/ harness.rs str_adapter.rs` | package E (not owned here): fixture trees, build, host |
| `root.pruv packet.pruv util.pruv pruv.json` and the other `.pruv` modules | packages A-D |
| `work/` | generated: builds, case trees, logs, reports. Disposable |

## Commands (from the repository root)

```
export PATH="$HOME/.cargo/bin:$PATH" GROK_TS_TRANSPILER=/path/to/typescript   # typescript package: only used to transpile plan.ts
python3 bench/l3/opencode/test_impl.py --list                                   # fixtures and the mutants they are mapped to
python3 bench/l3/opencode/test_impl.py --reference-only --only F03 --only F07 --only F14   # package F's local check: TS runs + `expected` blocks
python3 bench/l3/opencode/test_impl.py --pairs rs/rust --only F01                # one pair, some groups
python3 bench/l3/opencode/test_impl.py [--slow]                                  # all four pairs, all fixtures (--slow adds the real 20 s timeout)
python3 bench/l3/opencode/q.py --cases 2000 --seed 20261117
python3 bench/l3/opencode/q_mutants.py --check                                   # markers present exactly once? (no build)
python3 bench/l3/opencode/q_mutants.py                                           # M01-M16 on rs/rust
```

Results: not yet run.

## The TypeScript reference

`node --import ./reference-hooks.mjs reference.mjs` reads one JSON config per stdin line (the line the Rust host reads) and prints
`{"meta":{node,v8,sqlite}}` followed by one envelope per case line; the trace is JSONL on stderr.

* `reference-hooks.mjs` resolves imports into `../grok/reference/` (never copied or edited) and redirects only: opencode.ts's
  `node:fs/promises`, `node:os`, `node:sqlite`; tail.ts's `node:fs/promises`, `node:fs`; stubs.ts's `node:child_process`. Only `plan.ts` is
  transpiled, with `GROK_TS_TRANSPILER` (explicit; no built-in default path).
* `reference-primitives.mjs` wraps the real functions. SQLite: the single real execution reads INTEGER as bigint so the storage class is
  observable, then reproduces Node's default safe-range check and Number conversion; no second query. Child: the pinned `spawnEngine` stub
  runs a real `child_process.spawn` with the real output-file fd; its events and the timers of the active `waitExport` call are recorded as
  `clock_now` + `observe(deadline)` without replacing `waitExport`. `Date.now`, `setTimeout`, `clearTimeout`, `process.kill` are wrapped only
  during one serial case and restored in `finally` together with `IMPORT_LIMITS.rpcMs`.
* Schedule: the same matcher as `host.rs` (op, phase, sql, path with `@N` for the Nth mkdtemp suffix, occurrence per (op, phase, path-or-SQL),
  before-call / after-result-before-delivery barriers, actions, first error stops). `hold` and `return` are rejected (HarnessError).
* Envelope: ok `{status, mode, rows | turns+omittedBefore+title?+model?+lastPlan? | value | present+value?}`; error
  `{status:"error", mode, error:{class, code, name, detail, stage, rawCode, native}}`, the keys the host's `render_result` prints.

## Fixtures

`_defaults.json` supplies only omitted top-level fields: a `read` case on a one-session SQLite image (`S0000`, `/work`, title `DB Title`),
the fake exporter as `$BIN/exporter` (cwd expectation `$SRC`), `$TMP` mode 0700, frozen `nowMs`. A case that gives `db`, `export`, `env` or
`tree` replaces that block (env fills its two keys individually), so export-bearing cases repeat `"expect":{"cwd":"$SRC"}`.
Conveniences expanded by `test_impl.py`: `{"json": V, "size": N}` export bytes (padded to exactly N bytes), `{"$raw": TEXT}` verbatim JSON
tokens (`1e400`, escaped surrogates), `{"$rep"}`/`{"$cat"}` string macros, `kills`, `policies`, `expected`, `slow`, `serial`, `disabled`.

| File | Group | Names (DESIGN section 7) |
|---|---|---|
| `F01_paths` | XDG, DB path | `F01-xdg-*`, `F01-db-*` |
| `F02_admission` | DB admission | `F02-db-*`, `F02-{parent-realpath,lstat,file-realpath}-error` |
| `F03_schema` | SQLite/schema | `F03-*` |
| `F04_selection` | selection, limit, order | `F04-*` |
| `F05_metadata` | titles, counts, P9, times, ages | `F05-*` |
| `F06_read_admit` | read admission | `F06-*` |
| `F07_spawn` | argv/cwd/env/stdio, large stdout file | `F07-*` |
| `F08_lifecycle` | child lifecycle, file errors | `F08-*` (plus `F08-output-close-error`; `F08-real-20000ms` is `slow`) |
| `F09_parser` | size/parser boundaries | `F09-*` |
| `F10_shape` | shape | `F10-*` |
| `F11_attachment` | text/attachment | `F11-*` |
| `F12_metadata` | model/title/time, P4 | `F12-*` |
| `F13_tools` | tools | `F13-tools-*` |
| `F14_plans` | plans | `F14-*` |
| `F15_collector` | collector limits | `F15-*` |
| `F16_utilities` | data dir, folder key, rows, direct plan | `F16-*` |

Disabled until package E supplies the facility (reason inside each case): `F08-error-after-timeout-script` (scripted child events),
`F09-grow-after-stat` (`@N` in schedule **action** paths; the reference shim already resolves it).

## Policies (oracle.py)

Deviations, each only at its trigger and with its stated effect: P1 (invalid UTF-8 / depth > 128 / tape > 1,048,576 -> Pruv read-failed json),
P2, P3, P4 (TS rerun with only unsupported export timestamp tokens edited, `oracle.baseline_cases`), P5 (exact `InputTooLarge`), P8, P9
(updatedMs/updatedAt/maybeOpen recomputed independently from the SQL-selected rows recorded in the TS trace). P6, P7, P10-P13 are equal
policies: no relaxation. Reports separate explained deviations, unexplained failures and excluded-domain observations.
Narrowings made in the oracle and nowhere else: `stage` of `clock_now`/`observe` records, the env variable `_`, and the content hash of `-shm` files.

## Not yet run / to reconcile

See `Q_README.md` ("Assumptions and open points").
