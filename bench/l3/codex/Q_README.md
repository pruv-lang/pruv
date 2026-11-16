# Codex import Q (differential and mutants)

Run `test_impl.py` first: it builds `work/host`, `work/libpruv_str.rlib` and the four runners
`work/{py,rs}_{rust,llvm}/runner` that `q.py` and `q_mutants.py` reuse.

## q.py

```
python3 bench/l3/codex/q.py [N]
```

Cases: fixed probes (policy probes `probe-P1`, `probe-P2`, `probe-P3`, `probe-P4`, `probe-P6`, `probe-P8` and `edge:*`
framing cases), `N` (default 2000) seeded random scenarios with seed 20261116 (list and read, random items / turns /
timestamps / cursors / plans / errors / noise / server requests / exits / stalls), and three heavy cases (turn budget,
byte budget, 40-page cap). Every case runs on the pinned TypeScript (Node, `reference.mjs`) and on the four pairs against
the same fake app-server (`q_server.py`), each side with its own request log.

A case passes when the result equals the TS result and the request transcript (every line the client sent, in order)
equals the TS transcript, with no protocol violation recorded by the fake. A result difference is accepted only through
`oracle.classify`, which names the spec's policies and nothing else:

| Class | Policy | What differs |
|---|---|---|
| `P1-invalid-utf8` | P1 | an invalid UTF-8 line is skipped by Pruv, TS decodes it with U+FFFD and answers (Pruv times out) |
| `P2-surrogate` | P2 | a lone surrogate in a value is U+FFFD on the Pruv side |
| `P3-trailing-fffd` | P3 | TS strips a genuine trailing U+FFFD after a byte clip, Pruv keeps it |
| `P4-timestamp` | P4 | Pruv reads only integer and canonical-ISO timestamps (field absent, never different) |
| `P6-id-token` | P6 | a response id counts only when its token is exactly the decimal id (Pruv times out) |
| `P8-plan-cut` | P8 | the 200-unit plan slice drops a split surrogate pair |

Anything else is `unexplained` and fails the run (nonzero exit). Reports: `work/q-report.json`
(per pair: class counts, wall seconds, first unexplained cases), `work/q_cases.json`, `work/q_reference.json`,
`work/<pair>_q.json`.

Timing: stall and timeout cases use 300-400 ms deadlines on both sides and the fake never answers them, so no case races
a response. Only the total wall time per pair is recorded.

Results (2026-11-16, 2000 random + 3 large-budget + 10 policy probes + 10 edge probes = 2023 cases, identical on all four pairs):

| Class | Cases |
|---|---|
| exact | 1633 |
| P4-timestamp | 382 |
| P2-surrogate | 2 |
| P3-trailing-fffd | 2 |
| P6-id-token | 2 |
| P1-invalid-utf8 | 1 |
| P8-plan-cut | 1 |
| unexplained | 0 |

Wall time for the whole set: TypeScript reference 55.18 s, py/rust 57.31 s, py/llvm 57.33 s, rs/rust 56.88 s,
rs/llvm 56.90 s (dominated by the 300-400 ms deadline cases, which run the same on both sides).

## q_mutants.py

```
python3 bench/l3/codex/q_mutants.py
```

Sixteen hand mutants, single source replacements in `work/mutants/<name>/` (copies of the codex `.pruv` files, manifest
paths rewritten), compiled with the Rust compiler and Rust backend only (`rs/rust`):

| # | Mutant | File |
|---|---|---|
| 1 | wrong response id accepted | `rpc.pruv` |
| 2 | `initialized` notification omitted | `rpc.pruv` |
| 3 | `-32601` mapping to codex-unsupported lost | `root.pruv` |
| 4 | one not-found alternative (`no rollout`) dropped | `rpc.pruv` |
| 5 | list id admission (`validThread`) bypassed | `list.pruv` |
| 6 | paging stops after the first page | `root.pruv` |
| 7 | held turns fed newest first | `read.pruv` |
| 8 | commentary admitted as a reply | `item.pruv` |
| 9 | plan item counted as a tool | `item.pruv` |
| 10 | generic item not counted as a tool | `item.pruv` |
| 11 | budget omission (`omittedBefore` on a full budget) removed | `root.pruv` |
| 12 | reply bytes capped at 32767 instead of 32768 in the budget | `item.pruv` |
| 13 | a later plan replaces the first | `read.pruv` |
| 14 | a `cwd` with a lone surrogate accepted (U+FFFD matches) | `root.pruv` |
| 15 | `error.code` binary64 rounding lost (exact decimal only) | `rpc.pruv` |
| 16 | a prefix of the lower rounding bound accepted | `rpc.pruv` |

A mutant is killed by a verifier rejection (`verifier-rejected`) or by at least one fixed fixture of
`fixtures/cases.json` whose result differs from the checked-in expected JSON or whose requests violate the protocol
(`differential-killed`). A tool/parser error is reported as `build-error` and is not credited as a kill; `survived`
is not a claim of equivalence. Report: `work/mutants/report.json`.

Results (2026-11-16): **16 differential-killed, 0 verifier-rejected, 0 survived, 0 build errors.**

| # | Mutant | Kill mode | Killing fixtures | Example |
|---|---|---|---:|---|
| 1 | wrong-id-accepted | differential-killed | 3 | `list_noise_ignored` |
| 2 | initialized-omitted | differential-killed | 20 | `list_basic` |
| 3 | unsupported-mapping-lost | differential-killed | 8 | `read_unsupported` |
| 4 | not-found-alternative-dropped | differential-killed | 2 | `read_no_rollout` |
| 5 | list-admission-bypassed | differential-killed | 2 | `list_50_raw_then_filter` |
| 6 | stop-after-first-page | differential-killed | 9 | `read_paging` |
| 7 | newest-first-feeding | differential-killed | 10 | `read_pages_40` |
| 8 | commentary-admitted | differential-killed | 2 | `read_commentary` |
| 9 | plan-counted-as-tool | differential-killed | 6 | `read_tools` |
| 10 | generic-tool-not-counted | differential-killed | 2 | `read_tools` |
| 11 | budget-omission-removed | differential-killed | 5 | `read_budget_turns` |
| 12 | reply-bytes-cap-off-by-one | differential-killed | 1 | `read_budget_reply_cap_scalar_clip` |
| 13 | later-plan-replaces-first | differential-killed | 1 | `read_plan_newest_wins` |
| 14 | lone-cwd-accepted | differential-killed | 2 | `read_cwd_lone_surrogate` |
| 15 | code-rounding-lost | differential-killed | 2 | `read_unsupported_rounds` |
| 16 | code-low-prefix-accepted | differential-killed | 1 | `read_code_below_low` |

Mutant 12 first survived the one-byte `read_budget_reply_cap_boundary` (the TS collector's own 2 MiB pass trims the
extra turn again); `read_budget_reply_cap_scalar_clip` (3-byte scalars, 32766-byte clipped replies) kills it.
