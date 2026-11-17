"""L3-10 small perf grid (spec 9, milestone sizes): the pinned TypeScript through the oracle's time mode, plus an optional
implementation (native = Pruv-generated binary, hand = hand-written Rust) measured on the same axes and sizes.
  axes / spec sizes:  S: sessions 100, calls 100, runs 10, delegations 10, clocks 10
                      M: sessions 500, calls 500, runs 50, delegations 50, clocks 50
  python3 perf_budget.py [--axes sessions,calls] [--sizes 10,50] [--profile S|M|SM] [--repeat 5]
                         [--impl PATH [ARGS...]] [--impl-arg ARG] [--lane native|hand|ts] [--json work/perf/budget_perf.json]
Cases come from q_gen.perf_case(axis, n). --profile picks run_meta.AXIS_PROFILES (per-axis sizes); the default is SM (both sizes of
every axis). --sizes overrides the sizes of every selected axis explicitly.
Measurement: for rep r in range(repeat) the TS oracle runs once (time mode, repeat 1) and the implementation runs once; which one goes
first alternates (r even: ts first, r odd: impl first). The run order of each cell is recorded in the JSON as "order". Each lane
reports min, median and spread ((max - min) / median, a fraction) over its reps, in the JSON rows and in the table (spread in %).
Implementation timing is the observation's own coreNs when every rep reports one, else the process wall time (marked "wall",
includes process start). Counters that could not be read are the string "unavailable", never 0.
Validation: the implementation reads the one perf case as JSONL on stdin and must write exactly one observation line. EVERY output
line of EVERY rep is compared with the oracle's normal (non-time) observation of the same case through compare.compare (the
comparison q.py uses for Q cases); timing fields are ignored. A line that is not an object, misses a field the oracle observation
has ("missing field <name>") or differs ("result mismatch vs oracle: <first diff>") makes the cell a problem, never a measurement.
Linearity ratio per axis = (median ns at the largest size / median ns at the smallest size) divided by (largest size / smallest
size); a ratio above 2.0 is flagged SUSPECT (timing is supporting evidence).
Exit status: 0 = every run finished without problems and the implementation column was measured; 1 = problems (driver violation,
implementation failure or rejected output); 3 = no implementation, so the Pruv/hand column is "unavailable" (never counted as a
pass). The JSON output carries run_meta(...) under "meta"."""
import sys
sys.dont_write_bytecode = True
import argparse
import json
import statistics
import subprocess
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import compare as compare_mod  # noqa: E402
from common import HERE, strip_case  # noqa: E402
from oracle import run_oracle  # noqa: E402
from run_meta import AXIS_PROFILES, UNAVAILABLE, num  # noqa: E402
import q as q_mod  # noqa: E402

AXES = ('sessions', 'calls', 'runs', 'delegations', 'clocks')
FLAG = 2.0
EXIT_OK, EXIT_FAIL, EXIT_NO_IMPL = 0, 1, 3
DEFAULT_PROFILE = 'SM'
# Keys an implementation may add to its observation for timing; they never take part in the comparison.
TIMING_KEYS = ('coreNs', 'fireNs', 'perOp', 'depCalls', 'rssBytes', 'wallNs')
# Echo of the case's `domain` tag: compare.compare would classify any observation carrying it as excluded-domain without
# comparing anything, so it is removed from both sides first.
IGNORED_KEYS = ('domain',) + TIMING_KEYS


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


def stats(values):
    """-> {'min', 'median', 'spread'} of a list of numbers; spread = (max - min) / median (a fraction). UNAVAILABLE when empty
    or not numeric; spread is UNAVAILABLE when the median is 0."""
    vals = [v for v in values if _is_num(v)]
    if not vals or len(vals) != len(values):
        return {'min': UNAVAILABLE, 'median': UNAVAILABLE, 'spread': UNAVAILABLE}
    med = statistics.median(vals)
    return {'min': min(vals), 'median': med, 'spread': (max(vals) - min(vals)) / med if med else UNAVAILABLE}


def _observed(obs):
    return {k: v for k, v in obs.items() if k not in IGNORED_KEYS}


def validate_obs(expected, actual):
    """Compares one implementation output line with the oracle's normal observation. -> problem string or None.
    Uses compare.compare (exact or P6 accepted, as q.py does); timing fields and the `domain` echo are excluded."""
    if not isinstance(actual, dict):
        return 'result mismatch vs oracle: output is not a JSON object'
    want = _observed(expected)
    for name in sorted(want):
        if name not in actual:
            return f'missing field {name}'
    got = _observed(actual)
    cls, detail = compare_mod.compare(want, got)
    if cls in ('exact', 'P6'):
        return None
    return f'result mismatch vs oracle: {detail or cls}'


