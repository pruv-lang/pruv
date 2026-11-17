"""L3-9 Q runner: runs work/q/cases.jsonl on the Node oracle and (later) compares an implementation pair against it.
  python3 q.py [--cases work/q/cases.jsonl] [--batch 250] [--impl CMD] [--keep-going]
Oracle-only checks: no driver violations, every request terminal (pending only for cases that never settle on purpose),
determinism (two oracle runs identical). Writes work/q/oracle.jsonl and work/q/report.json. Exit 1 on any unexplained finding.
--impl CMD: CMD is an implementation harness binary path (or a command line) that reads the cases JSONL on stdin and writes one observation JSON per line (same shape as
the oracle). Absent -> oracle-only. Failures are written to work/q/fail/<n>.json (see Q_README.md)."""
import sys
sys.dont_write_bytecode = True
import json
import shlex
import subprocess
import time
from collections import Counter, defaultdict
from pathlib import Path

import compare as compare_mod
from common import HERE
from oracle import run_oracle

WORK = HERE / 'work' / 'q'


def load_cases(path):
    with open(path, encoding='utf-8') as f:
        return [json.loads(line) for line in f if line.strip()]


def run_batches(cases, size):
    out = []
    for i in range(0, len(cases), size):
        out.extend(run_oracle(cases[i:i + size]))
    return out


def run_impl(cmd, cases):
    stdin = ''.join(json.dumps(c, ensure_ascii=True) + '\n' for c in cases)
    proc = subprocess.run(shlex.split(cmd), input=stdin, capture_output=True, text=True, encoding='utf-8', cwd=HERE, timeout=1800)
    if proc.returncode != 0:
        raise RuntimeError(f'impl failed ({proc.returncode}): {proc.stderr[-2000:]}')
    res = [json.loads(line) for line in proc.stdout.split('\n') if line.strip()]
    if len(res) != len(cases):
        raise RuntimeError(f'impl returned {len(res)} results for {len(cases)} cases')
    return res


def first_trace_diff(a, b):
    """Index and both records of the first differing trace entry (None when equal)."""
    ta, tb = a.get('trace', []), b.get('trace', [])
    for i in range(max(len(ta), len(tb))):
        x = ta[i] if i < len(ta) else None
        y = tb[i] if i < len(tb) else None
        if x != y:
            return {'index': i, 'oracle': x, 'impl': y}
    return None


def compare(oracle_obs, impl_obs):
    """The implementation pair against the oracle (compare.compare_observation, full observation, no policy: Q cases are common
    fake cases): None when exact, else {class, path, expected, actual, firstTraceDiff, requestsEqual}."""
    if isinstance(impl_obs, dict) and (impl_obs.get('driver') is not None or 'InternalInvariant' in json.dumps(impl_obs)):
        return {'class': 'unexplained', 'path': 'driver', 'expected': None, 'actual': impl_obs.get('driver'),
                'firstTraceDiff': None, 'requestsEqual': False}
    r = compare_mod.compare_observation(oracle_obs, impl_obs)
    if r['class'] == 'exact':
        return None
    return dict(r, firstTraceDiff=first_trace_diff(oracle_obs, impl_obs), requestsEqual=oracle_obs.get('requests') == impl_obs.get('requests'))


def intentionally_pending(case):
    """No case is meant to leave a request pending; hook kept so a future family can opt in with tag 'pending-ok'."""
    return 'pending-ok' in case.get('tags', [])


def check_oracle(case, obs):
    problems = []
    if obs.get('driver') is not None:
        problems.append(f"driver violation: {obs['driver']}")
    for r in obs.get('requests', []):
        if r['terminal'] == 'pending' and not intentionally_pending(case):
            problems.append(f"request {r['id']} still pending")
    started = {e['req'] for e in case['script'] if e.get('ev') == 'start'}
    seen = {r['id'] for r in obs.get('requests', [])}
    if started - seen:
        problems.append(f'requests missing from observation: {sorted(started - seen)}')
    return problems


