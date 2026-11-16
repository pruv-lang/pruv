# G10 async Q

Gates in `docs/G10_ASYNC_SPEC.md` §8 Q. Problems and protocol choices are in `PROBLEMS.md`. Reference
programs are `solutions/q01.pruv` (a multi-client line server with a per-client timeout) and
`solutions/q02.pruv` (a supervisor that streams a child's output, with cancel and a deadline). `run.py`
drives loopback clients from a Rust host (127.0.0.1, ephemeral ports, every wait bounded); `mutate.py`
applies 16 named wrong edits and two valid controls. Fixtures under `fixtures/` are local Python children;
they write more than 1 MiB of stderr, so a program that never drains it does not see the child finish.
Fault injections are in `faults/` (see `faults/README.md`), the measurement in `perf.py` and `perf/`
(see `perf/README.md`).

From the repository root:

```sh
export PRUV_PYTHON=$PWD/.venv/bin/python
export PATH="$HOME/.cargo/bin:$PATH"
(cd pruvc-rs && cargo build --release)
$PRUV_PYTHON bench/g10_async/run.py --compiler both --backend both
$PRUV_PYTHON bench/g10_async/mutate.py --self-test
$PRUV_PYTHON bench/g10_async/mutate.py --compiler both --backend both
$PRUV_PYTHON bench/g10_async/faults/check.py
$PRUV_PYTHON bench/g10_async/perf.py --compiler both --repeat 3
```

All scripts take `--compiler py|rs|both`; `run.py` and `mutate.py` also take `--backend rust|llvm|both`.
`run.py SOLUTIONS_DIR OUT.json` grades another set of solutions (used for the independent author).
`PRUV_PYTHON`, `PRUVC_RS`, `RUSTC` and `CLANG` select the tools. Generated files stay under `.work/`.
Compiler rejections and proved-but-wrong oracle results are reported separately; unknowns, crashes,
timeouts and unsupported backends are unresolved, never catches. A loopback permission failure is
reported as `bind_permission`.

## Recorded run (2026-11-16, macOS, all four py/rs × rust/llvm pairs)

- Solutions: q01 and q02 `ok` (3/3 each) on every pair (`results/solutions_*.json`).
- Mutants: self-test passes. Every pair gives the same split: 11 static (m001-m004 `type/async_unconsumed`,
  m005-m010 and m012 `verify/call_requires`), 5 `wrong_but_proved` caught by the oracle (m011, m013-m016),
  2 controls accepted, `unexpected: []` (`results/mutants_*.json`). m006 and m008 route the edited operation
  through a small helper so the verifier gives a concrete precondition counterexample.
- Faults: f1-f4 caught on both compilers and both backends, 16/16 (`results/faults.json`).
- Measurement: `results/perf_{py,rs}.json`, summarized in RESULTS.md.
