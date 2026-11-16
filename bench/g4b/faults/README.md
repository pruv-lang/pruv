# G4b-Q fault injection (docs/G4B_SPEC.md section 8)

Seven mutants of the package resolver and the proof cache, each applied alone to a disposable copy of `pruvc/` (py) or
`pruvc-rs/` (rs), so 14 fault runs in total with both compilers. The source tree is never edited.
A fault is caught when its probe passes on the clean copy and fails on the faulty copy. A failed build or a probe
that times out is "not started": it is reported as an error and never counts as caught.

| fault | what is removed | probe |
|---|---|---|
| `cycle` | the dependency cycle check (`manifest.py` / `manifest.rs`) | `tests/g4b/check_pkg.py` |
| `identity` | the identity check (same package name from two places) | `check_pkg.py` |
| `version_line` | the version-line comparison (`found >= want` only) | `check_pkg.py` |
| `unused` | the unused-dependency check of a package root, and that of a legacy root | `check_pkg.py` |
| `unused_legacy` | only the unused-dependency check of a legacy root (no `mod`, no `pub`) | `check_pkg.py` |
| `store_sat` | the cache also stores `sat` answers | `tests/g4b/check_cache.py --case case_never_stored --case case_keys` |
| `drop_options` | the solver options are left out of the cache key | `check_cache.py` (same cases) |

Py copies are probed with `--py <copy>/pruvc/pruvc.py` (the trace tool is served from the same copy by a wrapper
inside `check_cache.py`). Rs copies are built one after another with `cargo build --release --bins` into one shared
`CARGO_TARGET_DIR=bench/g4b/.work/rs-target` and probed with `--rs <target>/release/pruvc`. The clean copy is built and
probed first (once per compiler).

## Run

```
python3 bench/g4b/faults/check.py                         # all faults, both compilers
python3 bench/g4b/faults/check.py --compiler py           # py only
python3 bench/g4b/faults/check.py --fault cycle --fault unused_legacy --compiler rs
python3 bench/g4b/faults/check.py --keep                  # keep the temporary copies
```

Output: a table (fault, compiler, baseline, faulty, caught, failing check names), the diffs
`bench/g4b/faults/<fault>.<py|rs>.diff`, and `bench/g4b/.work/faults.json`. Exit 0 only when every requested fault is
caught on every requested compiler.

Baseline of the test scripts by hand:

```
python3 tests/g4b/check_pkg.py WORK [--rs BIN] [--py PATH]
python3 tests/g4b/check_cache.py WORK [--rs BIN] [--py PATH] [--only py|rs] [--case NAME]...
```
