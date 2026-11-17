"""L3-10 small perf grid (spec 9, milestone sizes): the pinned TypeScript through the oracle's time mode, plus an optional
implementation (native = Pruv-generated binary, hand = hand-written Rust) measured on the same axes and sizes.
  axes / small sizes:  sessions 100,500,1000   calls 100,500,1000   runs 10,50,100   delegations 10,50,100   clocks 10,50,100
  python3 perf_budget.py [--axes sessions,calls] [--sizes 100,500,1000] [--profile S|M|SM] [--repeat 5]
                         [--impl PATH [ARGS...]] [--impl-arg ARG] [--lane native|hand|ts] [--json work/perf/budget_perf.json]
Cases come from q_gen.perf_case(axis, n). Output table: axis, n, calls, coreNs, ns/call, fireNs, plus an implementation column.
--profile S|M|SM replaces the sizes of every selected axis by run_meta.PROFILES (100 / 500 / both); --sizes overrides both.
The implementation reads the one perf case as JSONL on stdin and writes one observation line; its ns/call is the best of --repeat
runs of the observation's own coreNs when it reports one, else the process wall time (marked "wall", includes process start).
Counters that could not be read are the string "unavailable", never 0. Linearity ratio per axis = (ns at the largest size / ns at
the smallest size) divided by (largest size / smallest size); a ratio above 2.0 is flagged SUSPECT (timing is supporting evidence).
Exit status: 0 = every run finished without problems and the implementation column was measured; 1 = problems (driver violation,
implementation failure); 3 = no implementation, so the Pruv/hand column is "unavailable" (never counted as a pass). The JSON
output carries run_meta(...) under "meta"."""
import sys
sys.dont_write_bytecode = True
import argparse
import json
import subprocess
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from common import HERE, strip_case  # noqa: E402
from oracle import run_oracle  # noqa: E402
from run_meta import PROFILES, UNAVAILABLE, num  # noqa: E402
import q as q_mod  # noqa: E402

AXES = {
    'sessions': [100, 500, 1000],
    'calls': [100, 500, 1000],
    'runs': [10, 50, 100],
    'delegations': [10, 50, 100],
    'clocks': [10, 50, 100],
}
FLAG = 2.0
EXIT_OK, EXIT_FAIL, EXIT_NO_IMPL = 0, 1, 3


def _is_num(x):
    return isinstance(x, (int, float)) and not isinstance(x, bool)


def ratio(rows, key):
    """(value at largest n / value at smallest n) / (largest n / smallest n), or None when it cannot be computed
    (fewer than two rows, a zero or unavailable value)."""
    if len(rows) < 2:
        return None
    lo, hi = rows[0], rows[-1]
    if lo['n'] == hi['n'] or not _is_num(lo[key]) or not _is_num(hi[key]) or not lo[key] or not hi[key]:
        return None
    return (hi[key] / lo[key]) / (hi['n'] / lo['n'])


