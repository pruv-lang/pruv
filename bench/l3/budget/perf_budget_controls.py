"""Controls for perf_budget.py (and q.build_meta): fake implementations written to a temp dir prove that the perf runner
validates every output line against the oracle, uses the spec per-axis sizes, alternates the lanes and records honest metadata.
  python3 perf_budget_controls.py
Exit 0 only if every control behaves as expected; one PASS/FAIL line per control and a summary line at the end.
Controls
  1  correct implementation (prints the oracle observation)          -> measured, exit 0
  2  prints {}                                                        -> rejected (missing field)
  3  correct observation minus one field                              -> rejected, "missing field <name>"
  4  correct observation with one result field changed                -> rejected, "result mismatch vs oracle"
  5  prints two lines                                                 -> rejected
  6  --profile S: runs / delegations / clocks n=10, sessions / calls n=100
  7  --profile M: runs / delegations / clocks n=50, sessions / calls n=500
  8  the "order" list alternates and has 2 * repeat entries
  9  min <= median and a spread are present for both lanes
  10 q.build_meta records "unknown" without --backend / --build-flags, and the given values with them
Needs node and the pinned TypeScript (see oracle.py); the fakes get the oracle observations precomputed by this file."""
import sys
sys.dont_write_bytecode = True
import argparse
import contextlib
import io
import json
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import perf_budget  # noqa: E402
import q as q_mod  # noqa: E402
import q_gen  # noqa: E402
from oracle import run_oracle  # noqa: E402

FAKE = '''import json
import sys

mode, mapfile = sys.argv[1], sys.argv[2]
case = json.loads(sys.stdin.readline())
obs = json.load(open(mapfile, encoding="utf-8"))[case["id"]]
if mode == "good":
    print(json.dumps(obs))
elif mode == "empty":
    print("{}")
elif mode == "delfield":
    del obs["finalClocks"]
    print(json.dumps(obs))
elif mode == "mutate":
    obs["requests"][0]["id"] = "tampered"
    print(json.dumps(obs))
elif mode == "earlyterm":
    # P6 shape: the first request ends on a resource bound, so nothing before it is compared; final state also differs
    obs["requests"][0]["terminal"] = "core"
    obs["requests"][0]["value"] = "ResourceBound(input)"
    obs["finalClocks"] = {"tampered": 1}
    print(json.dumps(obs))
elif mode == "finalstate":
    obs["finalClocks"] = {"tampered": 1}
    print(json.dumps(obs))
elif mode == "late":
    # correct on the first run, wrong on every later run (a counter file next to the map file)
    import os
    cnt = mapfile + ".count"
    n = int(open(cnt).read()) if os.path.exists(cnt) else 0
    open(cnt, "w").write(str(n + 1))
    if n:
        obs["finalClocks"] = {"tampered": 1}
    print(json.dumps(obs))
elif mode == "twolines":
    print(json.dumps(obs))
    print(json.dumps(obs))
else:
    sys.exit(9)
'''

AXIS = 'runs'
SIZE = 10


class Env:
    """Temp dir with the fake script and the precomputed oracle observations (case id -> observation)."""

    def __init__(self, root):
        self.root = Path(root)
        self.fake = self.root / 'fake_impl.py'
        self.fake.write_text(FAKE, encoding='utf-8')
        self.mapfile = self.root / 'oracle_obs.json'
        case = q_gen.perf_case(AXIS, SIZE)
        self.mapfile.write_text(json.dumps({case['id']: run_oracle([case])[0]}), encoding='utf-8')
        self.count = 0

    def run(self, extra, mode=None):
        """Runs perf_budget.main in-process. -> (exit code, JSON dict, printed text)."""
        self.count += 1
        out = self.root / f'run{self.count}.json'
        argv = list(extra) + ['--json', str(out)]
        if mode:
            argv += ['--impl', sys.executable, str(self.fake), mode, str(self.mapfile)]
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = perf_budget.main(argv)
        return rc, json.loads(out.read_text(encoding='utf-8')), buf.getvalue()


BASE = ['--axes', AXIS, '--sizes', str(SIZE)]


def expect_rejected(env, mode, needle):
    rc, data, _text = env.run(BASE + ['--repeat', '2'], mode)
    if rc != 1 or data['result'] != 'fail':
        return f'expected exit 1 / fail, got exit {rc} / {data["result"]}'
    row = data['axes'][AXIS][0]
    if row['implNsPerCall'] != 'unavailable':
        return f'rejected cell still carries a measured value: {row["implNsPerCall"]!r}'
    if not any(needle in p for p in data['problems']):
        return f'no problem containing {needle!r}: {data["problems"]}'
    return None


def sizes_by_axis(data):
    return {axis: [r['n'] for r in rows] for axis, rows in data['axes'].items()}


def check_profile(env, profile, small, big):
    rc, data, _text = env.run(['--profile', profile, '--repeat', '1'])  # no implementation: exit 3, sizes still recorded
    if rc != 3:
        return f'expected exit 3 (no implementation), got {rc}'
    got = sizes_by_axis(data)
    want = {'sessions': [big], 'calls': [big], 'runs': [small], 'delegations': [small], 'clocks': [small]}
    return None if got == want else f'sizes {got} != {want}'


