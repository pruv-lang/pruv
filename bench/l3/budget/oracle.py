"""Runs the pinned-TypeScript oracle (reference.mjs) over a batch of scenarios: one node process per batch.
  run_oracle(cases, env=None, timeout=300, time_mode=False, repeat=5) -> list of observation dicts (id, requests, trace, finalSessions,
  finalClocks, finalToolSeen, activeTimers, driver, excluded[, domain]); with time_mode=True one timing dict per case
  (id, calls, coreNs, fireNs, perOp, depCalls, rssBytes, driver).
CLI:
  oracle.py [scenarios.jsonl [observations.jsonl]]   (stdin / stdout when omitted)
  oracle.py --smoke N [--seed S]                      N small random scenarios, run twice, checked for determinism and driver cleanliness
  oracle.py --perf 100,500,1000                       timing table: per size, N charge calls plus N noteTool calls on one session
Needs GROK_TS_TRANSPILER (defaults through common.TS_ENV) and Node v26.4.0 (BUDGET_NODE or `node` on PATH)."""
import sys
sys.dont_write_bytecode = True
import json
import subprocess

from common import HERE, NODE, TS_ENV, strip_case


def run_oracle(cases, env=None, timeout=300, time_mode=False, repeat=5):
    stdin = ''.join(json.dumps(strip_case(c), ensure_ascii=True) + '\n' for c in cases)
    cmd = [NODE, '--experimental-strip-types', '--no-warnings', '--import', str(HERE / 'reference-hooks.mjs'), str(HERE / 'reference.mjs')]
    if time_mode:
        cmd += ['--time', '--repeat', str(repeat)]
    full_env = dict(TS_ENV())
    if env:
        full_env.update(env)
    proc = subprocess.run(cmd, input=stdin, capture_output=True, text=True, encoding='utf-8', cwd=HERE, env=full_env, timeout=timeout)
    if proc.returncode != 0:
        raise RuntimeError(f'oracle failed ({proc.returncode}): {proc.stderr[-2000:]}')
    results = [json.loads(line) for line in proc.stdout.split('\n') if line.strip()]
    if len(results) != len(cases):
        raise RuntimeError(f'oracle returned {len(results)} results for {len(cases)} cases: {proc.stderr[-2000:]}')
    return results


# ---- smoke: a tiny deterministic scenario generator ------------------------------------------------------------------------
MASK = 0xFFFFFFFF
ISO0 = '2023-11-14T22:13:20.000Z'


class Xorshift:
    """xorshift32 (13, 17, 5)."""

    def __init__(self, seed):
        self.state = (seed & MASK) or 1

    def next(self):
        x = self.state
        x ^= (x << 13) & MASK
        x ^= x >> 17
        x ^= (x << 5) & MASK
        self.state = x & MASK
        return self.state

    def below(self, n):
        return self.next() % n

    def pick(self, items):
        return items[self.below(len(items))]

    def chance(self, num, den):
        return self.below(den) < num


def make_budget(rng, bid, state):
    budget = {
        'id': bid, 'openedAt': ISO0, 'origin': 'user', 'state': state,
        'limits': {'timeMinutes': rng.pick([1, 2, 120]), 'delegations': rng.pick([1, 2, 24]), 'commands': rng.pick([1, 3]), 'reviews': 2, 'checks': 1},
        'used': {'activeMs': 0, 'delegations': 0, 'commands': 0, 'reviews': 0, 'checks': 0, 'tools': 0, 'failures': 0, 'retries': 0},
    }
    if state == 'halted':
        budget['halt'] = {'reason': rng.pick(['time', 'commands', 'delegations']), 'at': ISO0, 'stoppedDelegationIds': []}
    if state == 'closed':
        budget['ended'] = {'how': rng.pick(['quiet', 'user-end']), 'at': ISO0}
    return budget


