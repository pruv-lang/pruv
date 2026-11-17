#!/usr/bin/env python3
"""Pruv vs hand-written Rust vs TS for the generated-code perf_headroom items (see COMPILER.md).

  python3 run_compiler_perf.py [--rounds 5] [--keys py,rs] [--no-build] [--json out.json]

1. copies m6_test/root.pruv + perf_extra.pruv and bud_num/bud_text/bud_date into work/perf_compiler/src
2. compiles it with the Python compiler and/or the Rust compiler (PRUVC_RS, default as run_m6.py), checks the two
   sol.rs are identical, and builds pruv_harness.rs and hand.rs with rustc -O --edition 2021
3. each round runs, per item and n, the Pruv harness, the hand Rust binary and the Node loop one after another
   (alternating, so load changes hit all three); each run is 1 warm-up + 3 reps, min. Reports the median over rounds.
The bar (Pruv/hand) is the fastest same-work hand writing; `_reuse` writings are printed for reference only.
Checksums must agree between Pruv, the hand `_u` writings and TS (the String writing of p_ref counts UTF-8 bytes).
"""
import argparse
import json
import os
import shutil
import statistics
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
BUDGET = HERE.parents[1]
sys.path.insert(0, str(BUDGET / 'm6_test'))
import run_m6 as M6                           # noqa: E402

WORK = BUDGET / 'work' / 'perf_compiler'
SIZES = (100, 500, 1000)
# item: (pruv op, pruv args, hand writings, ts source, ts args)
ITEMS = [
    ('p_int', [1234567890123], ['p_int', 'p_int_u', 'p_int_v'], 'node_ref', [1234567890123]),
    ('p_iso', [1700000000000], ['p_iso', 'p_iso_u'], 'node_ref', [1700000000000]),
    ('p_ref', [2, 0, 12, 99999, 0], ['p_ref', 'p_ref_u'], 'node_ref', [2, 0, 12, 99999, 0]),
    ('p_amt', [12345, 99999], ['p_amt', 'p_amt_u', 'p_amt_reuse'], 'extra', [12345, 99999]),
    ('p_wr', [1000], ['p_wr', 'p_wr_c'], 'extra', [1000]),
]


def prepare():
    src = WORK / 'src'
    if src.exists():
        shutil.rmtree(src)
    src.mkdir(parents=True)
    (src / 'root.pruv').write_text((BUDGET / 'm6_test' / 'root.pruv').read_text() + (HERE / 'perf_extra.pruv').read_text())
    for m in M6.MODULES:
        shutil.copy(BUDGET / m, src / m)


def build(keys):
    prepare()
    root = WORK / 'src' / 'root.pruv'
    for key in keys:
        out = WORK / key
        out.mkdir(parents=True, exist_ok=True)
        sol = out / 'sol.rs'
        if key == 'rs':
            cmd = [os.environ.get('PRUVC_RS') or M6.RS_DEFAULT, root, '-o', sol, '--lib', M6.LIB]
        else:
            cmd = [os.environ.get('PRUVC_PY_PYTHON', '/Users/spica4345/Projects/pruv/.venv/bin/python'),
                   M6.REPO / 'pruvc' / 'pruvc.py', root, '-o', sol, '--backend', 'rust', '--lib', M6.LIB]
        p = M6.sh(cmd)
        if p.returncode != 0:
            sys.exit(f'[{key}] compiler error\n{p.stdout}{p.stderr}')
        main_rs = out / 'main.rs'
        main_rs.write_text((HERE / 'pruv_harness.rs').read_text().replace('@SOL@', str(sol)))
        p = M6.sh(['rustc', '-O', '--edition', '2021', main_rs, '-o', out / 'harness'])
        if p.returncode != 0:
            sys.exit(f'[{key}] rustc error\n{p.stderr}')
    if len(keys) == 2:
        same = (WORK / 'py' / 'sol.rs').read_bytes() == (WORK / 'rs' / 'sol.rs').read_bytes()
        print(f'sol.rs py == rs: {same}')
        if not same:
            sys.exit(2)
    p = M6.sh(['rustc', '-O', '--edition', '2021', HERE / 'hand.rs', '-o', WORK / 'hand'])
    if p.returncode != 0:
        sys.exit(f'hand.rs rustc error\n{p.stderr}')


