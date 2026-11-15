# G10 net benchmark

Quality gates for the `net` effect and the owned `Socket` and `Listener` handles (`docs/G10_NET_SPEC.md` §8 Q). Every
generated file stays under `.work/` (not committed). Results are in `results/`.

| Command | What it does |
|---|---|
| `python3 run.py solutions` | grades the reference solutions on py/rs × rust/llvm against Rust drivers that host the peers in threads on 127.0.0.1 |
| `python3 run.py solutions/indep1 results/indep1_first.json` | grades the independent author's solutions as received |
| `python3 mutate.py --compiler py` (and `rs`) | 16 explicit wrong mutants and 2 controls on both backends, classified by stage |
| `python3 perf.py --compiler py` (and `rs`) | 1 MiB over loopback in 4 KiB chunks, Pruv against a hand-written Rust client |
| `python3 faults/check.py` | four compiler faults, each applied alone to a copy of `pruvc/` (see `faults/README.md`) |

`PROBLEMS.md` has the two problems: q01, a length-prefixed request/response client with exact framing, split reads and
every failure path closing the socket; q02, a bounded line server (listen on port 0, announce the port on a control
connection, serve a budget of connections, byte-based line assembly).

Mutants are reported in two groups. Static mutants (missing close, overwrite, branch, temporary, unchecked I/O, write
after shutdown, range errors, a reply buffer one byte short) must be rejected by the checker or the verifier. Oracle
mutants (two EOF misreadings, `net_read_all` duplication, a misread frame length, a wrong line terminator) are programs
that verify, because what the peer sends is not proved. The driver oracle must catch them at run time; they show up as
`wrong_but_proved`. That is the expected class for them, not a soundness failure.

What the peers send is checked by the driver at run time, not proved. Every peer thread has 30 s timeouts, the driver
only waits with bounds, and a watchdog ends the driver after 90 s; the grader kills it after 120 s.