def run_impl_once(argv, stdin, expected):
    """One process run of the implementation. -> (core ns or None, wall ns, problem or None)."""
    t0 = time.perf_counter_ns()
    try:
        proc = subprocess.run(argv, input=stdin, capture_output=True, text=True, encoding='utf-8', errors='replace', cwd=HERE, timeout=3600)
    except Exception as exc:  # noqa: BLE001
        return None, 0, f'implementation failed to run: {exc}'
    wall = time.perf_counter_ns() - t0
    if proc.returncode:
        return None, wall, f'implementation exit {proc.returncode}: {proc.stderr[-200:]}'
    lines = [ln for ln in proc.stdout.split('\n') if ln.strip()]
    if len(lines) != 1:
        return None, wall, f'implementation printed {len(lines)} lines for 1 case'
    try:
        obs = json.loads(lines[0])
    except ValueError:
        return None, wall, 'implementation output is not JSON'
    problem = validate_obs(expected, obs)
    if problem:
        return None, wall, problem
    core = obs.get('coreNs')
    return (core if _is_num(core) else None), wall, None


def measure_cell(impl_cmd, case, expected, repeat):
    """Alternating measurement of one cell. -> dict with order, ts (per-rep time-mode results), impl_core, impl_wall, problem."""
    stdin = json.dumps(strip_case(case), ensure_ascii=True) + '\n'
    order, ts_runs, impl_core, impl_wall = [], [], [], []
    problem = None
    for r in range(max(1, repeat)):
        lanes = ['ts', 'impl'] if (r % 2 == 0 or not impl_cmd) else ['impl', 'ts']
        for lane in lanes:
            order.append(lane)
            if lane == 'ts':
                ts_runs.append(run_oracle([case], time_mode=True, repeat=1, timeout=3600)[0])
            else:
                core, wall, problem = run_impl_once(impl_cmd, stdin, expected)
                if problem:
                    return {'order': order, 'ts': ts_runs, 'impl_core': impl_core, 'impl_wall': impl_wall, 'problem': problem}
                impl_core.append(core)
                impl_wall.append(wall)
    return {'order': order, 'ts': ts_runs, 'impl_core': impl_core, 'impl_wall': impl_wall, 'problem': None}