def run_bin(exe, lines):
    p = M6.sh([exe], ''.join(x + '\n' for x in lines))
    out = [x for x in p.stdout.split('\n') if x]
    if p.returncode != 0 or len(out) != len(lines):
        sys.exit(f'{exe} failed rc {p.returncode}\n{p.stderr[-2000:]}')
    return [(int(x.split()[1]), int(x.split()[2])) for x in out]


def timed(exe, op, n, args):
    """1 warm-up + 3 reps; returns (min ns/call, checksum)."""
    vals = run_bin(exe, [' '.join([op, str(n)] + [str(a) for a in args])] * 4)
    return min(v[0] for v in vals[1:]) / n, vals[1][1]


def ts_round():
    reqs_ref, reqs_extra, order_ref, order_extra = [], [], [], []
    for op, _, _, src, targs in ITEMS:
        for n in SIZES:
            r = {'op': 'perf', 'name': op, 'n': n, 'args': [str(a) for a in targs], 'reps': 3}
            (reqs_ref if src == 'node_ref' else reqs_extra).append(r)
            (order_ref if src == 'node_ref' else order_extra).append((op, n))
    res = {}
    for (op, n), r in zip(order_ref, M6.run_node(reqs_ref)):
        res[(op, n)] = (r['ns'] / n, r['sum'])
    cmd = [M6.NODE, HERE / 'ts_extra.mjs']
    p = M6.sh(cmd, ''.join(json.dumps(r) + '\n' for r in reqs_extra))
    for (op, n), line in zip(order_extra, [x for x in p.stdout.split('\n') if x.strip()]):
        r = json.loads(line)
        if 'err' in r:
            sys.exit(f'ts error {op}: {r["err"]}')
        res[(op, n)] = (r['ns'] / n, r['sum'])
    return res


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--rounds', type=int, default=5)
    ap.add_argument('--keys', default='py,rs')
    ap.add_argument('--no-build', action='store_true')
    ap.add_argument('--json')
    a = ap.parse_args()
    keys = [k for k in a.keys.split(',') if k]
    if not a.no_build:
        build(keys)
    samples = {}                                  # (item, n, who) -> [(ns, sum)]
    for rnd in range(a.rounds):
        ts = ts_round()
        for op, pargs, hands, _, _ in ITEMS:
            for n in SIZES:
                for key in keys:
                    samples.setdefault((op, n, key), []).append(timed(WORK / key / 'harness', op, n, pargs))
                for h in hands:
                    samples.setdefault((op, n, h), []).append(timed(WORK / 'hand', h, n, pargs))
                samples.setdefault((op, n, 'ts'), []).append(ts[(op, n)])
        print(f'round {rnd + 1}/{a.rounds} done', flush=True)
    med = {k: (statistics.median(x[0] for x in v), v[0][1]) for k, v in samples.items()}
    report = []
    print(f'\nmedian over {a.rounds} rounds of min-of-3 ns/call (checksum in brackets)')
    for op, _, hands, _, _ in ITEMS:
        for n in SIZES:
            whos = keys + hands + ['ts']
            row = {w: med[(op, n, w)] for w in whos}
            best_hand = min(row[h][0] for h in hands if not h.endswith('_reuse'))   # same-work writings only
            ratio = {k: row[k][0] / best_hand for k in keys}
            print(f'{op:6}{n:>6}  ' + '  '.join(f'{w}={row[w][0]:.1f}[{row[w][1]}]' for w in whos)
                  + '  ' + '  '.join(f'{k}/hand={ratio[k]:.2f}' for k in keys))
            report.append({'op': op, 'n': n, **{w: row[w][0] for w in whos}, 'sums': {w: row[w][1] for w in whos},
                           'best_hand': best_hand, 'ratio': ratio})
    if a.json:
        Path(a.json).write_text(json.dumps(report, indent=1))


if __name__ == '__main__':
    main()
