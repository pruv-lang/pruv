# M10b typed resident state

The M10b runner generates the Rust ABI from `root_m10b_typed.pruv`.
The driver keeps one `St` per world and passes it by mutable reference. The
session map holds budgets with numeric limits and counters, owned text fields,
and the halt's ordered `Vec<String>`. The clock map holds the clock owner,
interval, timer flag, active-run count, and a `HashMap<String, bool>` of waiting
flags. The clock ordinal and run insertion history preserve the order of the
TypeScript observations, including removal and reinsertion.

Pruv updates the records through borrowed text keys, removes closed clocks
with `map_remove`, and returns a `Reply` with a `Vec<Effect>`. Rust performs the
effects, schedules timer callbacks, and converts the Q wire format at ingress
and egress. The text codec preserves isolated UTF-16 surrogates using escaped
Unicode scalars so keys remain lossless inside Rust strings.

The synchronous halt path settles the clock, records the effect position, and
calls the host's `running_count` once. It takes the IDs in order via
`running_id`, whose owned `String` result is allowed under R-6d-1. The older
packed host fact still serves the other lanes. Unit responses remain
`Vec<i64>`, also supported under R-6d-1. UUID is outside the M10b entry set.

Run `run_m10b.py derive` to stage the Q cases and oracle locally, or pass
`--cases` and `--oracle` to reuse existing inputs. The runner keeps generated
Rust, proof caches and binaries in `.m10b-work/`. The H and G controls still
check rejected inputs for effects and state changes. The old packed arena's
Grow injection has no counterpart in the typed state.
