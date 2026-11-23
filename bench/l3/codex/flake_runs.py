"""Repeat one case on already built runners (no rebuild, no verify): G-DB57-4.

Runs the pinned TypeScript once for the reference result and request log, then RUNS times each pair's
work/<pair>/runner against the fake app-server, checking every run as test_impl.py does (expected JSON, TS result,
TS request log). Build the runners first with test_impl.py.

    python3 bench/l3/codex/flake_runs.py [--case read_server_requests] [--runs 30] [--pairs rs_rust,py_llvm]
"""
import sys
sys.dont_write_bytecode = True
import argparse
import json
from common import WORK, fake_binary, request_for, read_log, run_lines, node_command, check_case, load_cases


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--case', default='read_server_requests')
    ap.add_argument('--runs', type=int, default=30)
    ap.add_argument('--pairs', default='rs_rust,py_llvm')
    a = ap.parse_args()
    case = next(c for c in load_cases() if c['name'] == a.case)
    binary = fake_binary()
    ts_result = run_lines(node_command(), [request_for(case, 'flake_ts', 0, binary)])[0]
    ts_log = read_log(0, 'flake_ts')
    problems = check_case(case, ts_result, ts_log)
    if problems:
        raise SystemExit(f'TS reference run failed: {problems[0]}')
    summary = {}
    for pair in a.pairs.split(','):
        exe = WORK / pair / 'runner'
        if not exe.is_file():
            raise SystemExit(f'{exe} missing: run test_impl.py first')
        misses = []
        for run in range(a.runs):
            side = f'flake_{pair}'
            result = run_lines([exe], [request_for(case, side, 0, binary)])[0]
            p = check_case(case, result, read_log(0, side), ts_result, ts_log)
            if p:
                misses.append((run, p[0][:200]))
                print(f'{pair} run {run}: MISS {p[0][:200]}', flush=True)
        summary[pair] = {'runs': a.runs, 'misses': len(misses), 'details': misses}
        print(f'{pair}: {a.runs - len(misses)}/{a.runs} passed', flush=True)
    print(json.dumps(summary))
    if any(v['misses'] for v in summary.values()):
        raise SystemExit(1)


if __name__ == '__main__':
    main()
