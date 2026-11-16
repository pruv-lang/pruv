# L3-3 Q

From the repository root:

```sh
export PATH="$HOME/.cargo/bin:$PATH"
export MCP_NODE=/opt/homebrew/bin/node
python3 bench/l3/mcp/test_impl.py
python3 bench/l3/mcp/q.py
python3 bench/l3/mcp/q_mutants.py
```

The differential requires the four verified runners from `test_impl.py`.
Default corpus: 22 original fixtures, 2,000 seeded cases (seeds 0..1999), and
18 targeted boundary cases. `q.py N` replaces the seeded case count for diagnosis.
`q_server.py` extends fixture behaviours in a new file, keeping the existing
implementation fixtures untouched. Random tool lists, pagination, annotations,
invalid names, malformed/odd lines, notifications, wrong IDs, same-ID server
requests, old-protocol retry, RPC errors and stderr exits are covered. Targeted
cases cover invalid UTF-8, decimal rounding, multibyte line caps, signals,
unterminated partial lines and short deadlines. Servers use only stdio, have no
children, and exit on EOF or are killed/reaped by the client being tested.

Generated requests, full reference/actual output, class counts and wall times are
in `work/`. `oracle.py` checks exact results first and permits only explicit,
result-checked policy transformations. An unexplained mismatch makes Q fail.
The description-cut exception removes exactly one trailing high surrogate in
the fixed `cut` case; fingerprints and every other field must remain identical.
The mutation driver copies all Pruv modules to `work/mutants/<name>`, makes one
replacement, verifies/builds rs/rust and compares the 22 fixtures with the pinned
reference. Build/tool errors are not credited as verifier kills. Run Q before
mutants to establish `work/reference.json`.

No throughput or latency benchmark is performed. Reported pair wall time includes
all child processes, harness startup and Node fingerprint work. The blocking
synchronous-write limitation, native 30-second initialization deadline and
malicious descendants are not exercised. No whole-program equivalence is claimed.