def make_scenario(rng, k):
    sessions, refs, sids = [], [], []
    for j in range(2 + rng.below(2)):
        sid = f's{j + 1}'
        sids.append(sid)
        session = {'id': sid}
        if rng.chance(2, 3):
            bid = f'b{j + 1}'
            session['budget'] = make_budget(rng, bid, rng.pick(['open', 'open', 'halted', 'closed']))
            refs.append({'sessionId': sid, 'budgetId': bid})
        sessions.append(session)
        for n in (1, 2, 3, 4):  # budgets that attach/replace will open from the default UUID stream
            refs.append({'sessionId': sid, 'budgetId': '00000000-0000-4000-8000-%012x' % n})
    unit = {'timeMinutes': rng.pick([1, 2]), 'delegations': 2, 'commands': 1, 'reviews': 1, 'checks': 1}
    if rng.chance(1, 4):
        unit['defaultUnlimited'] = True

    def ref():
        return dict(rng.pick(refs))

    def sess():
        return {'sess': rng.pick(sids)}

    makers = [
        lambda: ('attach', [sess(), rng.pick(['user', 'schedule', 'direct', 'release'])] + ([rng.pick([1, 2, 4, 8, 'unlimited'])] if rng.chance(1, 2) else [])),
        lambda: ('charge', [ref(), {rng.pick(['delegations', 'commands', 'reviews', 'checks', 'retries']): rng.pick([1, 1, 2, 5])}]),
        lambda: ('noteTool', [ref(), rng.pick(['t1', 't2']), rng.pick(['a1', 'a2', 'a3'])]),
        lambda: ('runStarted', [ref(), rng.pick(['r1', 'r2'])]),
        lambda: ('runWaiting', [ref(), rng.pick(['r1', 'r2']), rng.chance(1, 2)]),
        lambda: ('runEnded', [ref(), rng.pick(['r1', 'r2'])]),
        lambda: ('raise', [ref(), rng.pick([1, 2, 4, 8, 'unlimited'])]),
        lambda: ('decide', [rng.pick(sids), rng.pick(refs)['budgetId'], rng.pick(['extend', 'end'])] + ([rng.pick([1, 2, 'unlimited'])] if rng.chance(1, 2) else [])),
        lambda: ('halt', [ref(), rng.pick(['time', 'delegations', 'commands', 'reviews', 'checks'])]),
        lambda: ('maybeClose', [sess()]),
        lambda: ('setCard', [ref(), {'deliverable': rng.pick(['ship it', '  padded  ', '']), 'done': rng.pick([['a'], [], ['a', ' b ']])}]),
        lambda: ('hasCard', [ref()]),
        lambda: ('current', [ref()]),
        lambda: ('isHalted', [ref()]),
        lambda: ('forgetTurn', [rng.pick(['t1', 't2'])]),
        lambda: ('replaceForUserMessage', [sess()] + ([rng.pick([1, 2])] if rng.chance(1, 2) else [])),
        lambda: ('reopen', [ref()]),
        lambda: ('noteFailure', [ref()]),
        lambda: ('limitText', [make_budget(rng, 'x', 'open'), rng.pick([1, 5]), rng.pick(['ko', 'en'])]),
        lambda: ('refusalWire', [rng.pick(['time', 'commands', 'checks']), make_budget(rng, 'x', 'open')]),
        lambda: ('continueText', [rng.pick(['time', 'delegations', 'reviews']), make_budget(rng, 'x', 'open'), rng.pick(['ko', 'en'])]),
        lambda: ('cardMissing', [rng.pick(['ko', 'en'])]),
        lambda: ('budgetUnitOf', [rng.pick([{'timeMinutes': 3, 'delegations': 4, 'commands': 5, 'reviews': 6, 'checks': 7}, {'undef': 1}])]),
    ]
    script = []
    for index in range(6 + rng.below(11)):
        roll = rng.below(10)
        if roll == 0:
            script.append({'ev': 'advance', 'ms': rng.pick([1000, 60000, 3600000])})
        elif roll == 1 and rng.chance(1, 2):
            dep = rng.pick(['runningDelegationIds', 'isQuiet', 'releaseHeld'])
            replies = [rng.pick([['d1', 'd2'], [], ['d3']])] if dep == 'runningDelegationIds' else [rng.chance(1, 2)]
            script.append({'ev': 'setDepReply', 'dep': dep, 'replies': replies})
        else:
            op, args = rng.pick(makers)()
            script.append({'ev': 'call', 'req': f'r{index}', 'op': op, 'args': args})
    return {
        'id': f'smoke-{k}',
        'world': {
            'sessions': sessions, 'unit': unit,
            'deps': {'runningDelegationIds': [['d1', 'd2'], [], ['d3']], 'isQuiet': [True, False, True, True], 'releaseHeld': [False, True, False]},
        },
        'script': script,
    }