def _per_call(stat, calls):
    return {k: (v / calls if _is_num(v) and _is_num(calls) and calls else UNAVAILABLE) for k, v in stat.items() if k != 'spread'} | {'spread': stat['spread']}


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--axes', default=','.join(AXES), help='comma list of ' + ','.join(AXES))
    ap.add_argument('--sizes', help='comma list of sizes replacing the sizes of every selected axis')
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
    profile = args.profile or DEFAULT_PROFILE
    if args.sizes:
        sizes_of = {a: sorted(int(x) for x in args.sizes.split(',')) for a in axes}
    else:
        sizes_of = {a: list(AXIS_PROFILES[profile][a]) for a in axes}
    impl_cmd = q_mod.impl_from_args(args.impl, args.impl_arg)
    meta = q_mod.build_meta(args, extra={'sizes': sizes_of, 'sizeSource': 'sizes' if args.sizes else 'AXIS_PROFILES.' + profile,
                                         'repeat': args.repeat})
    col = 'hand' if args.lane == 'hand' else 'Pruv'

    results = {}
    problems = []
    for axis in axes:
        rows = []
        for n in sizes_of[axis]:
            case = q_gen.perf_case(axis, n)
            expected = run_oracle([case])[0]  # the oracle's normal observation: what every implementation line must equal
            row = {'axis': axis, 'n': n}
            for p in q_mod.oracle_problems(expected):
                problems.append(f'{axis} n={n}: oracle {p}')
            cell = measure_cell(impl_cmd, case, expected, args.repeat)
            ts = cell['ts']
            first = ts[0]
            calls = first.get('calls')
            core_s = stats([t.get('coreNs') for t in ts])
            fire_s = stats([t.get('fireNs') for t in ts])
            ts_per = _per_call(core_s, calls)
            row.update({
                'calls': num(calls), 'coreNs': core_s['min'], 'coreNsMedian': core_s['median'], 'coreNsSpread': core_s['spread'],
                'coreNsRuns': [num(t.get('coreNs')) for t in ts],
                'fireNs': fire_s['median'], 'fireNsMin': fire_s['min'], 'fireNsSpread': fire_s['spread'],
                'fireNsRuns': [num(t.get('fireNs')) for t in ts],
                'nsPerCall': ts_per['min'], 'nsPerCallMedian': ts_per['median'], 'nsPerCallSpread': ts_per['spread'],
                'depCalls': num(first.get('depCalls')), 'rssBytes': num(first.get('rssBytes')), 'perOp': num(first.get('perOp')),
                'order': cell['order'],
            })
            for t in ts:
                if t.get('driver'):
                    problems.append(f'{axis} n={n}: driver {t["driver"]}')
                    break
            if not impl_cmd:
                row.update({'impl': UNAVAILABLE, 'implNsPerCall': UNAVAILABLE, 'implNsPerCallMedian': UNAVAILABLE,
                            'implNsPerCallSpread': UNAVAILABLE, 'implNsRuns': UNAVAILABLE, 'implTiming': UNAVAILABLE})
            elif cell['problem']:
                problems.append(f'{axis} n={n}: {cell["problem"]}')
                row.update({'impl': UNAVAILABLE, 'implNsPerCall': UNAVAILABLE, 'implNsPerCallMedian': UNAVAILABLE,
                            'implNsPerCallSpread': UNAVAILABLE, 'implNsRuns': UNAVAILABLE, 'implTiming': UNAVAILABLE})
            else:
                use_core = all(_is_num(c) for c in cell['impl_core'])
                runs = cell['impl_core'] if use_core else cell['impl_wall']
                st = stats(runs)
                per = _per_call(st, calls)
                timing = 'core' if use_core else 'wall'
                row.update({
                    'impl': f"{per['min']:.0f} / {per['median']:.0f} ns/call, spread {per['spread'] * 100:.1f}% ({timing})"
                            if _is_num(per['min']) and _is_num(per['median']) and _is_num(per['spread']) else UNAVAILABLE,
                    'implNs': st['min'], 'implNsMedian': st['median'], 'implNsSpread': st['spread'], 'implNsRuns': list(runs),
                    'implNsPerCall': per['min'], 'implNsPerCallMedian': per['median'], 'implNsPerCallSpread': per['spread'],
                    'implTiming': timing,
                })
            rows.append(row)
        results[axis] = rows

    def pct(x):
        return f'{x * 100:.1f}' if _is_num(x) else UNAVAILABLE

    def fmt(x):
        return f'{x:.0f}' if _is_num(x) else UNAVAILABLE

    print(f"{'axis':<12} {'n':>6} {'calls':>8} {'ts min':>10} {'ts med':>10} {'spr%':>6} {'fireNs':>12}  {col} ({args.lane}): ns/call min / median, spread %")
    flagged = []
    summary = {}
    for axis, rows in results.items():
        for r in rows:
            print(f"{axis:<12} {r['n']:>6} {r['calls']:>8} {fmt(r['nsPerCall']):>10} {fmt(r['nsPerCallMedian']):>10} {pct(r['nsPerCallSpread']):>6} "
                  f"{r['fireNs']:>12}  {r['impl']}")
        core, fire = ratio(rows, 'coreNsMedian'), ratio(rows, 'fireNs')
        summary[axis] = {'coreRatio': core, 'fireRatio': fire}
        parts = []
        for label, val in (('core', core), ('fire', fire)):
            if val is None:
                continue
            bad = val > FLAG
            parts.append(f'{label} {val:.2f}' + (' SUSPECT' if bad else ''))
            if bad:
                flagged.append(f'{axis} {label} {val:.2f}')
        print(f'  linearity {axis} (medians): ' + (', '.join(parts) if parts else 'n/a') + f' (flag > {FLAG})')

    if problems:
        rc, result = EXIT_FAIL, 'fail'
    elif not impl_cmd:
        rc, result = EXIT_NO_IMPL, 'unavailable'
    else:
        rc, result = EXIT_OK, 'measured'
    out = Path(args.json)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps({'repeat': args.repeat, 'lane': args.lane, 'profile': profile, 'result': result, 'exit': rc, 'axes': results,
                               'linearity': summary, 'flagged': flagged, 'problems': problems,
                               'pruv': UNAVAILABLE if not impl_cmd else ('rejected' if problems else 'measured'), 'meta': meta},
                              indent=1), encoding='utf-8')
    print(f'{col}: {UNAVAILABLE if not impl_cmd else ("rejected" if problems else "measured")}; flagged: '
          + (', '.join(flagged) if flagged else 'none') + f'; raw results in {out}')
    for p in problems:
        print('PROBLEM ' + p)
    print('RESULT ' + result + ('' if impl_cmd else ' (no implementation; the Pruv/hand column is unavailable and not counted)'))
    return rc


if __name__ == '__main__':
    sys.exit(main())
