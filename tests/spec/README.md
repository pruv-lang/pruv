# tests/spec: rule-traced tests

Each test here names the SPEC.md rule IDs (`R-<section>-<n>`) it exercises. `tests/spec_trace.py` runs every test
with both compilers (Python `pruvc/pruvc.py` and Rust `pruvc-rs`) and fails if a SPEC rule ID lacks a passing accept
or reject test, if a test names an ID that SPEC.md does not define, or if a diagnostic code is missing from the SPEC.md
code table. Run it with `tests/run_all.py --only spec_trace` or directly:

    .venv/bin/python tests/spec_trace.py            # both compilers
    .venv/bin/python tests/spec_trace.py --only-py  # one compiler

## Format

A test is `NAME.pruv` or a project directory `NAME/root.pruv`. Its first line is

    // spec: R-x-n [R-y-m ...] accept|reject

A `.pruv` file without that header (a project's other modules, a cache prime file) is not a test.
An optional `NAME.json` (`NAME/root.json`) next to the source sets the expectation:

| key | meaning |
|---|---|
| `flags` | extra compiler flags (list) |
| `backend` | `"rust"` (default), `"llvm"`, or `"none"` (no `--backend` flag, for native `--emit`) |
| `stage`, `code` | reject: some emitted diagnostic has this stage and code (exit 1); required for a reject test that exits 1 |
| `exit` | expected exit code (default 0 for accept, 1 for reject) |
| `gen_contains`, `gen_absent` | strings that must / must not appear in the generated output |
| `gen_regex`, `gen_regex_absent` | regular expressions (multiline) that must / must not match the generated output |
| `out_contains` | strings that must appear in stdout+stderr |
| `cache` | `{"prime": "OTHER.pruv", "hits": "positive"\|"zero"}`: compile `prime` (default the test itself) into a fresh `--proof-cache`, then compile the test with that cache and `--trust-report`; `proof_cache.hits` must be > 0 or == 0 |
| `py`, `rs` | objects merged over the keys above for that compiler only; a `null` value drops the key (for a driver-only rule) |

A flat test can also include a `run` object with a runtime expectation (see `tests/run_all.py`) and a
`NAME.rs` driver that calls the generated module as `sol`. The runtime harness runs it on both backends by
default. Use `--only runtime --sample-prefix tests/spec/` to select these rows, or
`--only runtime --sample-prefix tests/spec/r6g4_ --backend both` for the buffer growth fixtures.

An accept test of a code-generation policy (stack arrays, small result buffers, inlining) checks the generated code;
its reject test is a program just outside the policy, whose generated code must lack the optimized form. The policy
is not a diagnostic: such a reject test exits 0.
