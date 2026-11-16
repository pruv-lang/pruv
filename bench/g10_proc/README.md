# G10 proc benchmark

Quality gates for the `proc` effect and the owned `Process` handle (`docs/G10_PROC_SPEC.md` §8 Q). Every generated
file stays under `.work/` (not committed). Results are in `results/`.

| Command | What it does |
|---|---|
| `python3 run.py solutions` | grades the reference solutions on py/rs × rust/llvm against the fixture child `child.rs` |
| `python3 run.py solutions/indep1 results/indep1_first.json` | grades the independent author's solutions as received |
| `python3 mutate.py --compiler py` (and `rs`) | 16 explicit wrong mutants and 2 controls on both backends, classified by stage |
| `python3 perf.py --compiler py` (and `rs`) | 1 MiB over both outputs in 4 KiB chunks, Pruv against hand-written Rust |
| `python3 mutate_env.py --compiler py` (and `rs`) | G10-PROC-ENV: 7 wrong mutants and 1 control of `tests/runtime/g10p_r22_env.pruv` on both backends (2 static, 5 oracle) |
| `python3 faults/check.py` | six compiler faults, each applied alone to a copy of `pruvc/` (see `faults/README.md`) |

`PROBLEMS.md` has the two problems: q01, a `--version` run with exact argv, cwd, both outputs and the exit Result;
q02, streaming line statistics with a finite read budget while the child floods stderr.

Mutants are reported in two groups. Static mutants (missing consume, overwrite, branch, temporary, unchecked I/O,
write after close, range errors, a line counter that can exceed its bound) must be rejected by the checker or the
verifier. Oracle mutants (EOF misreading, read-all duplication, a lost line boundary) are programs that verify, because
what the child prints is not proved. The fixture oracle must catch them at run time; they show up as
`wrong_but_proved`. That is the expected class for them, not a soundness failure.
