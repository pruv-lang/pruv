# M10b typed resident state

The M10b runner generates the Rust ABI from `root_m10b_typed.pruv`.
The driver keeps one `St` per world and passes it by mutable reference. The
session map holds budgets with numeric limits and counters, owned text fields,
and the halt's ordered `Vec<String>`. The clock map holds the clock owner,
interval, timer flag, active-run count, and a `HashMap<String, i64>` of runs.
Each run value is its insertion ordinal, negated while the run is waiting, so
the clock ordinal and the run ordinals preserve the order of the TypeScript
observations, including removal and reinsertion, without a separate order list.

Pruv updates the records through borrowed text keys, removes closed clocks
with `map_remove`, and returns a scalar `Reply`. Rust reads effects from
`St.out.fx`, schedules timer callbacks, and converts the Q wire format at
ingress and egress. The text codec preserves isolated UTF-16 surrogates using
escaped Unicode scalars so keys remain lossless inside Rust strings.

The synchronous halt path settles the clock, records the effect position, and
calls the host's `running_count` once. It takes all IDs in order via one
`running_take` call, whose owned `Vec<String>` result is allowed under R-6d-1,
and stores them in the budget's reused `stopped` vector (`vec_clear` on reset).
Text comparisons use `string_eq`. The older
packed host fact still serves the other lanes. Unit responses remain
`Vec<i64>`, also supported under R-6d-1. UUID is outside the M10b entry set.

Run `run_m10b.py derive` to stage the Q cases and oracle locally, or pass
`--cases` and `--oracle` to reuse existing inputs. The runner keeps generated
Rust, proof caches and binaries in `.m10b-work/`. The H and G controls still
check rejected inputs for effects and state changes. The old packed arena's
Grow injection has no counterpart in the typed state.

`t_call` clears the resident effect vector with `vec_clear` before dispatch.
R-6g-3 lets helpers borrow `inout st.out` alongside a live session or clock
borrow. This reuses the buffer's capacity across calls (D25, D28, D29).
New runs and clocks return status 2 when insertion would advance their
ordinal counter to 2^40. Updates to existing runs remain available at the cap.
