# L3-9 Q: 2,000 random scenarios

Run (oracle only, about 4 s):

    export PATH="$HOME/.cargo/bin:$PATH" GROK_TS_TRANSPILER=/Users/spica4345/Projects/lair/node_modules/typescript
    python3 q_gen.py          # writes work/q/cases.jsonl (seed 20261119, deterministic)
    python3 q.py              # oracle twice, checks, work/q/oracle.jsonl, work/q/report.json
    python3 q.py --impl "CMD" # also compare an implementation pair (see below)

## Generator
xorshift32 with rejection-sampled uniform, seed 20261119. The family multiset (Q1 200, Q2 200, Q3 300, Q4 300, Q5 250, Q6 250,
Q7 250, Q8 250) is Fisher-Yates shuffled, then cases are generated in that order from one random stream. Case ids are
`Q<n>-<ordinal in family, 4 digits>`. Each line of cases.jsonl is a SCENARIO.md scenario plus `family`, `tags`, and optionally `tieBreak`
and `corrupt`. The oracle ignores the extra keys.

| Family | Axes |
|---|---|
| Q1 | sources/projection: accounts, duplicate ids, env, missing or malformed source, label |
| Q2 | preflight: folder errors, mode, fromAppAction, Grok account, initially bound sessions |
| Q3 | list: duplicate session ids, titles, temporary, metadata, per-item snapshot (mutation during a deferred list) |
| Q4 | open data: turns, time, UUIDs, slots, Unicode, title cut at 80 units, plan |
| Q5 | Grok stages: target, copy, verify, create, rollback (rm), race, cancel during copy/verify |
| Q6 | errors and cancel: class/name/code combos, reason identity, 20,000 ms deadline, cancel during cleanup |
| Q7 | concurrency: 2-4 requests on one instance, same or other key, session mutation, retry wave |
| Q8 | protocol/domain: honest scenarios tagged with a `corrupt` descriptor, limits, excluded values |

Strings (titles, prompts, thread ids, messages) mix lone surrogates (`{u16:[...]}`), U+0085, U+200B, astral characters and combining marks.

## tieBreak
`"tieBreak": true` (Q7 only, at most 12 of 250) marks a case where two requests' timers fire at one instant (a single `advance`
to 20000 ms after starts without gaps). Ordering of the two requests is then decided by the scheduler's tie break, not by the
handler, so implementations may need the same tie rule. All other Q7 cases space starts with `advance 1` and settle one label at a time.

## corrupt (Q8)
`{"kind": completion|limit|excluded, "at": ..., "field": ..., "nth": n}` describes the corruption a protocol-corruption driver
should apply to the n-th matching dependency completion. The oracle runs the honest scenario, so the descriptor is data only.

## q.py checks
Per case: no driver violations, every started request has a terminal (no `pending`, unless tagged `pending-ok`), and the second
oracle run is identical. It prints per-family counts and terminal histograms. Exit code 1 on any problem or difference.

## Implementation hook
`--impl CMD` runs CMD with cases.jsonl on stdin and expects one observation JSON per line in the same order and shape as
the oracle. `compare(oracle_obs, impl_obs)` in q.py returns None when equal. Without `--impl` the run is oracle-only.

## Failure artifact
work/q/fail/<n>.json: `{seed, case, family, input (world), script, oracle, impl, firstTraceDifference {index, oracle, impl}, diff}`.
