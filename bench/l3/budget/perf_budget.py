"""L3-10 small perf grid (spec 9, milestone sizes), measured on the pinned TypeScript through the oracle's time mode.
  axes / small sizes:  sessions 100,500,1000   calls 100,500,1000   runs 10,50,100   delegations 10,50,100   clocks 10,50,100
  python3 perf_budget.py [--axes sessions,calls] [--sizes 100,500,1000] [--repeat 5] [--json work/perf/budget_perf.json]
Cases come from q_gen.perf_case(axis, n). Output table: axis, n, calls, coreNs, ns/call, fireNs, plus a "Pruv" column that is always
"not implemented" until a Pruv harness exists. Linearity ratio per axis = (ns at the largest size / ns at the smallest size) divided by
(largest size / smallest size); a ratio above 2.0 is flagged SUSPECT (spec 9: timing is supporting evidence only). The ratio is
computed on coreNs, and on fireNs when both ends are nonzero. --sizes overrides the sizes of every selected axis.
Exit 0 when every run finished without driver violations, 1 otherwise."""
import sys
sys.dont_write_bytecode = True
import argparse
import json
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from common import HERE  # noqa: E402
from oracle import run_oracle  # noqa: E402

AXES = {
    'sessions': [100, 500, 1000],
    'calls': [100, 500, 1000],
    'runs': [10, 50, 100],
    'delegations': [10, 50, 100],
    'clocks': [10, 50, 100],
}
FLAG = 2.0
PRUV = 'not implemented'


def ratio(rows, key):
    """(value at largest n / value at smallest n) / (largest n / smallest n), or None when it cannot be computed."""
    if len(rows) < 2:
        return None
    lo, hi = rows[0], rows[-1]
    if lo['n'] == hi['n'] or not lo[key] or not hi[key]:
        return None
    return (hi[key] / lo[key]) / (hi['n'] / lo['n'])


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--axes', default=','.join(AXES), help='comma list of ' + ','.join(AXES))
    ap.add_argument('--sizes', help='comma list of sizes replacing the default of every selected axis')
    ap.add_argument('--repeat', type=int, default=5)
    ap.add_argument('--json', default=str(HERE / 'work' / 'perf' / 'budget_perf.json'), help='output path of the raw results')
    args = ap.parse_args(argv)

    import q_gen
    axes = [a.strip() for a in args.axes.split(',') if a.strip()]
    for a in axes:
        if a not in AXES:
            print(f'unknown axis {a!r} (use {", ".join(AXES)})')
            return 2
    override = sorted(int(x) for x in args.sizes.split(',')) if args.sizes else None

    results = {}
    problems = []
    for axis in axes:
        sizes = override or AXES[axis]
        cases = [q_gen.perf_case(axis, n) for n in sizes]
        obs = run_oracle(cases, time_mode=True, repeat=args.repeat, timeout=3600)
        rows = []
        for n, res in zip(sizes, obs):
            calls = res.get('calls', 0)
            rows.append({'axis': axis, 'n': n, 'calls': calls, 'coreNs': res.get('coreNs', 0), 'fireNs': res.get('fireNs') or 0,
                         'nsPerCall': (res.get('coreNs', 0) / calls) if calls else None, 'depCalls': res.get('depCalls'),
                         'rssBytes': res.get('rssBytes'), 'perOp': res.get('perOp'), 'pruv': PRUV})
            if res.get('driver'):
                problems.append(f'{axis} n={n}: driver {res["driver"]}')
        results[axis] = rows

    print(f"{'axis':<12} {'n':>6} {'calls':>8} {'coreNs':>14} {'ns/call':>10} {'fireNs':>12}  Pruv")
    flagged = []
    summary = {}
    for axis, rows in results.items():
        for r in rows:
            per = f"{r['nsPerCall']:.0f}" if r['nsPerCall'] is not None else '-'
            print(f"{axis:<12} {r['n']:>6} {r['calls']:>8} {r['coreNs']:>14} {per:>10} {r['fireNs']:>12}  {r['pruv']}")
        core, fire = ratio(rows, 'coreNs'), ratio(rows, 'fireNs')
        summary[axis] = {'coreRatio': core, 'fireRatio': fire}
        parts = []
        for label, val in (('core', core), ('fire', fire)):
            if val is None:
                continue
            bad = val > FLAG
            parts.append(f'{label} {val:.2f}' + (' SUSPECT' if bad else ''))
            if bad:
                flagged.append(f'{axis} {label} {val:.2f}')
        print(f'  linearity {axis}: ' + (', '.join(parts) if parts else 'n/a') + f' (flag > {FLAG})')
    out = Path(args.json)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps({'repeat': args.repeat, 'axes': results, 'linearity': summary, 'flagged': flagged, 'pruv': PRUV}, indent=1), encoding='utf-8')
    print(f'Pruv: {PRUV}; flagged: ' + (', '.join(flagged) if flagged else 'none') + f'; raw results in {out}')
    for p in problems:
        print('PROBLEM ' + p)
    return 1 if problems else 0


if __name__ == '__main__':
    sys.exit(main())