def main(argv):
    arg = lambda name, default=None: argv[argv.index(name) + 1] if name in argv else default
    cases_path = Path(arg('--cases', WORK / 'cases.jsonl'))
    batch = int(arg('--batch', 250))
    impl = arg('--impl')
    keep_going = '--keep-going' in argv
    t0 = time.time()
    cases = load_cases(cases_path)
    first = run_batches(cases, batch)
    t1 = time.time()
    second = run_batches(cases, batch)
    t2 = time.time()
    diff_idx = [i for i, (a, b) in enumerate(zip(first, second)) if a != b]
    WORK.mkdir(parents=True, exist_ok=True)
    with open(WORK / 'oracle.jsonl', 'w', encoding='utf-8') as f:
        for obs in first:
            f.write(json.dumps(obs, ensure_ascii=True, separators=(',', ':')) + '\n')

    fam_count = Counter(c['family'] for c in cases)
    hist = defaultdict(Counter)
    problems = []
    for c, o in zip(cases, first):
        for r in o.get('requests', []):
            hist[c['family']][r['terminal']] += 1
        for p in check_oracle(c, o):
            problems.append({'id': c['id'], 'family': c['family'], 'problem': p})
    unexplained = len(problems) + len(diff_idx)

    print('cases per family: ' + ' '.join(f'{k}={fam_count[k]}' for k in sorted(fam_count)))
    for fam in sorted(hist):
        print(f'  {fam} terminals: ' + ' '.join(f'{t}={n}' for t, n in sorted(hist[fam].items())))
    print(f"tieBreak cases: {sum(1 for c in cases if c.get('tieBreak'))}")
    for p in problems[:20]:
        print(f"PROBLEM {p['id']}: {p['problem'][:300]}")
    if len(problems) > 20:
        print(f'... {len(problems) - 20} more problems')
    print(f"determinism: {'identical' if not diff_idx else 'DIFFERENT at ' + ', '.join(cases[i]['id'] for i in diff_idx[:10])}")

    fails = []
    if impl:
        impl_obs = run_impl(impl, cases)
        for c, o, im in zip(cases, first, impl_obs):
            d = compare(o, im)
            if d is not None:
                fails.append({'seed': 20261119, 'case': c['id'], 'family': c['family'], 'input': c['world'], 'script': c['script'], 'oracle': o, 'impl': im,
                              'firstTraceDifference': d.get('firstTraceDiff'), 'diff': d})
        fail_dir = WORK / 'fail'
        fail_dir.mkdir(parents=True, exist_ok=True)
        for old in fail_dir.glob('*.json'):
            old.unlink()
        for n, fl in enumerate(fails, 1):
            (fail_dir / f'{n}.json').write_text(json.dumps(fl, ensure_ascii=False, indent=1), encoding='utf-8')
        print(f'implementation compare: {len(cases) - len(fails)}/{len(cases)} equal' + (f' ({len(fails)} artifacts in {fail_dir})' if fails else ''))
        unexplained += len(fails)
    else:
        print('implementation compare: skipped (no --impl; oracle-only run)')

    report = {'seed': 20261119, 'cases': len(cases), 'perFamily': dict(fam_count), 'terminals': {k: dict(v) for k, v in hist.items()}, 'problems': problems,
              'deterministic': not diff_idx, 'impl': bool(impl), 'implFailures': len(fails), 'oracleSeconds': round(t1 - t0, 1), 'rerunSeconds': round(t2 - t1, 1)}
    (WORK / 'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=1), encoding='utf-8')
    print(f"oracle run: {len(cases)} cases, {len(problems)} problems, {len(diff_idx)} nondeterministic; first run {t1 - t0:.1f}s, second run {t2 - t1:.1f}s, total {time.time() - t0:.1f}s")
    return 0 if unexplained == 0 or keep_going and False else 1


if __name__ == '__main__':
    sys.exit(main(sys.argv))