def smoke(count, seed):
    rng = Xorshift(seed)
    cases = [make_scenario(rng, k) for k in range(count)]
    first = run_oracle(cases)
    second = run_oracle(cases)
    problems = []
    if first != second:
        problems.append('two runs differ')
    for case, obs in zip(cases, first):
        if obs['driver'] is not None:
            problems.append(f"{case['id']}: driver violations {obs['driver']}")
        for request in obs['requests']:
            if request['terminal'] not in ('ok', 'thrown'):
                problems.append(f"{case['id']}/{request['id']}: terminal {request['terminal']!r}")
    thrown = sum(1 for obs in first for r in obs['requests'] if r['terminal'] == 'thrown')
    requests = sum(len(obs['requests']) for obs in first)
    records = sum(len(obs['trace']) for obs in first)
    print(f'smoke: {count} scenarios (seed {seed}), {requests} requests ({thrown} thrown), {records} trace records, {len(problems)} problems')
    for problem in problems[:20]:
        print('  ' + problem)
    return 0 if not problems else 1


# ---- perf ------------------------------------------------------------------------------------------------------------------
def perf_scenario(size):
    ref = {'sessionId': 's1', 'budgetId': 'b1'}
    budget = {
        'id': 'b1', 'openedAt': ISO0, 'origin': 'user', 'state': 'open',
        'limits': {'timeMinutes': 120, 'delegations': 10 ** 9, 'commands': 10 ** 9, 'reviews': 10 ** 9, 'checks': 10 ** 9},
        'used': {'activeMs': 0, 'delegations': 0, 'commands': 0, 'reviews': 0, 'checks': 0, 'tools': 0, 'failures': 0, 'retries': 0},
    }
    script = [{'ev': 'call', 'req': f'c{i}', 'op': 'charge', 'args': [ref, {'commands': 1}]} for i in range(size)]
    script += [{'ev': 'call', 'req': f'n{i}', 'op': 'noteTool', 'args': [ref, 'turn1', f'act{i}']} for i in range(size)]
    return {
        'id': f'perf-{size}',
        'world': {'sessions': [{'id': 's1', 'budget': budget}], 'unit': {'timeMinutes': 120, 'delegations': 24, 'commands': 3, 'reviews': 2, 'checks': 1}},
        'script': script,
    }


def perf(sizes, repeat=3):
    results = run_oracle([perf_scenario(n) for n in sizes], time_mode=True, repeat=repeat, timeout=3600)
    print(f"{'size':>8} {'calls':>8} {'coreNs':>14} {'ns/call':>10}  per-op ns/call")
    for size, res in zip(sizes, results):
        calls = res['calls']
        per_op = ' '.join(f"{op}={v['ns'] / v['n']:.0f}" for op, v in res['perOp'].items() if v['n'])
        print(f"{size:>8} {calls:>8} {res['coreNs']:>14} {res['coreNs'] / max(calls, 1):>10.0f}  {per_op}")
        if res['driver']:
            print('  driver:', res['driver'])
    return 0


def main(argv):
    if len(argv) > 1 and argv[1] == '--smoke':
        count = int(argv[2]) if len(argv) > 2 else 50
        seed = int(argv[argv.index('--seed') + 1]) if '--seed' in argv else 20261117
        return smoke(count, seed)
    if len(argv) > 1 and argv[1] == '--perf':
        return perf([int(x) for x in (argv[2] if len(argv) > 2 else '100,500,1000').split(',')])
    src = open(argv[1], encoding='utf-8') if len(argv) > 1 else sys.stdin
    cases = [json.loads(line) for line in src if line.strip()]
    out = open(argv[2], 'w', encoding='utf-8') if len(argv) > 2 else sys.stdout
    for result in run_oracle(cases):
        out.write(json.dumps(result, ensure_ascii=False) + '\n')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