def main():
    results = []

    def record(name, err):
        results.append((name, err))
        print(('PASS ' if err is None else 'FAIL ') + name + ('' if err is None else ': ' + err), flush=True)

    def guarded(name, fn):
        try:
            record(name, fn())
        except Exception as exc:  # noqa: BLE001
            record(name, f'{type(exc).__name__}: {exc}')

    with tempfile.TemporaryDirectory() as tmp:
        env = Env(tmp)
        good = {}

        def c1():
            rc, data, _t = env.run(BASE + ['--repeat', '3'], 'good')
            good.update(data=data)
            if rc != 0 or data['result'] != 'measured' or data['problems']:
                return f'exit {rc}, result {data["result"]}, problems {data["problems"]}'
            return None

        def c8():
            order = good['data']['axes'][AXIS][0]['order']
            want = ['ts', 'impl', 'impl', 'ts', 'ts', 'impl']
            return None if order == want else f'order {order} != {want}'

        def c9():
            row = good['data']['axes'][AXIS][0]
            if row['implTiming'] != 'wall':
                return f'fake has no coreNs, expected wall timing, got {row["implTiming"]}'
            for lane, lo, med, spr in (('ts', 'coreNs', 'coreNsMedian', 'coreNsSpread'), ('impl', 'implNs', 'implNsMedian', 'implNsSpread')):
                if not all(isinstance(row[k], (int, float)) for k in (lo, med, spr)):
                    return f'{lane}: min / median / spread not numeric: {row[lo]!r} {row[med]!r} {row[spr]!r}'
                if not row[lo] <= row[med]:
                    return f'{lane}: min {row[lo]} > median {row[med]}'
                if row[spr] < 0:
                    return f'{lane}: negative spread {row[spr]}'
            if len(row['coreNsRuns']) != 3 or len(row['implNsRuns']) != 3:
                return f'expected 3 per-rep values per lane, got {len(row["coreNsRuns"])} / {len(row["implNsRuns"])}'
            return None

        def c10():
            def parse(argv, binary):
                ap = argparse.ArgumentParser()
                q_mod.add_common_args(ap)
                return ap.parse_args(['--impl', str(binary)] + argv)

            binary = Path(tmp) / 'fake_binary'
            binary.write_text('#!/bin/sh\n', encoding='utf-8')
            meta = q_mod.build_meta(parse([], binary))
            if meta['backend'] != 'unknown' or meta['flags'] != 'unknown':
                return f'no options: backend {meta["backend"]!r}, flags {meta["flags"]!r}'
            meta = q_mod.build_meta(parse(['--backend', 'rust', '--build-flags=-C opt-level=3'], binary))
            if meta['backend'] != 'rust' or meta['flags'] != ['-C', 'opt-level=3']:
                return f'explicit options: backend {meta["backend"]!r}, flags {meta["flags"]!r}'
            side = Path(str(binary) + '.build.json')
            side.write_text(json.dumps({'backend': 'cranelift', 'flags': ['-O']}), encoding='utf-8')
            meta = q_mod.build_meta(parse([], binary))
            if meta['backend'] != 'cranelift' or meta['flags'] != ['-O']:
                return f'build.json: backend {meta["backend"]!r}, flags {meta["flags"]!r}'
            other = Path(tmp) / 'other_binary'
            other.write_text('#!/bin/sh\n', encoding='utf-8')
            meta = q_mod.build_meta(parse([], binary), impl_argv_used=[str(other)])
            if meta['backend'] != 'unknown' or meta['flags'] != 'unknown':
                return f'override argv: metadata read from --impl, not from the binary run: {meta["backend"]!r}'
            side.unlink()
            meta = q_mod.build_meta(parse(['--lane', 'ts'], binary))
            if meta['backend'] != 'ts-oracle' or meta['flags'] != 'n/a':
                return f'lane ts: backend {meta["backend"]!r}, flags {meta["flags"]!r}'
            return None

        guarded('1 correct implementation is measured (exit 0)', c1)
        guarded('2 {} is rejected', lambda: expect_rejected(env, 'empty', 'missing field'))
        guarded('3 missing field is rejected', lambda: expect_rejected(env, 'delfield', 'missing field finalClocks'))
        guarded('4 changed result field is rejected', lambda: expect_rejected(env, 'mutate', 'result mismatch vs oracle'))
        guarded('5 two output lines are rejected', lambda: expect_rejected(env, 'twolines', '2 lines'))
        guarded('6 --profile S sizes (runs/delegations/clocks 10, sessions/calls 100)', lambda: check_profile(env, 'S', 10, 100))
        guarded('7 --profile M sizes (runs/delegations/clocks 50, sessions/calls 500)', lambda: check_profile(env, 'M', 50, 500))
        guarded('8 order alternates, 2 * repeat entries', c8 if 'data' in good else lambda: 'control 1 produced no data')
        guarded('9 min <= median and spread for both lanes', c9 if 'data' in good else lambda: 'control 1 produced no data')
        guarded('10 q.build_meta records unknown / the given backend and flags (of the argv actually run)', c10)

        def c14():
            rc, data, _t = env.run(['--axes', AXIS, '--sizes', str(SIZE), '--repeat', '3'])
            row = data['axes'][AXIS][0]
            if rc != 3 or len(row['coreNsRuns']) != 3 or row['order'] != ['ts', 'ts', 'ts']:
                return f'no implementation, repeat 3: exit {rc}, {len(row["coreNsRuns"])} ts runs, order {row["order"]}'
            return None

        guarded('11 P6 early terminal with a different final state is rejected',
                lambda: expect_rejected(env, 'earlyterm', 'result mismatch vs oracle'))
        guarded('12 changed final state alone is rejected', lambda: expect_rejected(env, 'finalstate', 'result mismatch vs oracle'))
        guarded('13 a wrong result on a later rep only is rejected', lambda: expect_rejected(env, 'late', 'result mismatch vs oracle'))
        guarded('14 no implementation: repeat N gives N oracle runs', c14)

    failed = [n for n, e in results if e is not None]
    print(f'SUMMARY {len(results) - len(failed)}/{len(results)} controls passed' + ('' if not failed else '; FAILED: ' + '; '.join(failed)))
    return 0 if not failed else 1


if __name__ == '__main__':
    sys.exit(main())