def measure_impl(argv, case, repeat):
    """-> (ns_per_call_total or None, coreNs or None, timing, problem or None) for one perf case. Best of `repeat` runs."""
    stdin = json.dumps(strip_case(case), ensure_ascii=True) + '\n'
    best = None
    best_obs = None
    for _ in range(max(1, repeat)):
        t0 = time.perf_counter_ns()
        try:
            proc = subprocess.run(argv, input=stdin, capture_output=True, text=True, encoding='utf-8', errors='replace', cwd=HERE, timeout=3600)
        except Exception as exc:  # noqa: BLE001
            return None, None, None, f'implementation failed to run: {exc}'
        wall = time.perf_counter_ns() - t0
        if proc.returncode:
            return None, None, None, f'implementation exit {proc.returncode}: {proc.stderr[-200:]}'
        lines = [ln for ln in proc.stdout.split('\n') if ln.strip()]
        if len(lines) != 1:
            return None, None, None, f'implementation printed {len(lines)} lines for 1 case'
        try:
            obs = json.loads(lines[0])
        except ValueError:
            return None, None, None, 'implementation output is not JSON'
        core = obs.get('coreNs') if isinstance(obs, dict) else None
        cost = core if _is_num(core) else wall
        if best is None or cost < best:
            best, best_obs = cost, (obs, _is_num(core))
    obs, has_core = best_obs
    return best, (best if has_core else None), 'core' if has_core else 'wall', None


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--axes', default=','.join(AXES), help='comma list of ' + ','.join(AXES))
    ap.add_argument('--sizes', help='comma list of sizes replacing the default of every selected axis')
    ap.add_argument('--repeat', type=int, default=5)
    ap.add_argument('--json', default=str(HERE / 'work' / 'perf' / 'budget_perf.json'), help='output path of the raw results')
    q_mod.add_common_args(ap)
    args = ap.parse_args(argv)

    import q_gen
    axes = [a.strip() for a in args.axes.split(',') if a.strip()]
    for a in axes:
        if a not in AXES:
            print(f'unknown axis {a!r} (use {", ".join(AXES)})')
            return 2
    if args.sizes:
        override = sorted(int(x) for x in args.sizes.split(','))
    elif args.profile:
        override = list(PROFILES[args.profile])
    else:
        override = None
    impl_cmd = q_mod.impl_from_args(args.impl, args.impl_arg)
    meta = q_mod.build_meta(args, extra={'sizes': override if override else UNAVAILABLE, 'repeat': args.repeat})
    col = 'hand' if args.lane == 'hand' else 'Pruv'

    results = {}
    problems = []
    for axis in axes:
        sizes = override or AXES[axis]
        cases = [q_gen.perf_case(axis, n) for n in sizes]
        obs = run_oracle(cases, time_mode=True, repeat=args.repeat, timeout=3600)
        rows = []
        for n, case, res in zip(sizes, cases, obs):
            calls = res.get('calls')
            core = res.get('coreNs')
            row = {'axis': axis, 'n': n, 'calls': num(calls), 'coreNs': num(core), 'fireNs': num(res.get('fireNs')),
                   'nsPerCall': (core / calls) if _is_num(core) and _is_num(calls) and calls else UNAVAILABLE,
                   'depCalls': num(res.get('depCalls')), 'rssBytes': num(res.get('rssBytes')), 'perOp': num(res.get('perOp'))}
            if impl_cmd:
                cost, _core, timing, problem = measure_impl(impl_cmd, case, args.repeat)
                if problem:
                    problems.append(f'{axis} n={n}: {problem}')
                    row.update({'impl': UNAVAILABLE, 'implNsPerCall': UNAVAILABLE, 'implTiming': UNAVAILABLE})
                else:
                    per = cost / calls if _is_num(calls) and calls else UNAVAILABLE
                    row.update({'impl': f'{per:.0f} ns/call ({timing})' if _is_num(per) else UNAVAILABLE, 'implNsPerCall': per, 'implTiming': timing})
            else:
                row.update({'impl': UNAVAILABLE, 'implNsPerCall': UNAVAILABLE, 'implTiming': UNAVAILABLE})
            rows.append(row)
            if res.get('driver'):
                problems.append(f'{axis} n={n}: driver {res["driver"]}')
        results[axis] = rows

    print(f"{'axis':<12} {'n':>6} {'calls':>8} {'coreNs':>14} {'ns/call':>10} {'fireNs':>12}  {col} ({args.lane})")
    flagged = []
    summary = {}
    for axis, rows in results.items():
        for r in rows:
            per = f"{r['nsPerCall']:.0f}" if _is_num(r['nsPerCall']) else UNAVAILABLE
            print(f"{axis:<12} {r['n']:>6} {r['calls']:>8} {r['coreNs']:>14} {per:>10} {r['fireNs']:>12}  {r['impl']}")
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

    if problems:
        rc, result = EXIT_FAIL, 'fail'
    elif not impl_cmd:
        rc, result = EXIT_NO_IMPL, 'unavailable'
    else:
        rc, result = EXIT_OK, 'measured'
    out = Path(args.json)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps({'repeat': args.repeat, 'lane': args.lane, 'result': result, 'exit': rc, 'axes': results, 'linearity': summary,
                               'flagged': flagged, 'problems': problems, 'pruv': UNAVAILABLE if not impl_cmd else 'measured', 'meta': meta},
                              indent=1), encoding='utf-8')
    print(f'{col}: {UNAVAILABLE if not impl_cmd else "measured"}; flagged: ' + (', '.join(flagged) if flagged else 'none') + f'; raw results in {out}')
    for p in problems:
        print('PROBLEM ' + p)
    print('RESULT ' + result + ('' if impl_cmd else ' (no implementation; the Pruv/hand column is unavailable and not counted)'))
    return rc


if __name__ == '__main__':
    sys.exit(main())
