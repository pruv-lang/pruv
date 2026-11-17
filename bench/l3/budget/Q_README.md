# L3-10 Q corpus and fixtures: commands

Environment: Node v26.4.0, Python 3 (no third-party packages), and the pinned TypeScript transpiler:

    export GROK_TS_TRANSPILER=/Users/spica4345/Projects/lair/node_modules/typescript   # common.TS_ENV() defaults to this
    # BUDGET_NODE=/path/to/node overrides the node binary (default: node on PATH, else /opt/homebrew/bin/node)

Run from `bench/l3/budget/`:

    python3 gen_fixtures.py                 # writes fixtures/A.json .. N.json (140 fixtures), deterministic
    python3 q_gen.py                        # 8 families x 250 -> work/q/cases.jsonl (seed 20261117)
    python3 q_gen.py --seed 1 --families Q1,Q3 --per-family 50 --out /tmp/cases.jsonl
    python3 q_gen.py --count 300            # 300 cases total, round-robin over the shuffled families
    python3 oracle.py --smoke 300           # M0: deterministic oracle seed scenarios (written by the oracle worker)
    python3 check_oracle.py                 # M2: runs the 140 fixtures on the oracle and checks every `expect`
    python3 q.py [--impl CMD]               # M2: oracle twice over cases.jsonl (determinism), optional implementation comparison
    python3 test_impl.py                    # M2: runs the implementation pairs on fixtures + Q
    python3 perf_budget.py                  # M2: axes of spec section 9 using q_gen.perf_case(axis, n)

`check_oracle.py`, `q.py`, `test_impl.py` and `perf_budget.py` come in a later milestone; this milestone delivers the corpus
(`common.py`, `gen_fixtures.py`, `q_gen.py`, `SCENARIO.md`, this file).

## Generator
xorshift32 (state never 0) with rejection-sampled `below(n)`; seed 20261117, recorded in every case. The family list is Fisher-Yates shuffled
with the seed. Each case has its own stream derived from `(seed, family number, ordinal)`, so `--families Q1,Q3` reproduces the same Q1/Q3
cases as a full run. Case ids are `Q<f>-<seed>-<n>` (n is the 1-based ordinal inside the family). Scripts have 4-20 steps.

| Family | Axes |
|---|---|
| Q1 | opening, replacement, reopen, stale references, origins, sizes |
| Q2 | charges, atomicity, tools, failures, retries |
| Q3 | multi-run waiting/running transitions and timer delivery |
| Q4 | halt, quiet close, delegation order, nested halts |
| Q5 | raise/decide combinations, retained runs, queue/release order |
| Q6 | card validation, UTF-16, optional fields, Number classification |
| Q7 | auto-continuation ownership, restoration, cleanup, multiple instances |
| Q8 | text/helpers, admitted compatibility values, larger inputs |

Boundaries the generator leans on: charges equal to the remaining limit and one above, zero-ms advances, 300/301 UTF-16 units (including astral and lone
surrogates), list lengths 0/1/5/6, missing versus `{undef:1}`, raw `{num}` card counts (fractions, -0, NaN, ±Infinity, 2^53, 1e300).
A small model (`Sim` in q_gen.py) tracks sessions, budget states, clocks and timers so that references stay valid (and stale ones are retained on purpose)
and so that Q3's timer tokens are exact. The model only chooses arguments; a mismatch with the oracle never invalidates a case.

## Python API (imported by q.py and the perf scripts)
- `q_gen.gen(seed, families, per_family, count=None)` -> list of cases.
- `q_gen.perf_case(axis, n)` -> one scenario; axes `sessions` (S sessions, one fixed session addressed), `calls` (n charge + n noteTool),
  `runs` (n runStarted then one refresh), `delegations` (halt with n running ids), `clocks` (n clocks, then forgetSession and dispose).
- `common`: `HERE FIXTURES WORK NODE GROUPS IGNORED_KEYS strip_case load_group load_all TS_ENV`.

## Not generated (domain rules)
Dependency throws, per-call `now` overrides, spec 15.1 rows 10 (owner mismatch in settle), 16 (halt without reason driving continuation),
26 (throwing deps) and 27 (halted budget without `halt` charged). Format and expectation DSL: `SCENARIO.md`.
