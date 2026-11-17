"""Req list three-way split, legs A (Node oracle) and B (Pruv harness as committed), plus leg C (a stripped Pruv harness copy).

Scenario: bench/l3/req perf_req.py's `list`, placement `none` (no session carries a requested thread, so every item walks every
session), K = 8 candidates per session, M items. Each cell runs once per binary in a fresh process; the best of --reps runs is
kept. Wall time is the whole process (start, scenario parse, run, observation print).

  python3 run_ab.py [--sizes 2000,20000,50000] [--m 10] [--reps 3] [--b work/b/harness] [--c work/c/harness] [--out work/ab.json]
"""
import sys
sys.dont_write_bytecode = True
import argparse
import json
import subprocess
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REQ = HERE.parent / 'req'
sys.path.insert(0, str(REQ))
import perf_req  # noqa: E402  (read-only reuse of the scenario generator)


def timed(cmd, infile, outfile):
    t0 = time.perf_counter()
    with open(infile, 'rb') as fi, open(outfile, 'wb') as fo:
        rc = subprocess.run(cmd, stdin=fi, stdout=fo, stderr=subprocess.DEVNULL, cwd=REQ).returncode
    return time.perf_counter() - t0, rc


def summary(path):
    obs = json.loads(open(path, encoding='utf-8').readline())
    c = perf_req.counts(obs)
    req = obs['requests'][0] if obs['requests'] else {}
    return {'terminal': req.get('terminal'), 'isTemporary': c['isTemporary'], 'next': c['next'], 'trace': c['trace'],
            'value': json.dumps(req.get('value'), sort_keys=True)}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--sizes', default='2000,20000,50000')
    ap.add_argument('--m', type=int, default=10)
    ap.add_argument('--reps', type=int, default=3)
    ap.add_argument('--b', default=str(HERE / 'work' / 'b' / 'harness'), help='leg B harness (build_bc.sh)')
    ap.add_argument('--c', default=str(HERE / 'work' / 'c' / 'harness'), help='leg C harness (build_bc.sh)')
    ap.add_argument('--legs', default='A,B,C')
    ap.add_argument('--out', default=str(HERE / 'work' / 'ab.json'))
    args = ap.parse_args()
    legs = {'A': perf_req.ORACLE_CMD, 'B': [str(Path(args.b).resolve())]}
    legs['C'] = [str(Path(args.c).resolve())]
    legs = {k: v for k, v in legs.items() if k in args.legs.split(',')}
    work = HERE / 'work'
    work.mkdir(exist_ok=True)
    rows = []
    # empty scenario: process start + module load only
    empty = work / 'empty.jsonl'
    empty.write_text(json.dumps({'id': 'empty', 'world': {'sessions': []}, 'script': []}) + '\n')
    base = {k: min(timed(v, empty, work / f'empty_{k}.out')[0] for _ in range(args.reps)) for k, v in legs.items()}
    for S in [int(x) for x in args.sizes.split(',')]:
        sc = perf_req.scenario('list', S, args.m, 'none')
        infile = work / f'list_{S}_{args.m}.jsonl'
        infile.write_text(json.dumps(sc, ensure_ascii=True) + '\n')
        row = {'S': S, 'M': args.m}
        for k, cmd in legs.items():
            best, rc = None, None
            for _ in range(args.reps):
                t, rc = timed(cmd, infile, work / f'out_{k}_{S}.jsonl')
                best = t if best is None else min(best, t)
            row[k] = best
            row[k + '_rc'] = rc
            row[k + '_sum'] = summary(work / f'out_{k}_{S}.jsonl')
        vals = {row[k + '_sum']['value'] for k in legs}
        row['same_value'] = len(vals) == 1
        rows.append(row)
        print(json.dumps({k: (round(v, 3) if isinstance(v, float) else v) for k, v in row.items() if not k.endswith('_sum')}), flush=True)
    Path(args.out).write_text(json.dumps({'empty': base, 'rows': rows}, indent=1))
    print('empty-scenario process time:', {k: round(v, 3) for k, v in base.items()})


if __name__ == '__main__':
    main()
