# Resident-state performance harness

Measures the claims of `docs/RESIDENT_STATE_SPEC.md` (§6, §8): a host that keeps `Vec<i64>` buffers and passes
`&mut [i64]` to a Pruv host entry pays per call only for what the call touches, and the fixed cost of one boundary
call is in the Node range.

## Files

- `state.pruv` — explicit-mode program. State = 4-word header + S records of 4 words. Entries: `init` (once),
  `step` (resident, in place, O(1)), `noop` (empty call), `step_copy` (pre-batch shape: read-only state in, fresh
  O(S) copy out). `step` validates its input and the header in executable code, so it is also safe under
  `--trust-callers`.
- `driver.rs` — Rust host. For S = 2000 and 20000 it allocates, calls `init`, warms up, then times 10,000 calls each
  of `step`, `noop` and `step_copy` (the host replaces its `Vec` with the returned copy). Prints
  `backend=<b> S=<s> kind=<step|noop|copy> calls=10000 ns_per_call=<x>`.
- `run.py` — builds and runs everything, prints the table and the verdicts.

## Run

```
cd pruvc-rs && cargo build --release --locked      # once
python3 tests/resident/perf/run.py [--pruvc PATH] [--runs 3] [--out FILE.md] [--no-checked] [--keep]
```

Needs `rustc`, `clang` (LLVM backend objects and `pruvc/runtime.c`) and the compiler binary
(default `pruvc-rs/target/release/pruvc`). Builds: Rust source backend and LLVM backend (`--rust-bindings`), each
with `--trust-callers`, plus checked builds (no `--trust-callers`) unless `--no-checked`. The driver is built with
`rustc -C opt-level=3` in the same way as `tests/run_all.py` (generated module via `#[path]`, LLVM objects linked
with `-C link-arg`). Each driver runs N=3 times (`--runs`); the median per measurement is used.

## Gates

Applied to the `--trust-callers` builds (checked builds are shown, informational unless `--gate-checked`):

1. **Size independence** (both backends): `ns_per_call(step, S=20000) / ns_per_call(step, S=2000) < 2.0`.
2. **Fixed boundary cost** (**llvm build only**): `ns_per_call(noop) <= 2000` at both sizes (Node round trip is
   about 800 ns).

Why the boundary gate is llvm-only: the rust-backend build compiles the generated code and the driver in one crate,
and the generated wrappers are `#[inline]`, so rustc can inline and remove the empty call. Its `noop` is therefore
not a real call boundary. It is still measured and printed, but its rows are labelled `integrated (inlinable)` in
the table and verdict lines, and its noop is not gated. The llvm build links the generated code as a separate object
(`gen.o`) into the driver, so each call is a real external call through the raw ABI wrapper (overlap check
included); that is the number the 2000 ns boundary gate judges. Exit codes and CLI options are unchanged.

`step_copy` is a contrast only; it is expected to grow roughly with S.

Exit code: 0 both gates pass, 1 a gate fails, 2 a tool is missing or a build/run step failed.
Results are recorded in `RESULTS.md` (written by the lead, not by this harness).
